use crate::{Result, portfolio::*};
use lugus_financial::{
    domain::fingerprint,
    storage::{RunStatus, history::HistoryReadPage},
};
use lugus_portfolio::*;
use std::collections::BTreeSet;
pub(super) struct OwnedHistoryInput {
    pub ledgers: Vec<Ledger>,
    pub closes: Vec<HistoricalClose>,
    pub benchmark: Vec<HistoricalClose>,
    pub baseline: Day,
    pub end: Day,
}
pub(super) fn history_inputs(
    doc: &PortfolioDocument,
    account: Option<&str>,
    baseline: Day,
    end: Day,
    bundles: &[(HistoryEvidenceRef, HistoryReadPage)],
) -> Result<OwnedHistoryInput> {
    if account.is_some_and(|id| !doc.accounts.iter().any(|a| a.id == id)) {
        return Err(invalid("history account mismatch"));
    }
    let mut result = OwnedHistoryInput {
        ledgers: doc
            .accounts
            .iter()
            .filter(|a| account.is_none_or(|id| id == a.id))
            .map(|a| a.ledger.clone())
            .collect(),
        closes: vec![],
        benchmark: vec![],
        baseline,
        end,
    };
    let mut instruments = BTreeSet::new();
    let mut total = 0usize;
    for (reference, page) in bundles {
        total = total
            .checked_add(page.items.len())
            .ok_or_else(|| invalid("historical observation overflow"))?;
        if total > 100_000 || !instruments.insert(reference.instrument_id.clone()) {
            return Err(invalid("duplicate or excessive history evidence"));
        }
        let run = &page.run;
        let manifest = run
            .manifest
            .as_ref()
            .ok_or_else(|| invalid("missing history manifest"))?;
        if run.status != RunStatus::Complete
            || run.row_count != page.items.len()
            || run.id.to_string() != reference.run_id
            || run.provider != reference.provider
            || fingerprint(manifest)? != reference.manifest_fingerprint
            || page.next_offset.is_some()
        {
            return Err(invalid("history evidence identity mismatch"));
        }
        let id = if let Some(id) = &reference.instrument_id {
            let binding = doc
                .instruments
                .iter()
                .find(|i| &i.id == id)
                .and_then(|i| i.binding.as_ref())
                .ok_or_else(|| invalid("history binding missing"))?;
            if binding.instance_id != reference.provider.instance_id
                || binding.native_id != manifest.instrument
            {
                return Err(invalid("historical source binding changed"));
            }
            id.as_str()
        } else {
            if manifest.instrument.namespace != "yahoo:symbol"
                || manifest.instrument.value != "^SP500TR"
                || manifest.source_basis != "total_return_index"
            {
                return Err(invalid("benchmark is not the S&P 500 total-return index"));
            }
            "^SP500TR"
        };
        if page.items.first().map(|r| r.day.date) != Some(manifest.coverage_start)
            || page.items.last().map(|r| r.day.date) != Some(manifest.anchor)
        {
            return Err(invalid("incomplete history action coverage"));
        }
        let actions = page
            .items
            .iter()
            .filter_map(|r| {
                r.day
                    .split
                    .as_ref()
                    .map(|a| (r.day.date, (a.numerator, a.denominator)))
            })
            .collect::<Vec<_>>();
        for (index, item) in page.items.iter().enumerate() {
            let row = &item.day;
            if index > 0 && page.items[index - 1].day.date.succ_opt() != Some(row.date) {
                return Err(invalid("nonconsecutive historical evidence"));
            }
            let chain = actions
                .iter()
                .filter(|(date, _)| *date > row.date)
                .map(|(_, ratio)| *ratio)
                .collect::<Vec<_>>();
            let factor = Decimal::parse("1")
                .map_err(engine)?
                .adjusted_by_splits(&chain)
                .map_err(engine)?;
            if factor != Decimal::parse(row.factor_to_anchor.as_str()).map_err(engine)? {
                return Err(invalid(
                    "source normalization factor does not match action chain",
                ));
            }
            let close = row
                .close
                .as_ref()
                .map(|v| Decimal::parse(v.as_str()).map_err(engine))
                .transpose()?;
            let expected = row
                .source_close
                .as_ref()
                .map(|v| {
                    Decimal::parse(v.as_str())
                        .and_then(|v| v.adjusted_by_splits(&chain))
                        .map_err(engine)
                })
                .transpose()?;
            if close != expected {
                return Err(invalid(
                    "normalized price does not match source and split evidence",
                ));
            }
            let observed = HistoricalClose {
                instrument_id: id.into(),
                date: row.date,
                session_close: row.market_close,
                close,
                split: row.split.as_ref().map(|v| (v.numerator, v.denominator)),
                observation_id: format!("{}:{}", reference.fetch_id, item.id),
                unsupported_action: row.unsupported_action.clone(),
            };
            if reference.instrument_id.is_some() {
                result.closes.push(observed);
            } else {
                result.benchmark.push(observed);
            }
        }
    }
    Ok(result)
}
