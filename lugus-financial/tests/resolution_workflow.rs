use lugus_financial::{
    capabilities::Provider,
    domain::ProviderIdentity,
    error::{Error, ErrorKind, Result},
    resolution::{application::*, catalog::*, *},
    storage::SqliteRepository,
};
use serde_json::json;
struct Source {
    identity: ProviderIdentity,
    calls: usize,
    fail: bool,
}
impl Provider for Source {
    fn identity(&self) -> &ProviderIdentity {
        &self.identity
    }
}
#[async_trait::async_trait]
impl CompanyResolutionProvider for Source {
    async fn search_companies(&mut self, r: &SearchRequest) -> Result<ResolutionPage> {
        self.calls += 1;
        if self.fail {
            return Err(Error::new(ErrorKind::Unavailable, "source unavailable"));
        }
        let items = if matches!(r.query, SearchQuery::Name { .. }) {
            vec![serde_json::from_value(json!({"identifier":{"namespace":"sec:cik","value":"0000051143"},"name":"IBM","aliases":[],"listings":[],"source_url":"https://example.com/source","source_checksum":"a".repeat(64),"retrieved_at":"2026-09-10T00:00:00Z","match_reasons":["exact_name"]})).unwrap()]
        } else {
            vec![]
        };
        Ok(ResolutionPage {
            items,
            next_cursor: None,
            snapshot: "one".into(),
            coverage: "fixture".into(),
        })
    }
    async fn lookup_company(&mut self, _: &LookupRequest) -> Result<Candidate> {
        Err(Error::new(ErrorKind::Unsupported, "fixture"))
    }
}
fn source(fail: bool) -> Source {
    Source {
        identity: ProviderIdentity {
            instance_id: "local".into(),
            plugin_id: "fixture".into(),
            plugin_version: "1".into(),
        },
        calls: 0,
        fail,
    }
}
#[tokio::test]
async fn bare_ticker_falls_back_only_after_completed_empty_search() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = SqliteRepository::open(dir.path().join("db")).unwrap();
    let mut p = source(false);
    let result = resolve_input(&mut repo, &mut p, "IBM", 10, 10)
        .await
        .unwrap();
    assert_eq!(p.calls, 2);
    assert_eq!(result.runs.len(), 2);
    assert!(matches!(
        result.outcome,
        ResolutionOutcome::Candidates { .. }
    ));
    let mut p = source(false);
    let result = resolve_input(&mut repo, &mut p, "$IBM", 10, 10)
        .await
        .unwrap();
    assert_eq!(p.calls, 1);
    assert!(matches!(result.outcome, ResolutionOutcome::NoMatch { .. }));
}
#[tokio::test]
async fn provider_failure_is_persisted_without_name_fallback() {
    let dir = tempfile::tempdir().unwrap();
    let mut repo = SqliteRepository::open(dir.path().join("db")).unwrap();
    let mut p = source(true);
    let result = resolve_input(&mut repo, &mut p, "IBM", 10, 10)
        .await
        .unwrap();
    assert_eq!(p.calls, 1);
    assert!(matches!(
        result.outcome,
        ResolutionOutcome::Incomplete { error: Some(_), .. }
    ));
}

struct LookupSource {
    identity: ProviderIdentity,
    kind: ErrorKind,
}
impl Provider for LookupSource {
    fn identity(&self) -> &ProviderIdentity {
        &self.identity
    }
}
#[async_trait::async_trait]
impl CompanyResolutionProvider for LookupSource {
    async fn search_companies(&mut self, _: &SearchRequest) -> Result<ResolutionPage> {
        unreachable!()
    }
    async fn lookup_company(&mut self, _: &LookupRequest) -> Result<Candidate> {
        Err(Error {
            kind: self.kind,
            message: "lookup result".into(),
            retry_after_seconds: Some(7),
        })
    }
}
#[tokio::test]
async fn absent_lookup_differs_from_rate_limit_with_retry_metadata_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let request = LookupRequest {
        identifier: lugus_financial::domain::CompanyId {
            namespace: "sec:cik".into(),
            value: "0000051143".into(),
        },
    };
    let mut source = LookupSource {
        identity: source(false).identity,
        kind: ErrorKind::NotFound,
    };
    assert!(matches!(
        lookup_and_store(&mut repo, &mut source, &request)
            .await
            .unwrap()
            .outcome,
        ResolutionOutcome::NoMatch { .. }
    ));
    source.kind = ErrorKind::RateLimited;
    let result = lookup_and_store(&mut repo, &mut source, &request)
        .await
        .unwrap();
    let run = result.runs[0];
    drop(repo);
    let repo = SqliteRepository::open(path).unwrap();
    let value = serde_json::to_value(repo.resolution_outcome(run).unwrap()).unwrap();
    assert_eq!(value["failure"]["kind"], "rate_limited");
    assert_eq!(value["failure"]["retry_after_seconds"], 7);
}
