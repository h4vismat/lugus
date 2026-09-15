use chrono::{DateTime, Utc};
use lugus_portfolio::*;
use serde::{Deserialize, Serialize};
/// Revisions are strings at every external boundary, preserving all u64 bits.
pub mod revision {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(v: &u64, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&v.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
        String::deserialize(d)?
            .parse()
            .map_err(serde::de::Error::custom)
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PortfolioCommand {
    pub request_id: String,
    pub portfolio_id: Option<String>,
    #[serde(with = "revision")]
    pub expected_revision: u64,
    pub mutation: PortfolioMutation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum PortfolioMutation {
    SetBenchmarkProvider {
        instance_id: Option<String>,
    },
    CreatePortfolio {
        name: String,
    },
    RenamePortfolio {
        name: String,
    },
    CreateInstrument {
        name: String,
        symbol: String,
        asset_kind: AssetKind,
    },
    CreateAccount {
        name: String,
        start: Day,
        opening: Opening,
        events: Vec<Event>,
    },
    RenameAccount {
        account_id: String,
        name: String,
    },
    ChangeSetup {
        account_id: String,
        start: Day,
        opening: Opening,
        edits: Vec<EventEdit>,
    },
    EditEvents {
        account_id: String,
        edits: Vec<EventEdit>,
    },
    ApplySplit {
        instrument_id: String,
        date: Day,
        order: u64,
        numerator: u64,
        denominator: u64,
    },
    ReplaceSplit {
        action_id: String,
        date: Day,
        order: u64,
        numerator: u64,
        denominator: u64,
    },
    VoidSplit {
        action_id: String,
    },
    BindInstrument {
        instrument_id: String,
        instance_id: String,
        native_id: lugus_financial::market_data::InstrumentId,
    },
    UnbindInstrument {
        instrument_id: String,
    },
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AssetKind {
    Stock,
    Etf,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum EventEdit {
    Append { event: Event },
    Replace { event: Event },
    Void { id: String },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioHeader {
    pub id: String,
    pub name: String,
    pub currency: String,
    #[serde(with = "revision")]
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    #[serde(with = "revision")]
    pub revision: u64,
    pub ledger: Ledger,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Instrument {
    pub id: String,
    pub name: String,
    pub symbol: String,
    pub asset_kind: AssetKind,
    pub currency: String,
    pub binding: Option<PortfolioBinding>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioBinding {
    pub instance_id: String,
    pub native_id: lugus_financial::market_data::InstrumentId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioDocument {
    #[serde(default)]
    pub benchmark_instance_id: Option<String>,
    pub header: PortfolioHeader,
    pub instruments: Vec<Instrument>,
    pub accounts: Vec<Account>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioReceipt {
    pub request_id: String,
    pub portfolio_id: String,
    #[serde(with = "revision")]
    pub revision: u64,
    pub account_ids: Vec<String>,
    pub instrument_ids: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountSummary {
    pub id: String,
    pub name: String,
    pub start: Day,
    pub simplified: bool,
    #[serde(with = "revision")]
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioView {
    pub id: String,
    pub name: String,
    #[serde(with = "revision")]
    pub revision: u64,
    pub as_of: Day,
    pub accounts: Vec<AccountSummary>,
    pub instruments: Vec<Instrument>,
    pub valuation: Valuation,
    pub realized: Decimal,
    pub dividends: Decimal,
    pub standalone_fees: Decimal,
    pub trade_fees: Decimal,
    pub deposits: Decimal,
    pub withdrawals: Decimal,
    pub price_status: Vec<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioPage<T> {
    pub items: Vec<T>,
    pub next_offset: Option<usize>,
    #[serde(with = "revision")]
    pub revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuditEntry {
    pub request_id: String,
    #[serde(with = "revision")]
    pub revision: u64,
    pub recorded_at: DateTime<Utc>,
    pub mutation: PortfolioMutation,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SnapshotRequest {
    pub request_id: String,
    pub portfolio_id: String,
    pub account_id: Option<String>,
    #[serde(with = "revision")]
    pub expected_revision: u64,
    pub conversation_id: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioSnapshot {
    pub prices: Vec<PortfolioPriceReceipt>,
    pub id: String,
    pub conversation_id: String,
    pub created_at: DateTime<Utc>,
    pub calculation_version: u32,
    pub summary: PortfolioView,
    pub row_counts: std::collections::BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RefreshRequest {
    pub request_id: String,
    pub portfolio_id: String,
    #[serde(with = "revision")]
    pub expected_revision: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortfolioPriceReceipt {
    pub instrument_id: String,
    pub binding: PortfolioBinding,
    pub fetch_id: Option<String>,
    pub provider: Option<lugus_financial::domain::ProviderIdentity>,
    pub observed_at: DateTime<Utc>,
    pub price: Option<PriceInput>,
    pub raw_bar: Option<lugus_financial::market_data::PriceBar>,
    pub status: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RefreshResult {
    pub id: String,
    pub request: RefreshRequest,
    pub status: String,
    pub receipts: Vec<PortfolioPriceReceipt>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortfolioPreview {
    pub view: PortfolioView,
    pub states: Vec<AccountState>,
}
