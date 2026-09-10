use async_trait::async_trait;
use chrono::{NaiveDate, Utc};
use lugus_financial::{application::*, capabilities::*, domain::*, error::*, storage::*};
fn query() -> Query {
    Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "1".into(),
        },
        filed_from: date("2024-01-01"),
        filed_to: date("2024-12-31"),
        forms: vec![],
        cursor: None,
        page_size: 1,
    }
}
fn date(s: &str) -> NaiveDate {
    s.parse().unwrap()
}
fn fact(value: &str) -> Fact {
    Fact {
        company: query().company,
        namespace: "us-gaap".into(),
        concept: "Assets".into(),
        label: None,
        value: Decimal::new(value).unwrap(),
        unit: "USD".into(),
        period: Period::Instant {
            date: date("2023-12-31"),
        },
        filing_id: "a".into(),
        form: "10-K".into(),
        filed: date("2024-02-01"),
        fiscal_year: None,
        fiscal_period: None,
        source_url: "https://example.test/a".into(),
        retrieved_at: Utc::now(),
    }
}
struct Fixture {
    id: ProviderIdentity,
    value: String,
    fail: bool,
}
impl Fixture {
    fn new() -> Self {
        Self {
            id: ProviderIdentity {
                instance_id: "local".into(),
                plugin_id: "fixture".into(),
                plugin_version: "1".into(),
            },
            value: "12.3400".into(),
            fail: false,
        }
    }
}
impl Provider for Fixture {
    fn identity(&self) -> &ProviderIdentity {
        &self.id
    }
}
#[async_trait]
impl FundamentalsProvider for Fixture {
    async fn fetch_facts(&mut self, q: &Query) -> Result<Page<Fact>> {
        if q.cursor.is_some() {
            return Err(Error::new(ErrorKind::Unavailable, "fixture failure"));
        }
        Ok(Page {
            items: vec![fact(&self.value)],
            next_cursor: self.fail.then(|| "next".into()),
        })
    }
}
#[tokio::test]
async fn refresh_revisions_failure_and_offline_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let mut provider = Fixture::new();
    ingest_facts(&mut repo, &mut provider, &query())
        .await
        .unwrap();
    ingest_facts(&mut repo, &mut provider, &query())
        .await
        .unwrap();
    let snapshot = repo.snapshot(&provider.id, &query()).unwrap();
    assert_eq!(snapshot.facts.len(), 1);
    assert_eq!(snapshot.runs.len(), 2);
    let first = repo.run_observations(snapshot.runs[0].id).unwrap();
    let second = repo.run_observations(snapshot.runs[1].id).unwrap();
    assert_eq!(first[0].observation_id, second[0].observation_id);
    assert!(second[0].retrieved_at >= first[0].retrieved_at);
    assert_eq!(snapshot.facts[0].fact.value.as_str(), "12.3400");
    provider.value = "13".into();
    ingest_facts(&mut repo, &mut provider, &query())
        .await
        .unwrap();
    provider.fail = true;
    assert!(
        ingest_facts(&mut repo, &mut provider, &query())
            .await
            .is_err()
    );
    drop(repo);
    let repo = SqliteRepository::open(&path).unwrap();
    let snapshot = repo.snapshot(&provider.id, &query()).unwrap();
    assert_eq!(snapshot.facts.len(), 2);
    assert_eq!(snapshot.runs.last().unwrap().status, RunStatus::Failed);
    assert_eq!(
        snapshot.runs.last().unwrap().facts_cursor.as_deref(),
        Some("next")
    );
    assert!(!snapshot.is_complete());
}
#[test]
fn page_is_atomic_and_running_is_visible() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let p = Fixture::new();
    let mut q = query();
    q.page_size = 2;
    let id = repo.start_run(&p.id, &q, "facts").unwrap();
    let mut invalid = fact("2");
    invalid.company.value = "other".into();
    assert!(
        repo.save_facts_page(
            id,
            &Page {
                items: vec![fact("1"), invalid],
                next_cursor: Some("next".into())
            },
            &map_metric
        )
        .is_err()
    );
    let s = repo.snapshot(&p.id, &query()).unwrap();
    assert!(s.facts.is_empty());
    assert_eq!(s.runs[0].facts_cursor, None);
    assert_eq!(s.runs[0].status, RunStatus::Running);
}
#[test]
fn document_content_addressing_and_limit() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let p = Fixture::new();
    let doc = Document {
        source_url: "https://example.test/doc".into(),
        media_type: "text/plain".into(),
        content_base64: "aGVsbG8=".into(),
        retrieved_at: Utc::now(),
    };
    assert!(repo.save_document(&p.id, &doc, 4).is_err());
    let checksum = repo.save_document(&p.id, &doc, 5).unwrap();
    assert_eq!(repo.save_document(&p.id, &doc, 5).unwrap(), checksum);
    assert_eq!(repo.stored_document(&checksum).unwrap(), b"hello");
}
#[test]
fn sql_failure_rolls_back_observations_and_cursor() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let mut repo = SqliteRepository::open(&path).unwrap();
    let p = Fixture::new();
    let mut q = query();
    q.page_size = 2;
    let run = repo.start_run(&p.id, &q, "facts").unwrap();
    let connection = rusqlite::Connection::open(&path).unwrap();
    connection.execute_batch("CREATE TRIGGER reject_second BEFORE INSERT ON observations WHEN (SELECT COUNT(*) FROM observations) = 1 BEGIN SELECT RAISE(ABORT, 'simulated disk failure'); END;").unwrap();
    assert!(
        repo.save_facts_page(
            run,
            &Page {
                items: vec![fact("1"), fact("2")],
                next_cursor: Some("next".into())
            },
            &map_metric
        )
        .is_err()
    );
    let s = repo.snapshot(&p.id, &q).unwrap();
    assert!(s.facts.is_empty());
    assert!(s.runs[0].facts_cursor.is_none());
    connection
        .execute_batch("DROP TRIGGER reject_second;")
        .unwrap();
    repo.save_facts_page(
        run,
        &Page {
            items: vec![fact("1")],
            next_cursor: None,
        },
        &map_metric,
    )
    .unwrap();
}
#[tokio::test]
async fn provider_isolation_and_custom_mapping() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let mut p = Fixture::new();
    ingest_facts_with_mapper(&mut repo, &mut p, &query(), &|_| {
        Some(Metric {
            id: "custom".into(),
            mapping_version: "test:1".into(),
        })
    })
    .await
    .unwrap();
    let s = repo.snapshot(&p.id, &query()).unwrap();
    assert_eq!(s.facts[0].metric.as_ref().unwrap().id, "custom");
    p.id.instance_id = "other".into();
    assert!(repo.snapshot(&p.id, &query()).unwrap().facts.is_empty());
}
struct Repeating(Fixture);
impl Provider for Repeating {
    fn identity(&self) -> &ProviderIdentity {
        self.0.identity()
    }
}
#[async_trait]
impl FundamentalsProvider for Repeating {
    async fn fetch_facts(&mut self, _: &Query) -> Result<Page<Fact>> {
        Ok(Page {
            items: vec![fact("1")],
            next_cursor: Some("same".into()),
        })
    }
}
#[tokio::test]
async fn repeated_cursor_marks_failed_and_new_run_restarts() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let mut p = Repeating(Fixture::new());
    assert_eq!(
        ingest_facts(&mut repo, &mut p, &query())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Protocol
    );
    let s = repo.snapshot(p.identity(), &query()).unwrap();
    assert_eq!(s.facts.len(), 1);
    assert_eq!(s.runs[0].status, RunStatus::Failed);
    let mut q = query();
    p.0.value = "1".into();
    q.cursor = Some("stale".into());
    ingest_facts(&mut repo, &mut p.0, &q).await.unwrap();
    let s = repo.snapshot(p.identity(), &q).unwrap();
    assert_eq!(s.runs[1].status, RunStatus::Complete);
    assert!(s.runs[1].query.cursor.is_none());
    assert_eq!(s.facts.len(), 1);
}
#[test]
fn distinct_disclosures_and_mapping_revisions_are_preserved() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let p = Fixture::new();
    let mut q = query();
    q.page_size = 2;
    let run = repo.start_run(&p.id, &q, "facts").unwrap();
    let first = fact("1");
    let mut second = first.clone();
    second.filing_id = "b".into();
    let page = Page {
        items: vec![first, second],
        next_cursor: None,
    };
    repo.save_facts_page(run, &page, &map_metric).unwrap();
    repo.finish_run(run, None).unwrap();
    let run = repo.start_run(&p.id, &q, "facts").unwrap();
    repo.save_facts_page(run, &page, &|_| None).unwrap();
    repo.finish_run(run, None).unwrap();
    let s = repo.snapshot(&p.id, &q).unwrap();
    assert_eq!(s.facts.len(), 4);
    assert!(s.is_complete());
}
#[test]
fn future_schema_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("db.sqlite");
    let conn = rusqlite::Connection::open(&path).unwrap();
    conn.execute_batch("PRAGMA user_version = 6;").unwrap();
    assert!(matches!(
        SqliteRepository::open(path),
        Err(Error {
            kind: ErrorKind::Persistence,
            ..
        })
    ));
}
#[test]
fn wrong_company_date_and_form_pages_are_rejected_atomically() {
    let mut repo = SqliteRepository::open(":memory:").unwrap();
    let p = Fixture::new();
    let mut q = query();
    q.page_size = 2;
    q.forms = vec!["10-K".into()];
    let mut wrong_company = fact("2");
    wrong_company.company.value = "other".into();
    let mut wrong_date = fact("2");
    wrong_date.filed = date("2025-01-01");
    let mut wrong_form = fact("2");
    wrong_form.form = "10-Q".into();
    for invalid in [wrong_company, wrong_date, wrong_form] {
        let run = repo.start_run(&p.id, &q, "both").unwrap();
        let filing = |f: &Fact| Filing {
            company: f.company.clone(),
            filing_id: f.filing_id.clone(),
            form: f.form.clone(),
            filed: f.filed,
            report_date: None,
            accepted_at: None,
            primary_document: None,
            source_url: f.source_url.clone(),
            retrieved_at: f.retrieved_at,
        };
        assert!(
            repo.save_filings_page(
                run,
                &Page {
                    items: vec![filing(&fact("1")), filing(&invalid)],
                    next_cursor: Some("bad".into())
                }
            )
            .is_err()
        );
        assert!(
            repo.save_facts_page(
                run,
                &Page {
                    items: vec![fact("1"), invalid],
                    next_cursor: Some("bad".into())
                },
                &map_metric
            )
            .is_err()
        );
        let s = repo.snapshot(&p.id, &q).unwrap();
        assert!(s.facts.is_empty());
        assert!(s.filings.is_empty());
        assert!(
            s.runs
                .iter()
                .all(|r| r.facts_cursor.is_none() && r.filings_cursor.is_none())
        );
    }
}
