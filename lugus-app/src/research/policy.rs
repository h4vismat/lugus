use super::{ResearchIntent, Workflow};
use crate::{AppError, DatasetRow, ErrorKind, Result};
use chrono::{Months, NaiveDate};
use lugus_financial::resolution::{MatchReason, catalog::CatalogEntry};

pub fn resolution_input(mention: &str) -> Result<String> {
    let text = mention.trim();
    let lower = text.to_ascii_lowercase();
    let cik = lower
        .strip_prefix("cik:")
        .or_else(|| lower.strip_prefix("cik "));
    if let Some(cik) = cik {
        let normalized =
            lugus_financial::resolution::normalize_cik(cik.trim()).map_err(AppError::from)?;
        return Ok(format!("sec:cik:{normalized}"));
    }
    Ok(text.to_owned())
}

pub fn date_range(intent: &ResearchIntent, today: NaiveDate) -> Result<(NaiveDate, NaiveDate)> {
    intent.validate(today)?;
    if let (Some(start), Some(end)) = (intent.start, intent.end) {
        return Ok((start, end));
    }
    let months = if intent.workflow == Workflow::Prices {
        12
    } else {
        60
    };
    let start = today
        .checked_sub_months(Months::new(months))
        .ok_or_else(|| {
            AppError::new(
                ErrorKind::InvalidInput,
                "Date range is outside supported dates",
                false,
            )
        })?;
    Ok((start, today))
}

/// A sole fuzzy candidate is still a suggestion, not a resolved company identity.
pub fn resolve_candidate(
    status: Option<&str>,
    rows: &[DatasetRow],
    next: Option<usize>,
) -> Result<CatalogEntry> {
    if next.is_none()
        && matches!(status, Some("resolved" | "candidates"))
        && let [DatasetRow::Candidate { entry }] = rows
        && entry.candidate.identifier.namespace == "sec:cik"
        && entry
            .candidate
            .match_reasons
            .iter()
            .any(|r| matches!(r, MatchReason::ExactIdentifier | MatchReason::ExactName))
    {
        return Ok(entry.clone());
    }
    Err(AppError::new(
        ErrorKind::NeedsAttention,
        "Please specify the exact company name or ticker and exchange; company identity could not be resolved unambiguously.",
        false,
    ))
}
