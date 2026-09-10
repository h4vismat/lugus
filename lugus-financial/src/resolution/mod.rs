//! Provider-neutral entity discovery. Identity reconciliation belongs to the host catalog.
use crate::{
    capabilities::Provider,
    domain::{CompanyId, Validate},
    error::{Error, ErrorKind, Result},
};
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum SearchQuery {
    Name {
        text: String,
    },
    Identifier {
        identifier: CompanyId,
        exchange: Option<CompanyId>,
    },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SearchRequest {
    pub query: SearchQuery,
    pub page_size: usize,
    pub cursor: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LookupRequest {
    pub identifier: CompanyId,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Listing {
    pub ticker: CompanyId,
    pub exchange: Option<CompanyId>,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MatchReason {
    ExactIdentifier,
    ExactName,
    NameSubstring,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Candidate {
    pub identifier: CompanyId,
    pub name: String,
    pub aliases: Vec<String>,
    pub listings: Vec<Listing>,
    pub source_url: String,
    pub source_checksum: String,
    pub retrieved_at: DateTime<Utc>,
    pub match_reasons: Vec<MatchReason>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResolutionPage {
    pub items: Vec<Candidate>,
    pub next_cursor: Option<String>,
    pub snapshot: String,
    pub coverage: String,
}
#[async_trait]
pub trait CompanyResolutionProvider: Provider + Send {
    async fn search_companies(&mut self, request: &SearchRequest) -> Result<ResolutionPage>;
    async fn lookup_company(&mut self, request: &LookupRequest) -> Result<Candidate>;
}
fn require(valid: bool, message: &str) -> Result<()> {
    if valid {
        Ok(())
    } else {
        Err(Error::new(ErrorKind::InvalidRequest, message))
    }
}
fn text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.chars().any(char::is_control)
}
fn identifier(value: &CompanyId) -> Result<()> {
    require(
        text(&value.namespace, 128) && text(&value.value, 128),
        "invalid identifier",
    )?;
    if value.namespace == "sec:cik" {
        require(
            normalize_cik(&value.value)? == value.value,
            "SEC CIK must be normalized",
        )?;
    }
    Ok(())
}
pub fn normalize_cik(value: &str) -> Result<String> {
    require(
        !value.is_empty()
            && value.len() <= 10
            && value.bytes().all(|b| b.is_ascii_digit())
            && value.bytes().any(|b| b != b'0'),
        "invalid SEC CIK",
    )?;
    Ok(format!("{value:0>10}"))
}
pub fn normalized_name(value: &str) -> String {
    value
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}
impl Validate for SearchQuery {
    fn validate(&self) -> Result<()> {
        match self {
            Self::Name { text: value } => require(text(value, 256), "invalid search name"),
            Self::Identifier {
                identifier: value,
                exchange,
            } => {
                identifier(value)?;
                if let Some(exchange) = exchange {
                    identifier(exchange)?;
                }
                Ok(())
            }
        }
    }
}
impl Validate for SearchRequest {
    fn validate(&self) -> Result<()> {
        self.query.validate()?;
        require(
            (1..=100).contains(&self.page_size),
            "invalid resolution page size",
        )?;
        require(
            self.cursor.as_ref().is_none_or(|v| text(v, 1024)),
            "invalid resolution cursor",
        )
    }
}
impl Validate for LookupRequest {
    fn validate(&self) -> Result<()> {
        identifier(&self.identifier)
    }
}
impl Validate for Candidate {
    fn validate(&self) -> Result<()> {
        identifier(&self.identifier)?;
        require(text(&self.name, 1024), "invalid company name")?;
        require(
            self.aliases.len() <= 100 && self.aliases.iter().all(|v| text(v, 1024)),
            "invalid aliases",
        )?;
        require(self.listings.len() <= 1000, "too many listings")?;
        for listing in &self.listings {
            identifier(&listing.ticker)?;
            if let Some(exchange) = &listing.exchange {
                identifier(exchange)?;
            }
        }
        require(text(&self.source_url, 4096), "invalid source URL")?;
        require(
            self.source_checksum.len() == 64
                && self.source_checksum.bytes().all(|b| b.is_ascii_hexdigit()),
            "invalid source checksum",
        )?;
        require(self.match_reasons.len() <= 3, "invalid match reasons")
    }
}
pub fn match_reasons(query: &SearchQuery, candidate: &Candidate) -> Vec<MatchReason> {
    match query {
        SearchQuery::Name { text } => {
            let needle = normalized_name(text);
            let names = std::iter::once(&candidate.name)
                .chain(candidate.aliases.iter())
                .map(|n| normalized_name(n))
                .collect::<Vec<_>>();
            if names.iter().any(|n| n == &needle) {
                vec![MatchReason::ExactName]
            } else if names.iter().any(|n| n.contains(&needle)) {
                vec![MatchReason::NameSubstring]
            } else {
                vec![]
            }
        }
        SearchQuery::Identifier {
            identifier,
            exchange,
        } => {
            let equal = |a: &CompanyId, b: &CompanyId| {
                a.namespace == b.namespace
                    && if a.namespace == "sec:ticker" || a.namespace == "sec:exchange" {
                        a.value.trim().eq_ignore_ascii_case(b.value.trim())
                    } else {
                        a.value == b.value
                    }
            };
            let found = (exchange.is_none() && equal(identifier, &candidate.identifier))
                || candidate.listings.iter().any(|listing| {
                    equal(identifier, &listing.ticker)
                        && exchange
                            .as_ref()
                            .is_none_or(|e| listing.exchange.as_ref().is_some_and(|x| equal(e, x)))
                });
            if found {
                vec![MatchReason::ExactIdentifier]
            } else {
                vec![]
            }
        }
    }
}
impl ResolutionPage {
    pub fn validate_for(&self, request: &SearchRequest) -> Result<()> {
        request.validate()?;
        require(
            self.items.len() <= request.page_size,
            "resolution page exceeds requested size",
        )?;
        require(
            text(&self.snapshot, 1024) && text(&self.coverage, 4096),
            "resolution source scope required",
        )?;
        require(
            self.next_cursor.as_ref().is_none_or(|v| text(v, 1024)),
            "invalid next cursor",
        )?;
        let mut identities = std::collections::HashSet::new();
        for candidate in &self.items {
            candidate.validate()?;
            require(
                identities.insert(candidate.identifier.clone()),
                "duplicate entity in page",
            )?;
            let expected = match_reasons(&request.query, candidate);
            require(
                !expected.is_empty() && candidate.match_reasons == expected,
                "candidate does not match requested query",
            )?;
        }
        Ok(())
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputQuery {
    pub primary: SearchQuery,
    pub fallback: Option<SearchQuery>,
}
pub fn parse_input(input: &str) -> Result<InputQuery> {
    require(
        !input.chars().any(char::is_control),
        "control characters in input",
    )?;
    let value = input.trim();
    require(text(value, 256), "invalid resolution input")?;
    if let Some(cik) = value.strip_prefix("sec:cik:") {
        return Ok(InputQuery {
            primary: SearchQuery::Identifier {
                identifier: CompanyId {
                    namespace: "sec:cik".into(),
                    value: normalize_cik(cik)?,
                },
                exchange: None,
            },
            fallback: None,
        });
    }
    let (symbol, explicit) = value
        .strip_prefix('$')
        .map_or((value, false), |s| (s, true));
    let ticker = !symbol.is_empty()
        && symbol.len() <= 128
        && symbol
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_' | b'^' | b'='));
    if explicit {
        require(ticker, "invalid cashtag")?;
    }
    if ticker {
        Ok(InputQuery {
            primary: SearchQuery::Identifier {
                identifier: CompanyId {
                    namespace: "sec:ticker".into(),
                    value: symbol.to_ascii_uppercase(),
                },
                exchange: None,
            },
            fallback: if explicit {
                None
            } else {
                Some(SearchQuery::Name { text: value.into() })
            },
        })
    } else {
        Ok(InputQuery {
            primary: SearchQuery::Name { text: value.into() },
            fallback: None,
        })
    }
}

pub mod catalog;

pub mod application;
