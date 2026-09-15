//! Calendar-complete, trading-date share-basis evidence, independent of ordinary price charts.
use crate::{
    domain::{Decimal, Validate},
    error::{Error, ErrorKind, Result},
    market_data::{Completeness, InstrumentId},
};
use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HistoryQuery {
    pub instrument: InstrumentId,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub anchor: NaiveDate,
    pub cursor: Option<String>,
    pub page_size: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SplitRatio {
    pub numerator: u64,
    pub denominator: u64,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryManifest {
    pub instrument: InstrumentId,
    pub requested_start: NaiveDate,
    pub requested_end: NaiveDate,
    pub coverage_start: NaiveDate,
    pub anchor: NaiveDate,
    pub last_completed_session: NaiveDate,
    pub currency: String,
    pub exchange_timezone: String,
    pub calendar: String,
    pub calendar_version: String,
    pub normalization_version: u32,
    pub source_basis: String,
    pub completeness: Completeness,
    pub retrieved_at: DateTime<Utc>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryDay {
    pub date: NaiveDate,
    pub market_close: Option<DateTime<Utc>>,
    pub source_close: Option<Decimal>,
    pub close: Option<Decimal>,
    pub factor_to_anchor: Decimal,
    pub split: Option<SplitRatio>,
    pub unsupported_action: Option<String>,
    pub source_url: String,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HistoryPage {
    pub manifest: HistoryManifest,
    pub items: Vec<HistoryDay>,
    pub next_cursor: Option<String>,
}
fn require(ok: bool, message: &str) -> Result<()> {
    if ok {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidRequest, message))
    }
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn nonnegative(value: &Decimal) -> bool {
    !value.as_str().starts_with('-')
        || value
            .as_str()
            .bytes()
            .all(|b| matches!(b, b'-' | b'.' | b'0'))
}
fn positive(value: &Decimal) -> bool {
    nonnegative(value) && value.as_str().bytes().any(|b| matches!(b, b'1'..=b'9'))
}
impl Validate for HistoryQuery {
    fn validate(&self) -> Result<()> {
        require(
            text(&self.instrument.namespace, 128) && text(&self.instrument.value, 128),
            "invalid historical instrument",
        )?;
        require(
            self.start <= self.end
                && self.end <= self.anchor
                && (self.anchor - self.start).num_days() < 100_000,
            "invalid history range or anchor",
        )?;
        require(
            (1..=200).contains(&self.page_size),
            "historical pages contain at most 200 rows",
        )?;
        require(
            self.cursor.as_ref().is_none_or(|c| text(c, 256)),
            "invalid history cursor",
        )
    }
}
impl HistoryPage {
    pub fn validate_for(&self, query: &HistoryQuery) -> Result<()> {
        query.validate()?;
        let m = &self.manifest;
        require(
            m.instrument == query.instrument
                && m.requested_start == query.start
                && m.requested_end == query.end
                && m.anchor == query.anchor,
            "history manifest does not match query",
        )?;
        require(
            m.coverage_start < query.start
                && m.last_completed_session >= m.coverage_start
                && m.last_completed_session <= m.anchor
                && m.anchor <= m.retrieved_at.date_naive(),
            "invalid historical coverage",
        )?;
        require(
            m.currency == "USD"
                && m.exchange_timezone == "America/New_York"
                && matches!(m.calendar.as_str(), "NYSE" | "NASDAQ")
                && m.calendar_version == "5.4.0"
                && m.normalization_version == 1,
            "unsupported history currency, calendar or normalization",
        )?;
        let benchmark =
            m.instrument.namespace == "yahoo:symbol" && m.instrument.value == "^SP500TR";
        require(
            m.source_basis
                == if benchmark {
                    "total_return_index"
                } else {
                    "yahoo_split_adjusted_close"
                },
            "unsupported historical price basis",
        )?;
        require(
            !self.items.is_empty()
                && self.items.len() <= query.page_size
                && self.next_cursor.as_ref().is_none_or(|c| text(c, 256)),
            "invalid history page size or cursor",
        )?;
        let mut previous = None;
        for day in &self.items {
            require(
                day.date >= m.coverage_start
                    && day.date <= m.anchor
                    && previous.is_none_or(|p: NaiveDate| p.succ_opt() == Some(day.date)),
                "historical calendar dates must be consecutive",
            )?;
            previous = Some(day.date);
            require(
                day.source_close.is_some() == day.close.is_some()
                    && day
                        .close
                        .iter()
                        .chain(day.source_close.iter())
                        .all(nonnegative)
                    && positive(&day.factor_to_anchor),
                "invalid historical close or split factor",
            )?;
            require(
                text(&day.source_url, 4096)
                    && day.unsupported_action.as_ref().is_none_or(|s| text(s, 256)),
                "invalid historical source metadata",
            )?;
            if let Some(close) = day.market_close {
                require(
                    close.date_naive() == day.date,
                    "session close disagrees with trading date",
                )?;
                require(
                    day.close.is_none()
                        || (close <= m.retrieved_at && day.date <= m.last_completed_session),
                    "price belongs to an incomplete session",
                )?;
            } else {
                require(
                    day.close.is_none() && day.split.is_none(),
                    "scheduled closure cannot contain price or split",
                )?;
            }
            if let Some(split) = &day.split {
                require(
                    split.numerator > 0 && split.denominator > 0 && !benchmark,
                    "invalid split ratio",
                )?;
            }
            if benchmark {
                require(
                    day.factor_to_anchor.as_str() == "1" && day.source_close == day.close,
                    "index level cannot be adjusted twice",
                )?;
            }
        }
        if query.cursor.is_none() {
            require(
                self.items[0].date == m.coverage_start,
                "first history page misses coverage start",
            )?;
        }
        if self.next_cursor.is_none() {
            require(
                self.items.last().unwrap().date == m.anchor,
                "last history page misses anchor",
            )?;
        }
        Ok(())
    }
}
