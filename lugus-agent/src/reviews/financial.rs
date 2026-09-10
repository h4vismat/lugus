//! Captures public financial snapshots; never reads financial table internals.
use super::{Evidence, ReviewError, ReviewResult};
use lugus_financial::{
    domain::{ProviderIdentity, Query, fingerprint},
    storage::Repository,
};
use serde_json::json;

/// Includes immutable facts, filing metadata and ingestion scope. Does not fetch
/// documents or research, select a single "latest" fact, or imply full coverage.
/// Callers may explicitly select a smaller subset before enqueueing a review.
pub fn capture_financial_evidence(
    repository: &dyn Repository,
    provider: &ProviderIdentity,
    query: &Query,
) -> ReviewResult<Vec<Evidence>> {
    let financial_error = |e: lugus_financial::error::Error| ReviewError::Financial(e.to_string());
    let snapshot = repository
        .snapshot(provider, query)
        .map_err(financial_error)?;
    let provider_key = fingerprint(provider).map_err(financial_error)?;
    let runs: Vec<_> = snapshot
        .runs
        .iter()
        .map(|r| json!({"id":r.id,"query":r.query,"operation":r.operation,"status":r.status}))
        .collect();
    let mut items = vec![Evidence::new(
        &format!("financial:{provider_key}:scope"),
        "Financial snapshot scope",
        json!({
            "kind":"snapshot_scope", "provider":provider, "query":query, "all_runs_complete":snapshot.is_complete(),
            "coverage_note":"Run completion does not establish source completeness or range coverage. Facts include historical revisions; filing entries are metadata, not document contents.", "runs":runs
        }),
    )?];
    for observation in snapshot.facts {
        let key = observation.fact.fingerprint().map_err(financial_error)?;
        items.push(Evidence::new(
            &format!("financial:{provider_key}:fact:{key}"),
            &observation.fact.concept,
            json!({"kind":"fact","provider":provider,"observation":observation}),
        )?);
    }
    for filing in snapshot.filings {
        let key = filing.fingerprint().map_err(financial_error)?;
        items.push(Evidence::new(
            &format!("financial:{provider_key}:filing:{key}"),
            &format!("{} {}", filing.form, filing.filing_id),
            json!({"kind":"filing_metadata","provider":provider,"filing":filing}),
        )?);
    }
    Ok(items)
}
