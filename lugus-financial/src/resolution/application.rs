//! Explicit provider effects plus catalog persistence; offline reads stay in the catalog.
use super::{
    catalog::{CatalogRepository, ResolutionOutcome},
    *,
};

#[derive(Debug, Serialize)]
pub struct ResolutionAttempt {
    pub runs: Vec<i64>,
    pub outcome: ResolutionOutcome,
}

pub async fn search_and_store<R: CatalogRepository, P: CompanyResolutionProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    mut request: SearchRequest,
    max_pages: usize,
) -> Result<ResolutionAttempt> {
    request.validate()?;
    require(
        max_pages > 0 && max_pages <= 1000,
        "resolution page budget must be 1..1000",
    )?;
    require(request.cursor.is_none(), "root search requires no cursor")?;
    let run = repo.start_resolution_run(provider.identity(), &request)?;
    let fetched: Result<()> = async {
        for _ in 0..max_pages {
            let page = provider.search_companies(&request).await?;
            repo.save_resolution_page(run, &request, &page)?;
            match page.next_cursor {
                None => return Ok(()),
                Some(cursor) => request.cursor = Some(cursor),
            }
        }
        Err(Error::new(
            ErrorKind::InvalidRequest,
            "resolution page budget exhausted; search incomplete",
        ))
    }
    .await;
    if let Err(error) = fetched {
        repo.fail_resolution_run_with_error(run, &error)?;
    }
    Ok(ResolutionAttempt {
        runs: vec![run],
        outcome: repo.resolution_outcome(run)?,
    })
}
pub async fn resolve_input<R: CatalogRepository, P: CompanyResolutionProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    input: &str,
    page_size: usize,
    max_pages: usize,
) -> Result<ResolutionAttempt> {
    let parsed = parse_input(input)?;
    let mut result = search_and_store(
        repo,
        provider,
        SearchRequest {
            query: parsed.primary,
            page_size,
            cursor: None,
        },
        max_pages,
    )
    .await?;
    if matches!(result.outcome, ResolutionOutcome::NoMatch { .. })
        && let Some(query) = parsed.fallback
    {
        let fallback = search_and_store(
            repo,
            provider,
            SearchRequest {
                query,
                page_size,
                cursor: None,
            },
            max_pages,
        )
        .await?;
        result.runs.extend(fallback.runs);
        result.outcome = fallback.outcome;
    }
    Ok(result)
}
pub async fn lookup_and_store<R: CatalogRepository, P: CompanyResolutionProvider + ?Sized>(
    repo: &mut R,
    provider: &mut P,
    request: &LookupRequest,
) -> Result<ResolutionAttempt> {
    request.validate()?;
    let query = SearchRequest {
        query: SearchQuery::Identifier {
            identifier: request.identifier.clone(),
            exchange: None,
        },
        page_size: 1,
        cursor: None,
    };
    let run = repo.start_resolution_run(provider.identity(), &query)?;
    let captured: Result<()> = async {
        let mut candidate = match provider.lookup_company(request).await {
            Ok(candidate) => candidate,
            Err(error) if error.kind == ErrorKind::NotFound => {
                return repo.save_resolution_page(run, &query, &ResolutionPage {
                    items: vec![], next_cursor: None,
                    snapshot: format!("lookup-not-found:{}:{}", request.identifier.namespace, request.identifier.value),
                    coverage: "explicit identifier lookup returned not_found; no source document retrieved".into(),
                });
            }
            Err(error) => return Err(error),
        };
        candidate.validate()?;
        require(
            candidate.identifier == request.identifier,
            "lookup identity mismatch",
        )?;
        candidate.match_reasons = vec![MatchReason::ExactIdentifier];
        repo.save_resolution_page(
            run,
            &query,
            &ResolutionPage {
                snapshot: candidate.source_checksum.clone(),
                coverage: "explicit entity lookup; no directory completeness implied".into(),
                items: vec![candidate],
                next_cursor: None,
            },
        )
    }
    .await;
    if let Err(error) = captured {
        repo.fail_resolution_run_with_error(run, &error)?;
    }
    Ok(ResolutionAttempt {
        runs: vec![run],
        outcome: repo.resolution_outcome(run)?,
    })
}
