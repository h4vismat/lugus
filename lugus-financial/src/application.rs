//! Explicit provider refresh; repository snapshots are always offline.
pub mod history;
use crate::{
    capabilities::*,
    domain::*,
    error::{Error, ErrorKind, Result},
    storage::Repository,
};
use std::collections::HashSet;
fn next_cursor(cursor: Option<String>, seen: &mut HashSet<String>) -> Result<Option<String>> {
    if let Some(ref value) = cursor
        && (value.is_empty() || !seen.insert(value.clone()))
    {
        return Err(Error::new(
            ErrorKind::Protocol,
            "provider repeated or returned an empty pagination cursor",
        ));
    }
    Ok(cursor)
}
async fn filings<R: Repository, P: FilingsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    run: i64,
    query: &Query,
) -> Result<()> {
    let mut q = query.clone();
    q.cursor = None;
    let mut seen = HashSet::new();
    loop {
        let page = provider.list_filings(&q).await?;
        let cursor = next_cursor(page.next_cursor.clone(), &mut seen)?;
        repo.save_filings_page(run, &page)?;
        q.cursor = cursor;
        if q.cursor.is_none() {
            return Ok(());
        }
    }
}
async fn facts<R: Repository, P: FundamentalsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    run: i64,
    query: &Query,
    mapper: &dyn Fn(&Fact) -> Option<Metric>,
) -> Result<()> {
    let mut q = query.clone();
    q.cursor = None;
    let mut seen = HashSet::new();
    loop {
        let page = provider.fetch_facts(&q).await?;
        let cursor = next_cursor(page.next_cursor.clone(), &mut seen)?;
        repo.save_facts_page(run, &page, mapper)?;
        q.cursor = cursor;
        if q.cursor.is_none() {
            return Ok(());
        }
    }
}
fn finish<R: Repository>(repo: &mut R, run: i64, outcome: Result<()>) -> Result<i64> {
    repo.finish_run(run, outcome.as_ref().err())?;
    outcome.map(|()| run)
}
pub async fn ingest<R: Repository, P: FilingsProvider + FundamentalsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &Query,
) -> Result<i64> {
    ingest_with_mapper(repo, provider, query, &map_metric).await
}
pub async fn ingest_with_mapper<
    R: Repository,
    P: FilingsProvider + FundamentalsProvider + ?Sized,
>(
    repo: &mut R,
    provider: &mut P,
    query: &Query,
    mapper: &dyn Fn(&Fact) -> Option<Metric>,
) -> Result<i64> {
    query.validate()?;
    let run = repo.start_run(provider.identity(), query, "both")?;
    let result = async {
        filings(repo, provider, run, query).await?;
        facts(repo, provider, run, query, mapper).await
    }
    .await;
    finish(repo, run, result)
}
pub async fn ingest_filings<R: Repository, P: FilingsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &Query,
) -> Result<i64> {
    query.validate()?;
    let run = repo.start_run(provider.identity(), query, "filings")?;
    let result = filings(repo, provider, run, query).await;
    finish(repo, run, result)
}
pub async fn ingest_facts<R: Repository, P: FundamentalsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &Query,
) -> Result<i64> {
    ingest_facts_with_mapper(repo, provider, query, &map_metric).await
}
pub async fn ingest_facts_with_mapper<R: Repository, P: FundamentalsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    query: &Query,
    mapper: &dyn Fn(&Fact) -> Option<Metric>,
) -> Result<i64> {
    query.validate()?;
    let run = repo.start_run(provider.identity(), query, "facts")?;
    let result = facts(repo, provider, run, query, mapper).await;
    finish(repo, run, result)
}
pub async fn retrieve_document<R: Repository, P: FilingsProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    source_url: &str,
    max_bytes: usize,
) -> Result<String> {
    if source_url.trim().is_empty() || max_bytes == 0 {
        return Err(Error::new(
            ErrorKind::InvalidRequest,
            "document URL and positive size limit are required",
        ));
    }
    let document = provider.fetch_document(source_url, max_bytes).await?;
    if document.source_url != source_url {
        return Err(Error::new(
            ErrorKind::MalformedData,
            "document source URL differs from requested URL",
        ));
    }
    repo.save_document(provider.identity(), &document, max_bytes)
}

pub mod market;
