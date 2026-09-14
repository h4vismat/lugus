//! Daily market evidence, independent of provider and company fundamentals.
use crate::{
    domain::{Decimal, Validate, fingerprint},
    error::{Error, ErrorKind, Result},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use std::cmp::Ordering;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstrumentId {
    pub namespace: String,
    pub value: String,
}
impl Validate for InstrumentId {
    fn validate(&self) -> Result<()> {
        require(
            !self.namespace.trim().is_empty() && !self.value.trim().is_empty(),
            "instrument namespace and value required",
        )
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PriceQuery {
    pub instrument: InstrumentId,
    pub start: NaiveDate,
    pub end: NaiveDate,
    #[serde(default)]
    pub cursor: Option<String>,
    pub page_size: usize,
}
impl Validate for PriceQuery {
    fn validate(&self) -> Result<()> {
        self.instrument.validate()?;
        require(self.start <= self.end, "reversed price date range")?;
        require(
            (1..=1000).contains(&self.page_size),
            "page_size must be between 1 and 1000",
        )?;
        require(
            self.cursor.as_ref().is_none_or(|v| !v.is_empty()),
            "empty cursor",
        )
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PriceBasis {
    SourceReported,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PricePrecision {
    BinaryFloatSource,
    DecimalSource,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Completeness {
    Unverified,
    Partial,
    Complete,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceCoverage {
    pub first_date: Option<NaiveDate>,
    pub last_date: Option<NaiveDate>,
    pub completeness: Completeness,
}
impl Validate for PriceCoverage {
    fn validate(&self) -> Result<()> {
        require(
            match (self.first_date, self.last_date) {
                (None, None) => true,
                (Some(first), Some(last)) => first <= last,
                _ => false,
            },
            "invalid observed coverage dates",
        )
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PriceBar {
    pub instrument: InstrumentId,
    pub date: NaiveDate,
    pub open: Decimal,
    pub high: Decimal,
    pub low: Decimal,
    pub close: Decimal,
    pub volume: u64,
    pub adjusted_close: Option<Decimal>,
    pub currency: String,
    pub exchange_timezone: String,
    pub price_basis: PriceBasis,
    pub precision: PricePrecision,
    pub source_url: String,
    pub retrieved_at: DateTime<Utc>,
}
impl Validate for PriceBar {
    fn validate(&self) -> Result<()> {
        self.instrument.validate()?;
        for price in [&self.open, &self.high, &self.low, &self.close]
            .into_iter()
            .chain(self.adjusted_close.iter())
        {
            require(!negative(price), "negative price")?;
        }
        require(
            compare(&self.low, &self.high) != Ordering::Greater
                && compare(&self.open, &self.low) != Ordering::Less
                && compare(&self.open, &self.high) != Ordering::Greater
                && compare(&self.close, &self.low) != Ordering::Less
                && compare(&self.close, &self.high) != Ordering::Greater,
            "inconsistent OHLC range",
        )?;
        require(
            [&self.currency, &self.exchange_timezone, &self.source_url]
                .iter()
                .all(|v| !v.trim().is_empty()),
            "source currency, timezone and URL required",
        )
    }
}
impl PriceBar {
    pub fn fingerprint(&self) -> Result<String> {
        let mut value = serde_json::to_value(self)?;
        value.as_object_mut().unwrap().remove("retrieved_at");
        fingerprint(&value)
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PricePage {
    pub items: Vec<PriceBar>,
    pub next_cursor: Option<String>,
    pub coverage: PriceCoverage,
}
impl PricePage {
    pub fn validate_for(&self, query: &PriceQuery) -> Result<()> {
        query.validate()?;
        self.coverage.validate()?;
        require(
            self.items.len() <= query.page_size,
            "price page exceeds requested size",
        )?;
        require(
            self.next_cursor.as_ref().is_none_or(|c| !c.is_empty()),
            "empty price continuation cursor",
        )?;
        if let (Some(first), Some(last)) = (self.coverage.first_date, self.coverage.last_date) {
            require(
                first >= query.start && last <= query.end,
                "coverage outside requested dates",
            )?;
            require(!self.items.is_empty(), "nonempty coverage with empty page")?;
            let mut previous = None;
            for bar in &self.items {
                bar.validate()?;
                require(
                    bar.instrument == query.instrument && bar.date >= first && bar.date <= last,
                    "price outside requested instrument or coverage",
                )?;
                require(
                    previous.is_none_or(|date| date < bar.date),
                    "price dates must be strictly ascending",
                )?;
                previous = Some(bar.date);
            }
            if query.cursor.is_none() {
                require(
                    self.items.first().unwrap().date == first,
                    "first page does not begin at observed coverage start",
                )?;
            }
            if self.next_cursor.is_none() {
                require(
                    self.items.last().unwrap().date == last,
                    "last page does not end at observed coverage end",
                )?;
            }
        } else {
            require(
                self.items.is_empty() && self.next_cursor.is_none(),
                "empty coverage cannot contain bars or pagination",
            )?;
        }
        Ok(())
    }
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidRequest, message))
    }
}
fn negative(value: &Decimal) -> bool {
    value.as_str().starts_with('-')
        && value
            .as_str()
            .bytes()
            .any(|b| b.is_ascii_digit() && b != b'0')
}
/// Exact comparison of validated nonnegative decimal strings, without floating conversion.
fn compare(left: &Decimal, right: &Decimal) -> Ordering {
    fn parts(value: &Decimal) -> (&str, &str) {
        let value = value.as_str().trim_start_matches('-');
        let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
        (integer.trim_start_matches('0'), fraction)
    }
    let (li, lf) = parts(left);
    let (ri, rf) = parts(right);
    li.len()
        .cmp(&ri.len())
        .then_with(|| li.cmp(ri))
        .then_with(|| {
            lf.bytes()
                .chain(std::iter::repeat(b'0'))
                .take(lf.len().max(rf.len()))
                .cmp(
                    rf.bytes()
                        .chain(std::iter::repeat(b'0'))
                        .take(lf.len().max(rf.len())),
                )
        })
}
pub use crate::capabilities::MarketDataProvider;
