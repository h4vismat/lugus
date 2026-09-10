use lugus_app::*;
use lugus_financial::{
    domain::*,
    market_data::*,
    selection::PriceSeries,
    storage::{SqliteRepository, market::MarketRepository},
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
fn reference_store(app: &Path, financial: &Path) -> SqliteApplicationStore {
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
use lugus_app::conversations::*;
fn managed_dataset(
    s: &mut SqliteApplicationStore,
    repo: &SqliteRepository,
    run: i64,
    c: &Conversation,
    request: &str,
) -> DatasetHeader {
    let mut receipt = receipt(repo, run);
    receipt.provenance.scope.workspace_id = c.workspace_id.clone();
    receipt.provenance.scope.request_id = request.into();
    let scope = receipt.provenance.scope.clone();
    let fetch = s.record_fetch(&receipt).unwrap();
    s.create_dataset(
        &scope,
        &fetch.id,
        DatasetProjection::Prices {
            run_id: run,
            query: query(),
            series: PriceSeries::Close,
        },
    )
    .unwrap()
}
fn second_company_dataset(
    s: &mut SqliteApplicationStore,
    repo: &mut SqliteRepository,
    c: &Conversation,
) -> DatasetHeader {
    let mut q = query();
    q.instrument.value = "AAPL".into();
    let mut b = bar("456", "2024-01-02");
    b.instrument = q.instrument.clone();
    let run = repo.start_market_run(&provider(), &q).unwrap();
    repo.save_prices_page(run, &page(vec![b])).unwrap();
    repo.finish_market_run(run, None).unwrap();
    let mut fetched = receipt(repo, run);
    fetched.provenance.scope = managed_scope(c, "apple-fetch");
    fetched.provenance.command = FetchCommand::Prices {
        instance_id: "local".into(),
        query: q.clone(),
    };
    let fetch = s.record_fetch(&fetched).unwrap();
    s.create_dataset(
        &managed_scope(c, "apple-data"),
        &fetch.id,
        DatasetProjection::Prices {
            run_id: run,
            query: q,
            series: PriceSeries::Close,
        },
    )
    .unwrap()
}
fn managed_scope(c: &Conversation, request: &str) -> Scope {
    Scope {
        workspace_id: c.workspace_id.clone(),
        request_id: request.into(),
        run_id: None,
    }
}
#[test]
fn views_attach_atomically_preserve_selection_and_closed_retry_stays_closed() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "123");
    let mut s = reference_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let d = managed_dataset(&mut s, &repo, run, &c, "fetch");
    let request = OpenViewRequest {
        dataset_id: d.id,
        kind: ViewKind::PriceChart,
    };
    let v1 = s.open_view(&managed_scope(&c, "view-1"), &request).unwrap();
    let apple = second_company_dataset(&mut s, &mut repo, &c);
    let v2 = s
        .open_view(
            &managed_scope(&c, "view-2"),
            &OpenViewRequest {
                dataset_id: apple.id.clone(),
                kind: ViewKind::PriceChart,
            },
        )
        .unwrap();
    assert_ne!(v1.dataset_id, v2.dataset_id);
    let apple_page = s
        .read_dataset(
            &managed_scope(&c, "read-apple"),
            &apple.id,
            PageRequest {
                offset: 0,
                limit: 1,
            },
        )
        .unwrap();
    let DatasetRow::Price { evidence, .. } = &apple_page.rows[0] else {
        panic!("price evidence")
    };
    assert_eq!(evidence.value.instrument.value, "AAPL");
    let state = s.workspace(&c.id).unwrap();
    assert_eq!(state.view_ids, vec![v1.id.clone(), v2.id.clone()]);
    assert_eq!(state.selected_view_id, Some(v1.id.clone()));
    assert_eq!(state.revision, 2);
    assert_eq!(
        s.mutate_workspace(
            &c.id,
            1,
            &WorkspaceMutation::Select {
                view_id: v2.id.clone()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    let closed = s
        .mutate_workspace(
            &c.id,
            2,
            &WorkspaceMutation::Close {
                view_id: v1.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(closed.selected_view_id, Some(v2.id.clone()));
    assert_eq!(
        s.open_view(&managed_scope(&c, "view-1"), &request)
            .unwrap()
            .id,
        v1.id
    );
    assert_eq!(s.workspace(&c.id).unwrap(), closed);
    assert!(s.read_view(&managed_scope(&c, "history"), &v1.id).is_ok());
    drop(s);
    let s = reference_store(&path, &fin);
    assert_eq!(s.workspace(&c.id).unwrap(), closed);
}
#[test]
fn full_layout_rolls_back_new_view_acceptance() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "123");
    let mut s = SqliteApplicationStore::open_with_conversation_limits(
        &path,
        Box::new(SqliteRepository::open(&fin).unwrap()),
        Limits::default(),
        ConversationLimits {
            open_views: 1,
            ..ConversationLimits::default()
        },
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    let c = s.create_conversation("create", "Research").unwrap();
    let d = managed_dataset(&mut s, &repo, run, &c, "fetch");
    let request = OpenViewRequest {
        dataset_id: d.id,
        kind: ViewKind::PriceChart,
    };
    s.open_view(&managed_scope(&c, "first"), &request).unwrap();
    assert_eq!(
        s.open_view(&managed_scope(&c, "second"), &request)
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    assert_eq!(
        sql.query_row(
            "SELECT count(*) FROM app_records WHERE category='view'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        1
    );
    assert_eq!(
        sql.query_row("SELECT count(*) FROM view_requests", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(s.workspace(&c.id).unwrap().revision, 1);
}
#[test]
fn selected_evidence_is_scoped_frozen_and_retry_does_not_reread_mutable_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "123");
    let mut s = reference_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let other = s.create_conversation("other", "Other").unwrap();
    let d = managed_dataset(&mut s, &repo, run, &c, "fetch");
    let view = s
        .open_view(
            &managed_scope(&c, "view"),
            &OpenViewRequest {
                dataset_id: d.id.clone(),
                kind: ViewKind::PriceChart,
            },
        )
        .unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let request = SendMessageRequest {
        conversation_id: c.id.clone(),
        request_id: "send".into(),
        text: "Compare prices".into(),
        selected: vec![
            SelectedReference::Dataset { id: d.id.clone() },
            SelectedReference::View {
                id: view.id.clone(),
            },
        ],
    };
    assert_eq!(
        s.admit(
            &epoch,
            &SendMessageRequest {
                conversation_id: other.id,
                ..request.clone()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ScopeMismatch
    );
    let run = s.admit(&epoch, &request).unwrap();
    assert!(run.input.references[0].serialized.contains("123"));
    let input = run.input.clone();
    s.report_presentation(
        &managed_scope(&c, "render"),
        &PresentationResult {
            view_id: view.id.clone(),
            descriptor_revision: 1,
            status: PresentationStatus::Failed,
        },
    )
    .unwrap();
    s.mutate_workspace(&c.id, 1, &WorkspaceMutation::Close { view_id: view.id })
        .unwrap();
    ingest(&mut repo, "999");
    assert_eq!(s.admit(&epoch, &request).unwrap().input, input);
    assert_eq!(s.run(&c.id, &run.id).unwrap().input, input);
}
#[test]
fn selected_payload_limits_preflight_before_decoding_and_admission() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut repo = SqliteRepository::open(&fin).unwrap();
    let run = ingest(&mut repo, "123");
    let mut s = SqliteApplicationStore::open_with_conversation_limits(
        &path,
        Box::new(SqliteRepository::open(&fin).unwrap()),
        Limits::default(),
        ConversationLimits {
            selected_bytes: 1024,
            ..ConversationLimits::default()
        },
        Box::new(TestClock),
        Box::new(TestIds(AtomicU64::new(1))),
    )
    .unwrap();
    let c = s.create_conversation("create", "Research").unwrap();
    let d = managed_dataset(&mut s, &repo, run, &c, "fetch");
    let view = s
        .open_view(
            &managed_scope(&c, "view"),
            &OpenViewRequest {
                dataset_id: d.id.clone(),
                kind: ViewKind::PriceChart,
            },
        )
        .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE app_records SET payload=?1 WHERE id=?2",
        rusqlite::params!["x".repeat(2048), view.id],
    )
    .unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let req = SendMessageRequest {
        conversation_id: c.id.clone(),
        request_id: "send".into(),
        text: "Compare".into(),
        selected: vec![SelectedReference::View { id: view.id }],
    };
    assert_eq!(
        s.admit(&epoch, &req).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert!(
        s.messages(
            &c.id,
            PageRequest {
                offset: 0,
                limit: 1
            }
        )
        .unwrap()
        .items
        .is_empty()
    );
}
