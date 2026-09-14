use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PriceBasis {
    Compatible { share_basis_date: Day },
    Unknown,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceInput {
    pub instrument_id: String,
    pub close: Decimal,
    pub currency: String,
    pub date: Day,
    pub basis: PriceBasis,
    pub observation_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HoldingValue {
    pub instrument_id: String,
    pub quantity: Decimal,
    pub basis: Decimal,
    pub market_value: Option<Decimal>,
    pub unrealized: Option<Decimal>,
    pub price: Option<PriceInput>,
    pub unpriced_reason: Option<String>,
    pub allocation_percent: Option<Decimal>,
    pub simplified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Valuation {
    pub cash: Decimal,
    pub holdings: Vec<HoldingValue>,
    pub priced_subtotal: Decimal,
    pub total_value: Option<Decimal>,
    pub unrealized: Option<Decimal>,
    pub complete: bool,
    pub cash_allocation_percent: Option<Decimal>,
}
pub fn value_accounts(
    states: &[AccountState],
    prices: &[PriceInput],
    as_of: Day,
) -> Result<Valuation> {
    let mut cash = Decimal::zero();
    let mut holdings: BTreeMap<String, HoldingValue> = BTreeMap::new();
    for s in states {
        require(s.as_of == as_of, "holdings accounting date mismatch")?;
        cash = cash.checked_add(&s.cash)?;
        for lot in &s.lots {
            let h = holdings
                .entry(lot.instrument_id.clone())
                .or_insert_with(|| HoldingValue {
                    instrument_id: lot.instrument_id.clone(),
                    quantity: Decimal::zero(),
                    basis: Decimal::zero(),
                    market_value: None,
                    unrealized: None,
                    price: None,
                    unpriced_reason: Some("No compatible USD price available".into()),
                    allocation_percent: None,
                    simplified: false,
                });
            h.quantity = h.quantity.checked_add(&lot.quantity)?;
            h.basis = h.basis.checked_add(&lot.basis)?;
            h.simplified |= lot.simplified;
        }
    }
    let mut subtotal = cash.clone();
    let mut unrealized = Decimal::zero();
    let mut complete = true;
    for h in holdings.values_mut() {
        let mut candidates=prices.iter().filter(|p|p.instrument_id==h.instrument_id&&p.currency=="USD"&&p.close.is_positive()&&p.date<=as_of&&matches!(p.basis,PriceBasis::Compatible{share_basis_date} if share_basis_date==as_of)).collect::<Vec<_>>();
        candidates.sort_by_key(|p| p.date);
        if let Some(p) = candidates.last() {
            if candidates
                .iter()
                .filter(|c| c.date == p.date)
                .any(|c| c.close != p.close)
            {
                h.unpriced_reason = Some("Conflicting price observations".into());
                complete = false;
                continue;
            }
            let value = h.quantity.checked_mul(&p.close)?;
            let gain = value.checked_sub(&h.basis)?;
            subtotal = subtotal.checked_add(&value)?;
            unrealized = unrealized.checked_add(&gain)?;
            h.market_value = Some(value);
            h.unrealized = Some(gain);
            h.price = Some((*p).clone());
            h.unpriced_reason = None;
        } else {
            complete = false;
        }
    }
    let mut cash_percent = None;
    if complete && subtotal.is_positive() {
        let hundred = Decimal::parse("100")?;
        cash_percent = Some(hundred.allocated(&cash, &subtotal)?);
        for h in holdings.values_mut() {
            h.allocation_percent = Some(hundred.allocated(
                h.market_value.as_ref().expect("complete holding"),
                &subtotal,
            )?);
        }
    }
    Ok(Valuation {
        cash,
        holdings: holdings.into_values().collect(),
        priced_subtotal: subtotal.clone(),
        total_value: complete.then_some(subtotal),
        unrealized: complete.then_some(unrealized),
        complete,
        cash_allocation_percent: cash_percent,
    })
}
