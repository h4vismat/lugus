use lugus_financial::{
    domain::CompanyId,
    instruments::{InstrumentKind, InstrumentMetadata},
    resolution::{Candidate, Listing},
};
use serde::{Deserialize, Serialize};
pub const POLICY: &str = "instrument-binding-v1";
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BindingAssessment {
    Supported {
        policy: String,
        reasons: Vec<String>,
    },
    Incomplete {
        reasons: Vec<String>,
    },
    Conflict {
        reasons: Vec<String>,
    },
}
fn name(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_punctuation() {
                ' '
            } else {
                c.to_ascii_lowercase()
            }
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}
fn venue(value: &CompanyId) -> Option<&'static str> {
    match (
        value.namespace.as_str(),
        value.value.trim().to_ascii_uppercase().as_str(),
    ) {
        ("sec:exchange", "NASDAQ") | ("yahoo:exchange", "NMS" | "NGM" | "NCM" | "NASDAQ") => {
            Some("Nasdaq")
        }
        ("sec:exchange", "NYSE") | ("yahoo:exchange", "NYQ" | "NYSE") => Some("Nyse"),
        _ => None,
    }
}
/// Pure policy: only retained source evidence can establish support.
pub fn assess_binding(
    candidate: &Candidate,
    listing: &Listing,
    metadata: &InstrumentMetadata,
) -> BindingAssessment {
    let mut conflicts = Vec::new();
    let mut missing = Vec::new();
    if !candidate.listings.contains(listing) {
        conflicts.push("selected listing is outside the company observation".into());
    }
    if candidate.identifier.namespace != "sec:cik" {
        missing.push("unsupported company identifier namespace".into());
    }
    match metadata.kind {
        Some(InstrumentKind::Equity) => (),
        Some(InstrumentKind::Other) => conflicts.push("instrument is not equity".into()),
        None => missing.push("instrument type is missing".into()),
    }
    if listing.ticker.namespace != "sec:ticker" {
        missing.push("unsupported listing ticker namespace".into());
    }
    match &metadata.ticker {
        Some(t) if !t.trim().eq_ignore_ascii_case(listing.ticker.value.trim()) => {
            conflicts.push("listing ticker differs".into())
        }
        None => missing.push("instrument ticker is missing".into()),
        _ => (),
    }
    match (
        listing.exchange.as_ref().and_then(venue),
        metadata.exchange.as_ref().and_then(venue),
    ) {
        (Some(a), Some(b)) if a != b => conflicts.push("listing exchange differs".into()),
        (Some(_), Some(_)) => (),
        _ => missing.push("listing exchange is missing or unsupported".into()),
    }
    let shared: Vec<_> = metadata
        .issuer_identifiers
        .iter()
        .filter(|i| i.namespace == candidate.identifier.namespace)
        .collect();
    if shared.iter().any(|i| i.value != candidate.identifier.value) {
        conflicts.push("issuer identifier contradicts company".into());
    } else if shared.is_empty() {
        match &metadata.issuer_name {
            None => missing.push("issuer evidence is missing".into()),
            Some(n)
                if name(n) != name(&candidate.name)
                    && !candidate.aliases.iter().any(|a| name(a) == name(n)) =>
            {
                conflicts.push("issuer name differs".into())
            }
            _ => (),
        }
    }
    if !conflicts.is_empty() {
        BindingAssessment::Conflict { reasons: conflicts }
    } else if !missing.is_empty() {
        BindingAssessment::Incomplete { reasons: missing }
    } else {
        BindingAssessment::Supported {
            policy: POLICY.into(),
            reasons: vec![
                "source_supported: issuer, selected ticker, exchange and equity type agree".into(),
            ],
        }
    }
}
