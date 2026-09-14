use lugus_app::*;
use lugus_financial::{
    domain::CompanyId,
    instruments::*,
    resolution::{catalog::*, *},
    storage::SqliteRepository,
};
fn scope(request: &str) -> Scope {
    Scope {
        workspace_id: "workspace".into(),
        request_id: request.into(),
        run_id: Some("agent-run".into()),
    }
}
fn provider() -> ProviderIdentity {
    ProviderIdentity {
        instance_id: "fixture".into(),
        plugin_id: "fixture".into(),
        plugin_version: "1".into(),
    }
}
fn id(ns: &str, value: &str) -> CompanyId {
    CompanyId {
        namespace: ns.into(),
        value: value.into(),
    }
}
struct Fixture {
    _root: tempfile::TempDir,
    app: std::path::PathBuf,
    financial: std::path::PathBuf,
    store: SqliteApplicationStore,
    request: BindRequest,
}
impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let app = root.path().join("app.sqlite");
        let financial = root.path().join("financial.sqlite");
        let mut repo = SqliteRepository::open(&financial).unwrap();
        let listing = Listing {
            ticker: id("sec:ticker", "AAPL"),
            exchange: Some(id("sec:exchange", "NASDAQ")),
        };
        let candidate = Candidate {
            identifier: id("sec:cik", "0000320193"),
            name: "Apple Inc.".into(),
            aliases: vec![],
            listings: vec![listing.clone()],
            source_url: "https://sec.test/apple".into(),
            source_checksum: "a".repeat(64),
            retrieved_at: "2026-09-10T00:00:00Z".parse().unwrap(),
            match_reasons: vec![MatchReason::NameSubstring],
        };
        let request = SearchRequest {
            query: SearchQuery::Name {
                text: "Apple".into(),
            },
            page_size: 10,
            cursor: None,
        };
        let run = repo.start_resolution_run(&provider(), &request).unwrap();
        repo.save_resolution_page(
            run,
            &request,
            &ResolutionPage {
                items: vec![candidate.clone()],
                next_cursor: None,
                snapshot: "snapshot".into(),
                coverage: "fixture".into(),
            },
        )
        .unwrap();
        let ResolutionOutcome::Candidates { items, .. } = repo.resolution_outcome(run).unwrap()
        else {
            panic!()
        };
        let observation_id = items[0].observation_id;
        let query = InstrumentLookup {
            instrument: lugus_financial::market_data::InstrumentId {
                namespace: "yahoo:symbol".into(),
                value: "AAPL".into(),
            },
        };
        let metadata = InstrumentMetadata {
            instrument: query.instrument.clone(),
            issuer_name: Some("APPLE INC".into()),
            ticker: Some("AAPL".into()),
            exchange: Some(id("yahoo:exchange", "NMS")),
            kind: Some(InstrumentKind::Equity),
            issuer_identifiers: vec![],
            source_url: "https://yahoo.test/apple".into(),
            source_checksum: "b".repeat(64),
            retrieved_at: candidate.retrieved_at,
        };
        let instrument = repo
            .save_instrument_observation(&provider(), &query, &metadata)
            .unwrap();
        let mut store = open(&app, &financial);
        let mut receipt = FetchResult {
            provenance: FetchProvenance {
                scope: scope("resolve"),
                provider: provider(),
                repository_id: repo.repository_identity().unwrap(),
                command: FetchCommand::Resolve {
                    instance_id: "fixture".into(),
                    input: "Apple".into(),
                },
                runs: vec![RunReceipt {
                    kind: RunKind::Resolution,
                    id: run,
                }],
                document: None,
                instrument_observation: None,
                binding_id: None,
            },
            error: None,
        };
        let f = store.record_fetch(&receipt).unwrap();
        let d = store
            .create_dataset(
                &scope("freeze"),
                &f.id,
                DatasetProjection::Resolution { run_id: run },
            )
            .unwrap();
        receipt.provenance.command = FetchCommand::InstrumentLookup {
            instance_id: "fixture".into(),
            query,
        };
        receipt.provenance.runs.clear();
        receipt.provenance.instrument_observation = Some(instrument);
        let f = store.record_fetch(&receipt).unwrap();
        Self {
            _root: root,
            app,
            financial,
            store,
            request: BindRequest {
                company_dataset_id: d.id,
                company_observation_id: observation_id,
                listing,
                instrument_fetch_id: f.id,
                supersedes: None,
            },
        }
    }
}
fn open(app: &std::path::Path, financial: &std::path::Path) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        app,
        Box::new(SqliteRepository::open(financial).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap()
}
#[test]
fn binding_deduplication_scope_reopen_and_immutable_history() {
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    assert_eq!(b.instrument.metadata.ticker.as_deref(), Some("AAPL"));
    assert_eq!(f.store.bind(&scope("bind"), &f.request).unwrap().id, b.id);
    let mut changed = f.request.clone();
    changed.supersedes = Some(b.id.clone());
    assert_eq!(
        f.store.bind(&scope("bind"), &changed).unwrap_err().kind,
        ErrorKind::Conflict
    );
    let other = Scope {
        workspace_id: "other".into(),
        ..scope("read")
    };
    assert_eq!(
        f.store.read_binding(&other, &b.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    let prepared = f.store.prepare_binding(&scope("prepare"), &b.id).unwrap();
    let next = f.store.bind(&scope("replace"), &changed).unwrap();
    assert!(
        matches!(f.store.read_binding(&scope("read"),&b.id).unwrap().status,BindingStatus::Superseded{binding_id} if binding_id==next.id)
    );
    assert_eq!(prepared.id, b.id);
    assert!(f.store.prepare_binding(&scope("again"), &b.id).is_err());
    let r = RevokeBindingRequest {
        binding_id: next.id.clone(),
    };
    f.store.revoke_binding(&scope("revoke"), &r).unwrap();
    f.store.revoke_binding(&scope("revoke"), &r).unwrap();
    assert!(f.store.prepare_binding(&scope("after"), &next.id).is_err());
    drop(f.store);
    let store = open(&f.app, &f.financial);
    assert_eq!(
        store.read_binding(&scope("read"), &b.id).unwrap().record.id,
        b.id
    );
    assert_eq!(
        store
            .list_bindings(
                &scope("list"),
                PageRequest {
                    offset: 0,
                    limit: 1
                }
            )
            .unwrap()
            .next_offset,
        Some(1)
    );
    assert_eq!(
        store
            .binding_history(
                &scope("history"),
                &b.id,
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .events
            .len(),
        2
    );
}
#[test]
fn binding_rejects_foreign_or_incomplete_evidence_before_mutation() {
    let mut f = Fixture::new();
    let mut request = f.request.clone();
    request.company_observation_id += 99;
    assert!(f.store.bind(&scope("bad-observation"), &request).is_err());
    request = f.request.clone();
    request.listing.ticker.value = "MSFT".into();
    assert_eq!(
        f.store
            .bind(&scope("bad-listing"), &request)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let other = Scope {
        workspace_id: "other".into(),
        ..scope("bad-workspace")
    };
    assert_eq!(
        f.store.bind(&other, &f.request).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    let conn = rusqlite::Connection::open(&f.app).unwrap();
    for status in ["identity_conflict", "incomplete"] {
        conn.execute(
            "UPDATE app_records SET payload=json_set(payload,'$.resolution_status',?1) WHERE id=?2",
            rusqlite::params![status, f.request.company_dataset_id],
        )
        .unwrap();
        assert!(f.store.bind(&scope(status), &f.request).is_err());
    }
    assert!(
        f.store
            .list_bindings(
                &scope("list"),
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .bindings
            .is_empty()
    );
}
#[test]
fn fetch_registration_validates_exact_lookup_receipt_and_binding_price_association() {
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    let original = f
        .store
        .read_fetch(&scope("read"), &f.request.instrument_fetch_id)
        .unwrap();
    let mut result = FetchResult {
        provenance: FetchProvenance {
            scope: scope("fetch"),
            provider: original.provider,
            repository_id: original.repository_id,
            command: original.command,
            runs: vec![],
            document: None,
            instrument_observation: original.instrument_observation,
            binding_id: None,
        },
        error: None,
    };
    result
        .provenance
        .instrument_observation
        .as_mut()
        .unwrap()
        .metadata
        .issuer_name = Some("Forged".into());
    assert_eq!(
        f.store.record_fetch(&result).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    result.provenance.instrument_observation = None;
    result.provenance.command = FetchCommand::Prices {
        instance_id: "fixture".into(),
        query: PriceQuery {
            instrument: b.instrument.request.instrument.clone(),
            start: "2024-01-01".parse().unwrap(),
            end: "2024-02-01".parse().unwrap(),
            page_size: 10,
            cursor: None,
        },
    };
    result.provenance.binding_id = Some(b.id.clone());
    f.store
        .revoke_binding(
            &scope("revoke"),
            &RevokeBindingRequest {
                binding_id: b.id.clone(),
            },
        )
        .unwrap();
    assert_eq!(
        f.store.record_fetch(&result).unwrap().binding_id,
        Some(b.id)
    );
    if let FetchCommand::Prices { query, .. } = &mut result.provenance.command {
        query.instrument.value = "MSFT".into();
    }
    assert_eq!(
        f.store.record_fetch(&result).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn concurrent_supersession_and_revocation_have_one_winner() {
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    let mut request = f.request.clone();
    request.supersedes = Some(b.id.clone());
    let mut other = open(&f.app, &f.financial);
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other_gate = gate.clone();
    let id = b.id.clone();
    let task = std::thread::spawn(move || {
        other_gate.wait();
        other.revoke_binding(&scope("revoke"), &RevokeBindingRequest { binding_id: id })
    });
    gate.wait();
    let replace = f.store.bind(&scope("replace"), &request);
    let revoke = task.join().unwrap();
    assert_ne!(replace.is_ok(), revoke.is_ok());
    assert_eq!(
        f.store
            .binding_history(
                &scope("history"),
                &b.id,
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .events
            .len(),
        2
    );
}
#[test]
fn bounded_pages_preflight_utf8_and_old_records_default_to_unbound() {
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    let limits = Limits {
        max_read_page_bytes: 100,
        ..Limits::default()
    };
    let small = SqliteApplicationStore::open(
        &f.app,
        Box::new(SqliteRepository::open(&f.financial).unwrap()),
        limits,
        Box::new(SystemClock),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    assert_eq!(
        small
            .list_bindings(
                &scope("list"),
                PageRequest {
                    offset: 0,
                    limit: 1
                }
            )
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        small
            .binding_history(
                &scope("history"),
                &b.id,
                PageRequest {
                    offset: 0,
                    limit: 1
                }
            )
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    let fetch = f
        .store
        .read_fetch(&scope("old"), &f.request.instrument_fetch_id)
        .unwrap();
    let mut old = serde_json::to_value(fetch).unwrap();
    old.as_object_mut()
        .unwrap()
        .remove("instrument_observation");
    old.as_object_mut().unwrap().remove("binding_id");
    let restored: FetchReference = serde_json::from_value(old).unwrap();
    assert!(restored.instrument_observation.is_none());
    assert!(restored.binding_id.is_none());
    let conn = rusqlite::Connection::open(&f.app).unwrap();
    assert!(
        conn.execute("DELETE FROM app_records WHERE id=?1", [b.id])
            .is_err()
    );
    assert!(conn.execute("DELETE FROM binding_history", []).is_err());
}
#[test]
fn binding_inputs_reject_nested_unknown_authority() {
    let f = Fixture::new();
    let mut value = serde_json::to_value(&f.request).unwrap();
    value["listing"]["ticker"]["verified"] = serde_json::json!(true);
    assert!(serde_json::from_value::<BindRequest>(value).is_err());
}
#[test]
fn concurrent_identical_binding_requests_share_one_record() {
    let mut f = Fixture::new();
    let mut other = open(&f.app, &f.financial);
    let request = f.request.clone();
    let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other_gate = gate.clone();
    let task = std::thread::spawn(move || {
        other_gate.wait();
        other.bind(&scope("same"), &request).unwrap()
    });
    gate.wait();
    let first = f.store.bind(&scope("same"), &f.request).unwrap();
    assert_eq!(first.id, task.join().unwrap().id);
    assert_eq!(
        f.store
            .list_bindings(
                &scope("list"),
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .bindings
            .len(),
        1
    );
}
#[test]
fn application_v1_migration_preserves_legacy_payloads_and_rejects_other_repositories() {
    let root = tempfile::tempdir().unwrap();
    let app = root.path().join("app.sqlite");
    let financial = root.path().join("financial.sqlite");
    let conn = rusqlite::Connection::open(&app).unwrap();
    conn.execute_batch("CREATE TABLE app_records(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,category TEXT NOT NULL,payload TEXT NOT NULL);CREATE TABLE dataset_rows(dataset_id TEXT NOT NULL REFERENCES app_records(id),ordinal INTEGER NOT NULL,payload TEXT NOT NULL,observation_id INTEGER,PRIMARY KEY(dataset_id,ordinal));CREATE TABLE view_requests(workspace TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,view_id TEXT NOT NULL REFERENCES app_records(id),PRIMARY KEY(workspace,request));PRAGMA application_id=1280657235;PRAGMA user_version=1;INSERT INTO app_records VALUES('legacy','w','r','fetch','untouched');").unwrap();
    let store = open(&app, &financial);
    assert!(
        store
            .list_bindings(
                &scope("list"),
                PageRequest {
                    offset: 0,
                    limit: 10
                }
            )
            .unwrap()
            .bindings
            .is_empty()
    );
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        7
    );
    assert_eq!(
        conn.query_row(
            "SELECT payload FROM app_records WHERE id='legacy'",
            [],
            |r| r.get::<_, String>(0)
        )
        .unwrap(),
        "untouched"
    );
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    let other = open(&f.app, &financial);
    assert_eq!(
        other.read_binding(&scope("read"), &b.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn prepared_price_dataset_keeps_binding_after_revocation() {
    use lugus_financial::market_data::{Completeness, PriceCoverage, PricePage};
    use lugus_financial::storage::market::MarketRepository;
    let mut f = Fixture::new();
    let b = f.store.bind(&scope("bind"), &f.request).unwrap();
    let prepared = f.store.prepare_binding(&scope("prepare"), &b.id).unwrap();
    f.store
        .revoke_binding(
            &scope("revoke"),
            &RevokeBindingRequest {
                binding_id: b.id.clone(),
            },
        )
        .unwrap();
    let query = PriceQuery {
        instrument: prepared.instrument.request.instrument,
        start: "2024-01-01".parse().unwrap(),
        end: "2024-02-01".parse().unwrap(),
        page_size: 10,
        cursor: None,
    };
    let mut repo = SqliteRepository::open(&f.financial).unwrap();
    let run = repo.start_market_run(&provider(), &query).unwrap();
    repo.save_prices_page(
        run,
        &PricePage {
            items: vec![],
            next_cursor: None,
            coverage: PriceCoverage {
                first_date: None,
                last_date: None,
                completeness: Completeness::Unverified,
            },
        },
    )
    .unwrap();
    repo.finish_market_run(run, None).unwrap();
    let result = FetchResult {
        provenance: FetchProvenance {
            scope: scope("prices"),
            provider: provider(),
            repository_id: repo.repository_identity().unwrap(),
            command: FetchCommand::Prices {
                instance_id: "fixture".into(),
                query: query.clone(),
            },
            runs: vec![RunReceipt {
                kind: RunKind::Market,
                id: run,
            }],
            document: None,
            instrument_observation: None,
            binding_id: Some(b.id.clone()),
        },
        error: None,
    };
    let fetch = f.store.record_fetch(&result).unwrap();
    let dataset = f
        .store
        .create_dataset(
            &scope("freeze"),
            &fetch.id,
            DatasetProjection::Prices {
                run_id: run,
                query,
                series: lugus_financial::selection::PriceSeries::Close,
            },
        )
        .unwrap();
    assert_eq!(dataset.binding_id, Some(b.id));
    drop(f.store);
    let reopened = open(&f.app, &f.financial);
    assert_eq!(
        reopened
            .dataset_header(&scope("read"), &dataset.id)
            .unwrap()
            .binding_id,
        dataset.binding_id
    );
}
#[test]
fn failed_lookup_receipt_remains_safe_without_financial_run() {
    let mut f = Fixture::new();
    let original = f
        .store
        .read_fetch(&scope("read"), &f.request.instrument_fetch_id)
        .unwrap();
    let result = FetchResult {
        provenance: FetchProvenance {
            scope: scope("failed"),
            provider: original.provider,
            repository_id: original.repository_id,
            command: original.command,
            runs: vec![],
            document: None,
            instrument_observation: None,
            binding_id: None,
        },
        error: Some(AppError::new(ErrorKind::RateLimited, "source secret", true)),
    };
    let receipt = f.store.record_fetch(&result).unwrap();
    assert!(receipt.runs.is_empty());
    assert!(receipt.instrument_observation.is_none());
    assert!(!receipt.error.as_ref().unwrap().message.contains("secret"));
    let request = BindRequest {
        instrument_fetch_id: receipt.id,
        ..f.request
    };
    assert_eq!(
        f.store
            .bind(&scope("bind-failed"), &request)
            .unwrap_err()
            .kind,
        ErrorKind::MissingData
    );
}
#[test]
fn binding_admission_reserves_every_status_envelope_before_mutation() {
    struct FixedId(String);
    impl IdSource for FixedId {
        fn next_id(&self) -> String {
            self.0.clone()
        }
    }
    struct FixedClock;
    impl Clock for FixedClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            "2026-09-10T00:00:00Z".parse().unwrap()
        }
    }
    let mut f = Fixture::new();
    let mut expected = f.store.bind(&scope("seed"), &f.request).unwrap();
    let escaped_id = "\\".repeat(Scope::MAX_ID_BYTES);
    let binding_scope = Scope {
        run_id: Some(escaped_id.clone()),
        ..scope("edge")
    };
    expected.id = escaped_id.clone();
    expected.scope = binding_scope.clone();
    expected.created_at = Clock::now(&FixedClock);
    let record_bytes = serde_json::to_vec(&expected).unwrap().len();
    let largest_view_bytes = serde_json::to_vec(&BindingView {
        record: expected,
        status: BindingStatus::Superseded {
            binding_id: escaped_id.clone(),
        },
    })
    .unwrap()
    .len();
    assert!(record_bytes >= Limits::MIN_OUTPUT_BYTES);
    let open_bounded = |maximum, id: String| {
        SqliteApplicationStore::open(
            &f.app,
            Box::new(SqliteRepository::open(&f.financial).unwrap()),
            Limits {
                max_output_bytes: maximum,
                max_read_page_bytes: maximum,
                ..Limits::default()
            },
            Box::new(FixedClock),
            Box::new(FixedId(id)),
        )
        .unwrap()
    };
    let conn = rusqlite::Connection::open(&f.app).unwrap();
    let counts = || {
        conn.query_row("SELECT (SELECT count(*) FROM app_records),(SELECT count(*) FROM binding_history),(SELECT count(*) FROM binding_requests)", [], |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?))).unwrap()
    };
    for maximum in [record_bytes, largest_view_bytes - 1] {
        let mut bounded = open_bounded(maximum, escaped_id.clone());
        let before = counts();
        assert_eq!(
            bounded.bind(&binding_scope, &f.request).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
        assert_eq!(
            counts(),
            before,
            "rejected admission must not append anything"
        );
    }
    let mut bounded = open_bounded(largest_view_bytes, escaped_id.clone());
    let first = bounded.bind(&binding_scope, &f.request).unwrap();
    assert_eq!(
        bounded
            .read_binding(&binding_scope, &first.id)
            .unwrap()
            .status,
        BindingStatus::Active
    );
    assert_eq!(
        bounded
            .prepare_binding(&binding_scope, &first.id)
            .unwrap()
            .id,
        first.id
    );
    // Removing the escaped run id exactly offsets adding the same escaped supersedes id.
    // Both revisions therefore fit the same boundary; the replacement ID is also maximally escaped.
    // A smaller replacement record must not let a tighter reopened store append
    // an unreadable superseded view to a previously accepted revision.
    let mut tightened = open_bounded(largest_view_bytes - 1, "\"".repeat(Scope::MAX_ID_BYTES));
    let before = counts();
    assert_eq!(
        tightened
            .bind(
                &Scope {
                    run_id: None,
                    ..scope("x")
                },
                &BindRequest {
                    supersedes: Some(first.id.clone()),
                    ..f.request.clone()
                }
            )
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(counts(), before);
    let next_scope = Scope {
        run_id: None,
        ..scope("next")
    };
    let mut replacement_store = open_bounded(largest_view_bytes, "\"".repeat(Scope::MAX_ID_BYTES));
    let next = replacement_store
        .bind(
            &next_scope,
            &BindRequest {
                supersedes: Some(first.id.clone()),
                ..f.request.clone()
            },
        )
        .unwrap();
    assert_eq!(
        bounded
            .read_binding(&binding_scope, &first.id)
            .unwrap()
            .status,
        BindingStatus::Superseded {
            binding_id: next.id.clone()
        }
    );
    assert_eq!(
        bounded.prepare_binding(&next_scope, &next.id).unwrap().id,
        next.id
    );
    assert_eq!(
        bounded
            .revoke_binding(
                &scope("revoke-boundary"),
                &RevokeBindingRequest {
                    binding_id: next.id.clone()
                }
            )
            .unwrap()
            .status,
        BindingStatus::Revoked
    );
    assert_eq!(
        bounded.read_binding(&next_scope, &next.id).unwrap().status,
        BindingStatus::Revoked
    );
}
