use super::*;
use lugus_portfolio::Decimal;
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AllocationMetric {
    pub key: String,
    pub label: String,
    pub value: Decimal,
    pub percent: Decimal,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldingMetric {
    pub instrument_id: String,
    pub unrealized_percent: Option<Decimal>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DashboardMetrics {
    pub invested_market_value: Option<Decimal>,
    pub by_asset_type: Vec<AllocationMetric>,
    pub largest_instrument_id: Option<String>,
    pub largest_weight: Option<Decimal>,
    pub top_two_weight: Option<Decimal>,
    pub holdings: Vec<HoldingMetric>,
}
pub fn dashboard_metrics(view: &PortfolioView) -> Result<DashboardMetrics> {
    let hundred = Decimal::parse("100").map_err(engine)?;
    let holdings = view
        .valuation
        .holdings
        .iter()
        .map(|h| {
            Ok(HoldingMetric {
                instrument_id: h.instrument_id.clone(),
                unrealized_percent: if h.basis.is_positive() {
                    h.unrealized
                        .as_ref()
                        .map(|gain| hundred.allocated(gain, &h.basis).map_err(engine))
                        .transpose()?
                } else {
                    None
                },
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let mut result = DashboardMetrics {
        invested_market_value: None,
        by_asset_type: vec![],
        largest_instrument_id: None,
        largest_weight: None,
        top_two_weight: None,
        holdings,
    };
    let Some(total) = view
        .valuation
        .total_value
        .as_ref()
        .filter(|_| view.valuation.complete)
    else {
        return Ok(result);
    };
    result.invested_market_value = Some(total.checked_sub(&view.valuation.cash).map_err(engine)?);
    if !total.is_positive() {
        return Ok(result);
    }
    let mut stocks = Decimal::zero();
    let mut etfs = Decimal::zero();
    let mut ranked = Vec::new();
    for h in &view.valuation.holdings {
        let Some(value) = &h.market_value else {
            return Err(invalid("complete valuation contains unpriced holding"));
        };
        let instrument = view
            .instruments
            .iter()
            .find(|i| i.id == h.instrument_id)
            .ok_or_else(|| invalid("holding instrument missing"))?;
        let target = if instrument.asset_kind == AssetKind::Stock {
            &mut stocks
        } else {
            &mut etfs
        };
        *target = target.checked_add(value).map_err(engine)?;
        ranked.push((&h.instrument_id, value));
    }
    for (key, label, value) in [
        ("stocks", "Stocks", stocks),
        ("etfs", "ETFs", etfs),
        ("cash", "Cash", view.valuation.cash.clone()),
    ] {
        if value.is_positive() {
            result.by_asset_type.push(AllocationMetric {
                key: key.into(),
                label: label.into(),
                percent: hundred.allocated(&value, total).map_err(engine)?,
                value,
            });
        }
    }
    ranked.sort_by(|a, b| b.1.cmp(a.1).then_with(|| a.0.cmp(b.0)));
    if let Some((id, value)) = ranked.first() {
        result.largest_instrument_id = Some((*id).clone());
        result.largest_weight = Some(hundred.allocated(value, total).map_err(engine)?);
        let sum = ranked
            .iter()
            .take(2)
            .try_fold(Decimal::zero(), |sum, (_, v)| sum.checked_add(v))
            .map_err(engine)?;
        result.top_two_weight = Some(hundred.allocated(&sum, total).map_err(engine)?);
    }
    Ok(result)
}
