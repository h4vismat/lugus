//! Conservative interpretation of the pinned bundled provider's documented Close field.
use super::PortfolioBinding;
use lugus_financial::{domain::ProviderIdentity, market_data::PriceBar};
use lugus_portfolio::{Day, Decimal, PriceBasis, PriceInput};
/// Yahoo Close is split-adjusted, not dividend-adjusted. Only a recent close from
/// the pinned plugin fetched for this date is eligible. A recorded split after
/// the bar date invalidates it; historical valuation is deliberately unsupported.
/// Source semantics: https://ca.finance.yahoo.com/quote/JNJ/history/
/// Upstream confirmation: https://github.com/ranaroussi/yfinance/discussions/1682
pub fn yfinance_price_input(
    instrument_id: &str,
    provider: &ProviderIdentity,
    binding: &PortfolioBinding,
    bar: &PriceBar,
    observation_id: i64,
    last_split: Option<Day>,
    as_of: Day,
) -> Option<PriceInput> {
    if provider.plugin_id != "yfinance"
        || provider.plugin_version != "0.2.0"
        || provider.instance_id != binding.instance_id
        || bar.instrument != binding.native_id
        || bar.currency != "USD"
        || bar.date > as_of
        || bar.date < as_of - chrono::Duration::days(14)
        || bar.retrieved_at.date_naive() != as_of
        || last_split.is_some_and(|date| date > bar.date)
        || observation_id <= 0
    {
        return None;
    }
    let close = Decimal::parse(bar.close.as_str()).ok()?;
    if !close.is_positive() {
        return None;
    }
    Some(PriceInput {
        instrument_id: instrument_id.into(),
        close,
        currency: "USD".into(),
        date: bar.date,
        basis: PriceBasis::Compatible {
            share_basis_date: as_of,
        },
        observation_id: observation_id.to_string(),
    })
}
