use crate::{Day, Decimal};
use serde::{Deserialize, Serialize};
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventKind {
    Buy {
        instrument_id: String,
        quantity: Decimal,
        price: Decimal,
        gross: Decimal,
        fees: Decimal,
        gross_overridden: bool,
    },
    Sell {
        instrument_id: String,
        quantity: Decimal,
        price: Decimal,
        gross: Decimal,
        fees: Decimal,
        gross_overridden: bool,
    },
    Deposit {
        amount: Decimal,
    },
    Withdrawal {
        amount: Decimal,
    },
    Dividend {
        instrument_id: String,
        amount: Decimal,
    },
    Fee {
        amount: Decimal,
    },
    Split {
        instrument_id: String,
        numerator: u64,
        denominator: u64,
        action_id: String,
    },
}
impl EventKind {
    pub fn instrument_id(&self) -> Option<&str> {
        match self {
            Self::Buy { instrument_id, .. }
            | Self::Sell { instrument_id, .. }
            | Self::Dividend { instrument_id, .. }
            | Self::Split { instrument_id, .. } => Some(instrument_id),
            _ => None,
        }
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Event {
    pub id: String,
    pub date: Day,
    pub order: u64,
    pub kind: EventKind,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OpeningLot {
    pub id: String,
    pub instrument_id: String,
    pub acquired: Day,
    pub tie_order: u64,
    pub quantity: Decimal,
    pub basis: Decimal,
    pub simplified: bool,
    pub date_assumed: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum Opening {
    FullHistory,
    Existing {
        cash: Decimal,
        lots: Vec<OpeningLot>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Ledger {
    pub account_id: String,
    pub start: Day,
    pub opening: Opening,
    pub events: Vec<Event>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Lot {
    pub id: String,
    pub instrument_id: String,
    pub acquired: Day,
    pub tie_order: u64,
    pub quantity: Decimal,
    pub basis: Decimal,
    pub simplified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SaleMatch {
    pub sale_id: String,
    pub lot_id: String,
    pub quantity: Decimal,
    pub basis: Decimal,
    pub net_proceeds: Decimal,
    pub realized: Decimal,
    pub simplified: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountState {
    pub account_id: String,
    pub start: Day,
    pub as_of: Day,
    pub cash: Decimal,
    pub realized: Decimal,
    pub dividends: Decimal,
    pub standalone_fees: Decimal,
    pub trade_fees: Decimal,
    pub deposits: Decimal,
    pub withdrawals: Decimal,
    pub lots: Vec<Lot>,
    pub matches: Vec<SaleMatch>,
}
