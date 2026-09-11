use lugus_app::*;
use lugus_financial::{
    domain::*,
    market_data::*,
    selection::{MetricQuery, PeriodSelection, PriceSeries},
    storage::{Repository, SqliteRepository, market::MarketRepository},
};
use std::{
    path::Path,
    sync::atomic::{AtomicU64, Ordering},
};
struct TestClock;
impl Clock for TestClock {
    fn now(&self) -> chrono::DateTime<chrono::Utc> {
        "2026-09-10T00:00:00Z".parse().unwrap()
    }
}
struct TestIds(AtomicU64);
impl IdSource for TestIds {
    fn next_id(&self) -> String {
        format!("ref-{}", self.0.fetch_add(1, Ordering::SeqCst))
    }
}
fn app_scope() -> Scope {
    Scope {
        workspace_id: "workspace".into(),
        request_id: "request".into(),
        run_id: None,
    }
}
fn store(app: &Path, financial: &Path) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        app,
        Box::new(SqliteRepository::open(financial).unwrap()),
        Limits::default(),
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap()
}
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "local".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn query() -> PriceQuery {
    PriceQuery {
        instrument: InstrumentId {
            namespace: "fixture:symbol".into(),
            value: "IBM".into(),
        },
        start: "2024-01-01".parse().unwrap(),
        end: "2024-12-31".parse().unwrap(),
        cursor: None,
        page_size: 100,
    }
}
fn scope() -> Query {
    Query {
        company: CompanyId {
            namespace: "sec:cik".into(),
            value: "51143".into(),
        },
        filed_from: query().start,
        filed_to: query().end,
        forms: vec![],
        cursor: None,
        page_size: 100,
    }
}
fn metric() -> MetricQuery {
    MetricQuery {
        scope: scope(),
        namespace: "us-gaap".into(),
        concept: "Assets".into(),
        unit: "USD".into(),
        periods: PeriodSelection::LatestInstant,
    }
}
fn bar(value: &str, day: &str) -> PriceBar {
    PriceBar {
        instrument: query().instrument,
        date: day.parse().unwrap(),
        open: Decimal::new(value).unwrap(),
        high: Decimal::new(value).unwrap(),
        low: Decimal::new(value).unwrap(),
        close: Decimal::new(value).unwrap(),
        volume: 10,
        adjusted_close: None,
        currency: "USD".into(),
        exchange_timezone: "America/New_York".into(),
        price_basis: PriceBasis::SourceReported,
        precision: PricePrecision::DecimalSource,
        source_url: "https://fixture.test/prices".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    }
}
fn page(items: Vec<PriceBar>) -> PricePage {
    PricePage {
        coverage: PriceCoverage {
            first_date: items.first().map(|b| b.date),
            last_date: items.last().map(|b| b.date),
            completeness: Completeness::Unverified,
        },
        items,
        next_cursor: None,
    }
}

fn ingest(repo: &mut SqliteRepository, value: &str) -> i64 {
    let run = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(run, &page(vec![bar(value, "2024-01-02")]))
        .unwrap();
    repo.finish_market_run(run, None).unwrap();
    run
}
fn receipt(repo: &SqliteRepository, run: i64) -> FetchResult {
    FetchResult {
        provenance: FetchProvenance {
            scope: app_scope(),
            provider: provider(),
            repository_id: repo.repository_identity().unwrap(),
            command: FetchCommand::Prices {
                instance_id: "local".into(),
                query: query(),
            },
            runs: vec![RunReceipt {
                kind: RunKind::Market,
                id: run,
            }],
            document: None,
            instrument_observation: None,
            binding_id: None,
        },
        error: None,
    }
}
fn freeze(s: &mut impl ApplicationStore, r: &FetchResult) -> DatasetHeader {
    let f = s.record_fetch(r).unwrap();
    s.create_dataset(
        &app_scope(),
        &f.id,
        DatasetProjection::Prices {
            run_id: r.provenance.runs[0].id,
            query: query(),
            series: PriceSeries::Close,
        },
    )
    .unwrap()
}
#[test]
fn frozen_prices_survive_reopen_new_ingestion_and_scope_denial() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.sqlite");
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "9007199254740993.00");
    let mut s = store(&db, &fin);
    let d = freeze(&mut s, &receipt(&repo, run));
    drop(s);
    ingest(&mut repo, "4");
    let s = store(&db, &fin);
    let p = s
        .read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
    let DatasetRow::Price { evidence, value } = &p.rows[0] else {
        panic!("price row")
    };
    assert_eq!(value.as_ref().unwrap().as_str(), "9007199254740993.00");
    assert_eq!(evidence.value.close.as_str(), "9007199254740993.00");
    assert!(p.header.coverage.is_some());
    let mut other = app_scope();
    other.workspace_id = "other".into();
    assert_eq!(
        s.read_dataset(
            &other,
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
    let other_fin = dir.path().join("other.sqlite");
    let s = store(&db, &other_fin);
    assert_eq!(
        s.read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn view_receipts_dedupe_and_renderer_failure_is_separate() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "2");
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let d = freeze(&mut s, &receipt(&repo, run));
    let request = OpenViewRequest {
        dataset_id: d.id.clone(),
        kind: ViewKind::PriceChart,
    };
    let a = s.open_view(&app_scope(), &request).unwrap();
    let b = s.open_view(&app_scope(), &request).unwrap();
    assert_eq!(a.id, b.id);
    assert_eq!(a.descriptor_revision, 1);
    let different = OpenViewRequest {
        kind: ViewKind::DataTable,
        ..request.clone()
    };
    assert_eq!(
        s.open_view(&app_scope(), &different).unwrap_err().kind,
        ErrorKind::Conflict
    );
    let report = PresentationResult {
        view_id: a.id.clone(),
        descriptor_revision: 1,
        status: PresentationStatus::Failed,
    };
    s.report_presentation(&app_scope(), &report).unwrap();
    assert_eq!(
        s.read_view(&app_scope(), &a.id).unwrap().presentation,
        Some(PresentationStatus::Failed)
    );
    assert_eq!(s.open_view(&app_scope(), &request).unwrap().id, a.id);
    let mut stale = report;
    stale.descriptor_revision = 2;
    assert_eq!(
        s.report_presentation(&app_scope(), &stale)
            .unwrap_err()
            .kind,
        ErrorKind::StaleReference
    );
    let mut scope = app_scope();
    scope.request_id = "other-request".into();
    assert_eq!(
        s.open_view(
            &scope,
            &OpenViewRequest {
                dataset_id: d.id,
                kind: ViewKind::Document
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Unsupported
    );
}

fn fact(value: &str, filed: &str, period: Period) -> Fact {
    Fact {
        company: scope().company,
        namespace: "us-gaap".into(),
        concept: "Assets".into(),
        label: None,
        value: Decimal::new(value).unwrap(),
        unit: "USD".into(),
        period,
        filing_id: format!("filing-{value}-{filed}"),
        form: "10-K".into(),
        filed: filed.parse().unwrap(),
        fiscal_year: Some(2024),
        fiscal_period: Some("FY".into()),
        source_url: "https://fixture.test/facts".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    }
}
fn instant(day: &str) -> Period {
    Period::Instant {
        date: day.parse().unwrap(),
    }
}
fn save_facts(repo: &mut SqliteRepository, facts: Vec<Fact>) -> i64 {
    let run = repo.start_run(&provider(), &scope(), "facts").unwrap();
    repo.save_facts_page(
        run,
        &Page {
            items: facts,
            next_cursor: None,
        },
        &|_| None,
    )
    .unwrap();
    repo.finish_run(run, None).unwrap();
    run
}

#[test]
fn facts_preserve_conflicts_and_failed_run_diagnostics_are_safe() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = save_facts(
        &mut repo,
        vec![
            fact("9007199254740993", "2024-03-01", instant("2023-12-31")),
            fact("9007199254740992", "2024-03-01", instant("2023-12-31")),
        ],
    );
    let mut r = receipt(&repo, run);
    r.provenance.command = FetchCommand::Facts {
        instance_id: "local".into(),
        query: scope(),
    };
    r.provenance.runs[0].kind = RunKind::Financial;
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Facts {
                run_id: run,
                query: metric(),
            },
        )
        .unwrap();
    let p = s
        .read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
    let DatasetRow::Fact { group } = &p.rows[0] else {
        panic!("fact row")
    };
    assert!(group.value.is_none());
    assert!(group.conflict.is_some());
    assert_eq!(group.candidates.len(), 2);
    let failed = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(
        failed,
        &PricePage {
            next_cursor: Some("next".into()),
            ..page(vec![bar("3", "2024-01-02")])
        },
    )
    .unwrap();
    repo.finish_market_run(
        failed,
        Some(&lugus_financial::error::Error::new(
            lugus_financial::error::ErrorKind::Unavailable,
            "SECRET_PROCESS_TEXT",
        )),
    )
    .unwrap();
    let mut r = receipt(&repo, failed);
    r.error = Some(AppError::new(
        ErrorKind::Cancelled,
        "SECRET_PROCESS_TEXT",
        false,
    ));
    let d = freeze(&mut s, &r);
    assert_eq!(
        d.selected_run.as_ref().unwrap().status,
        lugus_financial::storage::RunStatus::Failed
    );
    assert_eq!(d.error.as_ref().unwrap().kind, ErrorKind::Cancelled);
    assert!(
        !serde_json::to_string(&d)
            .unwrap()
            .contains("SECRET_PROCESS_TEXT")
    );
    assert_eq!(
        s.read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap()
        .rows
        .len(),
        1
    );
}
#[test]
fn unauthorized_runs_and_provider_versions_cannot_be_frozen() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "2");
    let other = ingest(&mut repo, "4");
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&receipt(&repo, run)).unwrap();
    assert_eq!(
        s.create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Prices {
                run_id: other,
                query: query(),
                series: PriceSeries::Close
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
    let mut forged = receipt(&repo, run);
    forged.provenance.provider.plugin_version = "different".into();
    let f = s.record_fetch(&forged).unwrap();
    assert!(
        s.create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Prices {
                run_id: run,
                query: query(),
                series: PriceSeries::Close
            }
        )
        .is_err()
    );
}
#[test]
fn page_read_never_decodes_unrequested_rows_and_bounds_metadata_before_decode() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.sqlite");
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(
        run,
        &page(vec![bar("1", "2024-01-02"), bar("2", "2024-01-03")]),
    )
    .unwrap();
    repo.finish_market_run(run, None).unwrap();
    let mut s = store(&db, &fin);
    let d = freeze(&mut s, &receipt(&repo, run));
    let sql = rusqlite::Connection::open(&db).unwrap();
    sql.execute(
        "UPDATE dataset_rows SET payload=?1 WHERE dataset_id=?2 AND ordinal=1",
        rusqlite::params!["é".repeat(600000), d.id],
    )
    .unwrap();
    assert_eq!(
        s.read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap()
        .rows
        .len(),
        1
    );
    assert_eq!(
        s.read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 1,
                limit: 1
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    sql.execute(
        "UPDATE app_records SET payload=?1 WHERE id=?2",
        rusqlite::params!["é".repeat(600000), d.id],
    )
    .unwrap();
    assert_eq!(
        s.dataset_header(&app_scope(), &d.id).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}
#[test]
fn strict_projection_rejects_unknown_nested_fields() {
    for value in [
        serde_json::json!({"kind":"prices","run_id":1,"query":{"instrument":{"namespace":"native","value":"IBM","workspace_id":"evil"},"start":"2024-01-01","end":"2024-02-01","page_size":100},"series":"Close"}),
        serde_json::json!({"kind":"facts","run_id":1,"query":{"scope":scope(),"namespace":"us-gaap","concept":"Assets","unit":"USD","periods":{"kind":"exact","period":{"kind":"instant","date":"2024-01-01","workspace_id":"evil"}}}}),
    ] {
        assert!(serde_json::from_value::<DatasetProjection>(value).is_err());
    }
}
#[test]
fn original_document_verifies_exact_source_retrieval_provider_and_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let doc = Document {
        source_url: "https://fixture.test/original".into(),
        media_type: "text/html".into(),
        content_base64: "aGVsbG8=".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    };
    let checksum = repo.save_document(&provider(), &doc, 100).unwrap();
    let observation = repo.document_observations(&checksum).unwrap().remove(0);
    let mut r = receipt(&repo, 1);
    r.provenance.runs.clear();
    r.provenance.command = FetchCommand::Document {
        instance_id: "local".into(),
        source_url: doc.source_url,
    };
    r.provenance.document = Some(observation.clone());
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(&app_scope(), &f.id, DatasetProjection::Document)
        .unwrap();
    let read = s.read_document(&app_scope(), &d.id, 1, 3).unwrap();
    assert_eq!(read.bytes, b"ell");
    assert_eq!(read.total_bytes, 5);
    assert_eq!(
        s.read_document(&app_scope(), &d.id, 6, 1).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let mut wrong = observation.clone();
    wrong.source_url = "https://fixture.test/other".into();
    assert!(repo.bounded_document(&wrong, 100).is_err());
    wrong = observation.clone();
    wrong.provider.plugin_version = "2".into();
    assert!(repo.bounded_document(&wrong, 100).is_err());
    wrong = observation.clone();
    wrong.retrieved_at += chrono::Duration::seconds(1);
    assert!(repo.bounded_document(&wrong, 100).is_err());
    assert!(matches!(
        repo.bounded_document(&observation, 4).unwrap_err(),
        lugus_financial::storage::bounded::BoundedReadError::LimitExceeded
    ));
    let sql = rusqlite::Connection::open(&fin).unwrap();
    sql.execute(
        "UPDATE document_content SET content=?1 WHERE checksum=?2",
        rusqlite::params![b"other".as_slice(), checksum],
    )
    .unwrap();
    assert_eq!(
        s.read_document(&app_scope(), &d.id, 0, 1).unwrap_err().kind,
        ErrorKind::Storage
    );
}
fn resolution(repo: &mut SqliteRepository, name: &str) -> (i64, i64) {
    use lugus_financial::resolution::{
        Candidate, MatchReason, ResolutionPage, SearchQuery, SearchRequest,
        catalog::{CatalogRepository, ResolutionOutcome},
    };
    let request = SearchRequest {
        query: SearchQuery::Name { text: name.into() },
        page_size: 100,
        cursor: None,
    };
    let run = repo.start_resolution_run(&provider(), &request).unwrap();
    repo.save_resolution_page(
        run,
        &request,
        &ResolutionPage {
            items: vec![Candidate {
                identifier: CompanyId {
                    namespace: "fixture:company".into(),
                    value: name.into(),
                },
                name: name.into(),
                aliases: vec![],
                listings: vec![],
                source_url: "https://fixture.test/company".into(),
                source_checksum: "a".repeat(64),
                retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
                match_reasons: vec![MatchReason::ExactName],
            }],
            next_cursor: None,
            snapshot: "snapshot".into(),
            coverage: "full".into(),
        },
    )
    .unwrap();
    let ResolutionOutcome::Candidates { items, .. } = repo.resolution_outcome(run).unwrap() else {
        panic!("candidates")
    };
    (run, items[0].observation_id)
}
#[test]
fn candidate_selection_requires_frozen_workspace_run_membership() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let (run, observation) = resolution(&mut repo, "Alpha");
    let (_, other) = resolution(&mut repo, "Beta");
    let mut r = receipt(&repo, run);
    r.provenance.runs[0].kind = RunKind::Resolution;
    r.provenance.command = FetchCommand::Resolve {
        instance_id: "local".into(),
        input: "Alpha".into(),
    };
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Resolution { run_id: run },
        )
        .unwrap();
    assert_eq!(d.row_count, 1);
    assert_eq!(
        s.select_candidate(&app_scope(), &d.id, other)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    let selected = s
        .select_candidate(&app_scope(), &d.id, observation)
        .unwrap();
    assert_eq!(selected.entry.run_id, run);
    assert_eq!(selected.entry.candidate.name, "Alpha");
    let mut wrong = app_scope();
    wrong.workspace_id = "other".into();
    assert_eq!(
        s.select_candidate(&wrong, &d.id, observation)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn concurrent_duplicate_view_acceptance_has_one_durable_receipt() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.sqlite");
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "1");
    let mut s = store(&db, &fin);
    let d = freeze(&mut s, &receipt(&repo, run));
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let jobs = (0..2)
        .map(|i| {
            let db = db.clone();
            let fin = fin.clone();
            let d = d.clone();
            let b = barrier.clone();
            std::thread::spawn(move || {
                let mut s = SqliteApplicationStore::open(
                    db,
                    Box::new(SqliteRepository::open(fin).unwrap()),
                    Limits::default(),
                    Box::new(TestClock),
                    Box::new(TestIds(AtomicU64::new(100 + i))),
                )
                .unwrap();
                b.wait();
                s.open_view(
                    &app_scope(),
                    &OpenViewRequest {
                        dataset_id: d.id,
                        kind: ViewKind::PriceChart,
                    },
                )
                .unwrap()
            })
        })
        .collect::<Vec<_>>();
    let receipts = jobs
        .into_iter()
        .map(|j| j.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(receipts[0].id, receipts[1].id);
    let sql = rusqlite::Connection::open(db).unwrap();
    assert_eq!(
        sql.query_row("SELECT count(*) FROM view_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
}
#[test]
fn resolution_conflict_keeps_original_run_candidate_after_later_retrieval() {
    use lugus_financial::resolution::{
        Candidate, Listing, MatchReason, ResolutionPage, SearchQuery, SearchRequest,
        catalog::CatalogRepository,
    };
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let request = SearchRequest {
        query: SearchQuery::Identifier {
            identifier: CompanyId {
                namespace: "sec:ticker".into(),
                value: "IBM".into(),
            },
            exchange: None,
        },
        page_size: 10,
        cursor: None,
    };
    let candidate = Candidate {
        identifier: CompanyId {
            namespace: "sec:cik".into(),
            value: "0000051143".into(),
        },
        name: "Original".into(),
        aliases: vec![],
        listings: vec![Listing {
            ticker: CompanyId {
                namespace: "sec:ticker".into(),
                value: "IBM".into(),
            },
            exchange: None,
        }],
        source_url: "https://fixture.test/company".into(),
        source_checksum: "a".repeat(64),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
        match_reasons: vec![MatchReason::ExactIdentifier],
    };
    let mut runs = vec![];
    for (i, cik) in ["0000051143", "0000000001", "0000051143"]
        .into_iter()
        .enumerate()
    {
        let run = repo.start_resolution_run(&provider(), &request).unwrap();
        runs.push(run);
        let mut c = candidate.clone();
        c.identifier.value = cik.into();
        if i == 2 {
            c.name = "New revision".into();
        }
        repo.save_resolution_page(
            run,
            &request,
            &ResolutionPage {
                items: vec![c],
                next_cursor: None,
                snapshot: "snapshot".into(),
                coverage: "directory".into(),
            },
        )
        .unwrap();
    }
    let mut r = receipt(&repo, runs[0]);
    r.provenance.runs[0].kind = RunKind::Resolution;
    r.provenance.command = FetchCommand::Resolve {
        instance_id: "local".into(),
        input: "IBM".into(),
    };
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Resolution { run_id: runs[0] },
        )
        .unwrap();
    assert_eq!(d.resolution_status.as_deref(), Some("identity_conflict"));
    let page = s
        .read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 10,
            },
        )
        .unwrap();
    assert_eq!(page.rows.len(), 1);
    let DatasetRow::Candidate { entry } = &page.rows[0] else {
        panic!("candidate")
    };
    assert_eq!(entry.candidate.name, "Original");
    assert_eq!(entry.run_id, runs[0]);
}
#[test]
fn projection_and_page_limits_are_enforced_without_truncation() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = repo.start_market_run(&provider(), &query()).unwrap();
    repo.save_prices_page(
        run,
        &page(vec![bar("1", "2024-01-02"), bar("2", "2024-01-03")]),
    )
    .unwrap();
    repo.finish_market_run(run, None).unwrap();
    let limits = Limits {
        max_items_per_fetch: 1,
        max_read_page_items: 1,
        ..Limits::default()
    };
    let mut s = SqliteApplicationStore::open(
        dir.path().join("app.sqlite"),
        Box::new(SqliteRepository::open(&fin).unwrap()),
        limits,
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    let f = s.record_fetch(&receipt(&repo, run)).unwrap();
    assert_eq!(
        s.create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Prices {
                run_id: run,
                query: query(),
                series: PriceSeries::Close
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    let projection = DatasetProjection::Facts {
        run_id: run,
        query: MetricQuery {
            periods: PeriodSelection::Exact {
                period: instant("2023-12-31"),
            },
            ..metric()
        },
    };
    assert!(
        serde_json::from_str::<DatasetProjection>(&serde_json::to_string(&projection).unwrap())
            .is_ok()
    );
}
#[test]
fn schema_identity_prevents_reusing_financial_database() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let repo = SqliteRepository::open(&fin).unwrap();
    let result = SqliteApplicationStore::open(
        &fin,
        Box::new(repo),
        Limits::default(),
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    );
    assert!(matches!(
        result,
        Err(AppError {
            kind: ErrorKind::Storage,
            ..
        })
    ));
}
#[test]
fn adjusted_price_gaps_remain_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "4");
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&receipt(&repo, run)).unwrap();
    let d = s
        .create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Prices {
                run_id: run,
                query: query(),
                series: PriceSeries::AdjustedClose,
            },
        )
        .unwrap();
    let page = s
        .read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
    assert!(matches!(
        page.rows[0],
        DatasetRow::Price { value: None, .. }
    ));
    assert!(page.header.conflicts.is_empty());
}
#[test]
fn filings_preserve_exact_query_run_and_retrieval() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = repo.start_run(&provider(), &scope(), "filings").unwrap();
    let filing = Filing {
        company: scope().company,
        filing_id: "filing-1".into(),
        form: "10-K".into(),
        filed: "2024-03-01".parse().unwrap(),
        report_date: None,
        accepted_at: None,
        primary_document: Some("https://fixture.test/original".into()),
        source_url: "https://fixture.test/filing".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    };
    repo.save_filings_page(
        run,
        &Page {
            items: vec![filing],
            next_cursor: None,
        },
    )
    .unwrap();
    repo.finish_run(run, None).unwrap();
    let mut r = receipt(&repo, run);
    r.provenance.command = FetchCommand::Filings {
        instance_id: "local".into(),
        query: scope(),
    };
    r.provenance.runs[0].kind = RunKind::Financial;
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(
            &app_scope(),
            &f.id,
            DatasetProjection::Filings { run_id: run },
        )
        .unwrap();
    assert_eq!(d.query["company"]["value"], "51143");
    assert_eq!(d.selected_run.as_ref().unwrap().run_id, run);
    let page = s
        .read_dataset(
            &app_scope(),
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
    let DatasetRow::Filing { evidence } = &page.rows[0] else {
        panic!("filing")
    };
    assert_eq!(evidence.value.filing_id, "filing-1");
    assert_eq!(
        evidence.retrieval.retrieved_at.to_rfc3339(),
        "2024-12-31T00:00:00+00:00"
    );
}

#[test]
fn fallback_run_receipts_survive_a_single_page_budget_and_reopen() {
    use lugus_financial::resolution::{
        ResolutionPage, SearchQuery, SearchRequest, catalog::CatalogRepository,
    };
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let app = dir.path().join("app.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let primary_query = SearchRequest {
        query: SearchQuery::Identifier {
            identifier: CompanyId {
                namespace: "sec:ticker".into(),
                value: "IBM".into(),
            },
            exchange: None,
        },
        page_size: 100,
        cursor: None,
    };
    let primary = repo
        .start_resolution_run(&provider(), &primary_query)
        .unwrap();
    repo.save_resolution_page(
        primary,
        &primary_query,
        &ResolutionPage {
            items: vec![],
            next_cursor: None,
            snapshot: "snapshot".into(),
            coverage: "directory".into(),
        },
    )
    .unwrap();
    let fallback = repo
        .start_resolution_run(
            &provider(),
            &SearchRequest {
                query: SearchQuery::Name { text: "IBM".into() },
                ..primary_query
            },
        )
        .unwrap();
    repo.fail_resolution_run(fallback, "provider page limit reached")
        .unwrap();
    let mut result = receipt(&repo, primary);
    result.provenance.command = FetchCommand::Resolve {
        instance_id: "local".into(),
        input: "IBM".into(),
    };
    result.provenance.runs = vec![
        RunReceipt {
            kind: RunKind::Resolution,
            id: primary,
        },
        RunReceipt {
            kind: RunKind::Resolution,
            id: fallback,
        },
    ];
    result.error = Some(AppError::new(
        ErrorKind::ResourceLimit,
        "provider page limit reached",
        false,
    ));
    let mut s = SqliteApplicationStore::open(
        &app,
        Box::new(SqliteRepository::open(&fin).unwrap()),
        Limits {
            max_pages_per_fetch: 1,
            ..Limits::default()
        },
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    let reference = s.record_fetch(&result).unwrap();
    drop(s);
    let s = store(&app, &fin);
    let reopened = s.read_fetch(&app_scope(), &reference.id).unwrap();
    assert_eq!(reopened.runs, result.provenance.runs);
    assert_eq!(reopened.error.unwrap().kind, ErrorKind::ResourceLimit);
}

#[test]
fn receipt_run_bounds_follow_operations_and_keep_positive_ids() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let repo = SqliteRepository::open(&fin).unwrap();
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let mut r = receipt(&repo, 1);
    r.provenance.runs.push(RunReceipt {
        kind: RunKind::Market,
        id: 2,
    });
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    r.provenance.command = FetchCommand::Resolve {
        instance_id: "local".into(),
        input: "IBM".into(),
    };
    r.provenance.runs = (1..=3)
        .map(|id| RunReceipt {
            kind: RunKind::Resolution,
            id,
        })
        .collect();
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    r.provenance.runs.truncate(2);
    r.provenance.command = FetchCommand::Lookup {
        instance_id: "local".into(),
        request: LookupRequest {
            identifier: scope().company,
        },
    };
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    r.provenance.runs.truncate(1);
    r.provenance.runs[0].id = 0;
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    r.provenance.runs[0].id = 1;
    r.provenance.runs[0].kind = RunKind::Market;
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    r.provenance.command = FetchCommand::Document {
        instance_id: "local".into(),
        source_url: "https://fixture.test/document".into(),
    };
    assert_eq!(
        s.record_fetch(&r).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let mut tiny = SqliteApplicationStore::open(
        dir.path().join("tiny.sqlite"),
        Box::new(SqliteRepository::open(&fin).unwrap()),
        Limits {
            max_input_bytes: 1,
            ..Limits::default()
        },
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    assert_eq!(
        tiny.record_fetch(&receipt(&repo, 1)).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn zero_row_header_obeys_total_creation_bytes_without_persisting_a_dataset() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let app = dir.path().join("app.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let doc = Document {
        source_url: format!("https://fixture.test/{}", "x".repeat(900)),
        media_type: "text/html".into(),
        content_base64: "aGVsbG8=".into(),
        retrieved_at: "2024-12-31T00:00:00Z".parse().unwrap(),
    };
    let checksum = repo.save_document(&provider(), &doc, 1000).unwrap();
    let mut r = receipt(&repo, 1);
    r.provenance.runs.clear();
    r.provenance.command = FetchCommand::Document {
        instance_id: "local".into(),
        source_url: doc.source_url,
    };
    r.provenance.document = Some(repo.document_observations(&checksum).unwrap().remove(0));
    let mut s = SqliteApplicationStore::open(
        &app,
        Box::new(SqliteRepository::open(&fin).unwrap()),
        Limits {
            max_bytes_per_fetch: 1000,
            max_document_bytes: 1000,
            ..Limits::default()
        },
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    let f = s.record_fetch(&r).unwrap();
    assert_eq!(
        s.create_dataset(&app_scope(), &f.id, DatasetProjection::Document)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    let sql = rusqlite::Connection::open(&app).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM app_records WHERE category='dataset'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM dataset_rows", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    assert!(s.read_fetch(&app_scope(), &f.id).is_ok());
}

fn all_facts_projection(run_id: i64) -> DatasetProjection {
    serde_json::from_value(serde_json::json!({"kind":"all_facts","run_id":run_id})).unwrap()
}

#[test]
fn all_facts_projection_is_strict() {
    let projection = all_facts_projection(1);
    assert_eq!(
        serde_json::to_value(projection).unwrap(),
        serde_json::json!({"kind":"all_facts","run_id":1})
    );
    for extra in ["query", "namespace", "workspace_id"] {
        let mut value = serde_json::json!({"kind":"all_facts","run_id":1});
        value[extra] = serde_json::json!({});
        assert!(serde_json::from_value::<DatasetProjection>(value).is_err());
    }
}

#[test]
fn all_facts_freezes_every_original_disclosure_offline_with_partial_run_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let db = dir.path().join("app.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let facts = (0..1003)
        .map(|i| {
            let mut f = fact("9007199254740993.000", "2024-03-01", instant("2023-12-31"));
            f.filing_id = format!("disclosure-{i}");
            f.namespace = ["us-gaap", "dei", "custom"][i % 3].into();
            f.unit = ["USD", "shares", "USD/shares"][i % 3].into();
            f.fiscal_year = None;
            f.fiscal_period = None;
            if i % 2 == 0 {
                f.period = Period::Duration {
                    start: "2023-01-01".parse().unwrap(),
                    end: "2023-12-31".parse().unwrap(),
                };
            }
            f
        })
        .collect::<Vec<_>>();
    let run = repo.start_run(&provider(), &scope(), "both").unwrap();
    for items in facts.chunks(100) {
        repo.save_facts_page(
            run,
            &Page {
                items: items.to_vec(),
                next_cursor: Some("remaining".into()),
            },
            &|_| None,
        )
        .unwrap();
    }
    repo.finish_run(
        run,
        Some(&lugus_financial::error::Error::new(
            lugus_financial::error::ErrorKind::Unavailable,
            "SECRET",
        )),
    )
    .unwrap();
    let mut r = receipt(&repo, run);
    r.provenance.command = FetchCommand::Facts {
        instance_id: "local".into(),
        query: scope(),
    };
    r.provenance.runs[0].kind = RunKind::Financial;
    let mut s = store(&db, &fin);
    let f = s.record_fetch(&r).unwrap();
    let d = s
        .create_dataset(&app_scope(), &f.id, all_facts_projection(run))
        .unwrap();
    assert_eq!(d.row_count, 1003);
    assert_eq!(d.kind, DatasetKind::Facts);
    assert_eq!(d.policy.as_deref(), Some("all-reported-facts-v1"));
    assert_eq!(d.query, serde_json::to_value(scope()).unwrap());
    assert_eq!(
        d.selected_run.as_ref().unwrap().status,
        lugus_financial::storage::RunStatus::Failed
    );
    assert!(d.limitations.iter().any(|s| s.contains("partial")));
    assert!(!serde_json::to_string(&d).unwrap().contains("SECRET"));
    let other = save_facts(
        &mut repo,
        vec![fact("1", "2024-03-01", instant("2023-12-31"))],
    );
    assert_eq!(
        s.create_dataset(&app_scope(), &f.id, all_facts_projection(other))
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    let wrong = repo.start_run(&provider(), &scope(), "filings").unwrap();
    repo.finish_run(wrong, None).unwrap();
    r.provenance.runs[0].id = wrong;
    r.provenance.command = FetchCommand::Filings {
        instance_id: "local".into(),
        query: scope(),
    };
    let wrong_fetch = s.record_fetch(&r).unwrap();
    assert_eq!(
        s.create_dataset(&app_scope(), &wrong_fetch.id, all_facts_projection(wrong))
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
    drop(s);
    let s = store(&db, &fin);
    let mut offset = 0;
    let mut actual = Vec::new();
    loop {
        let page = s
            .read_dataset(&app_scope(), &d.id, PageRequest { offset, limit: 100 })
            .unwrap();
        for row in page.rows {
            let value = serde_json::to_value(row).unwrap();
            assert_eq!(value["kind"], "reported_fact");
            assert!(
                value["evidence"]["retrieval"]["observation_id"]
                    .as_i64()
                    .unwrap()
                    > 0
            );
            assert_eq!(
                value["evidence"]["retrieval"]["retrieved_at"],
                "2024-12-31T00:00:00Z"
            );
            assert!(
                !value["evidence"]["retrieval"]["fingerprint"]
                    .as_str()
                    .unwrap()
                    .is_empty()
            );
            let fact = value["evidence"]["value"].clone();
            assert_eq!(fact["value"], "9007199254740993.000");
            assert!(fact["label"].is_null());
            assert!(fact["fiscal_year"].is_null());
            assert!(fact["fiscal_period"].is_null());
            actual.push(fact);
        }
        match page.next_offset {
            Some(next) => offset = next,
            None => break,
        }
    }
    let mut expected = facts
        .into_iter()
        .map(|f| serde_json::to_value(f).unwrap())
        .collect::<Vec<_>>();
    actual.sort_by_key(|f| f["filing_id"].as_str().unwrap().to_owned());
    expected.sort_by_key(|f| f["filing_id"].as_str().unwrap().to_owned());
    assert_eq!(actual, expected);
    let mut foreign = app_scope();
    foreign.workspace_id = "foreign".into();
    assert_eq!(
        s.read_dataset(
            &foreign,
            &d.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
}

#[test]
fn all_facts_marks_remaining_source_pages_even_when_run_is_complete() {
    let dir = tempfile::tempdir().unwrap();
    let fin = dir.path().join("financial.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = repo.start_run(&provider(), &scope(), "facts").unwrap();
    repo.save_facts_page(
        run,
        &Page {
            items: vec![fact("0", "2024-03-01", instant("2023-12-31"))],
            next_cursor: Some("next".into()),
        },
        &|_| None,
    )
    .unwrap();
    let mut r = receipt(&repo, run);
    r.provenance.command = FetchCommand::Facts {
        instance_id: "local".into(),
        query: scope(),
    };
    r.provenance.runs[0].kind = RunKind::Financial;
    let mut s = store(&dir.path().join("app.sqlite"), &fin);
    let f = s.record_fetch(&r).unwrap();
    assert_eq!(
        s.create_dataset(&app_scope(), &f.id, all_facts_projection(run))
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    repo.finish_run(run, None).unwrap();
    let d = s
        .create_dataset(&app_scope(), &f.id, all_facts_projection(run))
        .unwrap();
    assert_eq!(
        d.selected_run.unwrap().status,
        lugus_financial::storage::RunStatus::Complete
    );
    assert!(d.limitations.iter().any(|s| s.contains("partial")));
}
