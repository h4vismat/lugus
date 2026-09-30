mod conversations_support;
use conversations_support::*;
use lugus_app::{conversations::*, *};

#[test]
fn exact_receipts_survive_failure_and_reopen_without_changing_input() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = legacy_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "one")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    assert_eq!(s.preparation(&c.id, &r.id).unwrap(), None);
    let payload = " { \"source\" : \"évidence\", \"data\": [1,2] } ";
    s.save_preparation(&a, payload).unwrap();
    s.save_preparation(&a, payload).unwrap();
    assert_eq!(
        s.save_preparation(&a, "{}").unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.run(&c.id, &r.id).unwrap().input, r.input);
    s.finish_run(
        &a,
        &RunCompletion::Failed {
            error: AppError::new(ErrorKind::Storage, "analysis failed", false),
        },
    )
    .unwrap();
    assert_eq!(
        s.save_preparation(&a, payload).unwrap_err().kind,
        ErrorKind::Conflict
    );
    let next = s.admit(&epoch, &request(&c, "two")).unwrap();
    drop(s);
    let mut s = legacy_store(&path, &fin);
    assert_eq!(
        s.preparation(&c.id, &r.id).unwrap().as_deref(),
        Some(payload)
    );
    assert_eq!(
        s.latest_preparation(&c.id).unwrap().as_deref(),
        Some(payload)
    );
    let second = s.start(&epoch, &c.id, &next.id).unwrap();
    s.save_preparation(&second, "{\"new\":true}").unwrap();
    assert_eq!(
        s.latest_preparation(&c.id).unwrap().as_deref(),
        Some("{\"new\":true}")
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    for statement in [
        "UPDATE conversation_preparations SET payload='{}'",
        "DELETE FROM conversation_preparations",
    ] {
        assert!(sql.execute(statement, []).is_err());
    }
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 2);
}

#[test]
fn scope_and_epoch_checks_apply_even_without_receipts() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = legacy_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let other = s.create_conversation("other", "Other").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "one")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    assert_eq!(
        s.preparation(&other.id, &r.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        s.preparation(&c.id, "missing").unwrap_err().kind,
        ErrorKind::MissingData
    );
    assert_eq!(
        s.latest_preparation("missing").unwrap_err().kind,
        ErrorKind::MissingData
    );
    let mut foreign = legacy_store(&path, &dir.path().join("foreign.sqlite"));
    assert_eq!(
        foreign.preparation(&c.id, &r.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        foreign.latest_preparation(&c.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        foreign.save_preparation(&a, "{}").unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE conversation_config SET epoch=epoch+1", [])
        .unwrap();
    assert_eq!(
        s.save_preparation(&a, "{}").unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.latest_preparation(&c.id).unwrap(), None);
}

#[test]
fn invalid_and_oversized_json_never_mutate_and_corrupt_reads_are_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(
        &path,
        &fin,
        ConversationLimits {
            selected_bytes: 64,
            ..ConversationLimits::default()
        },
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "one")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    for payload in ["[]", "null", "broken", "42"] {
        assert_eq!(
            s.save_preparation(&a, payload).unwrap_err().kind,
            ErrorKind::InvalidInput
        );
    }
    let oversized = format!("{{\"x\":\"{}\"}}", "é".repeat(32));
    assert_eq!(
        s.save_preparation(&a, &oversized).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(s.preparation(&c.id, &r.id).unwrap(), None);
    let exact = format!("{{\"x\":\"{}\"}}", "a".repeat(56));
    assert_eq!(exact.len(), 64);
    s.save_preparation(&a, &exact).unwrap();
    assert_eq!(s.preparation(&c.id, &r.id).unwrap(), Some(exact));
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("DROP TRIGGER conversation_preparations_update; UPDATE conversation_preparations SET payload=printf('%1000000s','x');").unwrap();
    assert_eq!(
        s.preparation(&c.id, &r.id).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        s.latest_preparation(&c.id).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn v5_migration_preserves_runs_messages_and_workspace() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = legacy_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "one")).unwrap();
    let messages = s.messages(&c.id, page()).unwrap();
    let workspace = s.workspace(&c.id).unwrap();
    drop(s);
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("DROP TABLE portfolio_history_evidence; DROP TABLE portfolio_performance_days; DROP TABLE portfolio_history_jobs; DROP TABLE portfolio_snapshot_rows; DROP TABLE portfolio_snapshots; DROP TABLE portfolio_prices; DROP TABLE portfolio_refreshes; DROP TABLE portfolio_history; DROP TABLE portfolio_requests; DROP TABLE portfolio_event_ids; DROP TABLE portfolios; DROP TABLE conversation_preparations; DROP TABLE IF EXISTS comparison_dependencies; DROP TABLE IF EXISTS comparison_entries; DROP TABLE IF EXISTS comparison_records; DROP TABLE IF EXISTS research_packages; DROP TABLE IF EXISTS comparison_jobs; PRAGMA user_version=5;")
        .unwrap();
    let s = legacy_store(&path, &fin);
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    assert_eq!(s.run(&c.id, &r.id).unwrap(), r);
    assert_eq!(s.messages(&c.id, page()).unwrap(), messages);
    assert_eq!(s.workspace(&c.id).unwrap(), workspace);
    assert_eq!(s.latest_preparation(&c.id).unwrap(), None);
}

#[test]
fn context_and_output_caps_reserve_serialized_string_overhead() {
    let dir = tempfile::tempdir().unwrap();
    for (name, context_bytes, output_bytes, cap) in [
        ("context", 4096, 1_048_576, 4096),
        ("output", 262_144, 65_536, (65_536 - 2) / 6),
    ] {
        let path = dir.path().join(format!("{name}.sqlite"));
        let mut s = SqliteApplicationStore::open_with_conversation_limits(
            &path,
            Box::new(
                lugus_financial::storage::SqliteRepository::open(dir.path().join("fin.sqlite"))
                    .unwrap(),
            ),
            Limits {
                max_output_bytes: output_bytes,
                max_read_page_bytes: output_bytes,
                ..Limits::default()
            },
            ConversationLimits {
                context_bytes,
                ..ConversationLimits::default()
            },
            Box::new(SystemClock),
            Box::new(RandomIds::new().unwrap()),
        )
        .unwrap();
        let c = s.create_conversation("create", "Research").unwrap();
        let lease = LocalExecutionLease::acquire(&path).unwrap();
        let epoch = s.activate(&lease).unwrap();
        let r = s.admit(&epoch, &request(&c, "one")).unwrap();
        let a = s.start(&epoch, &c.id, &r.id).unwrap();
        let payload = format!("{{\"x\":\"{}\"}}", "a".repeat(cap - 8));
        assert_eq!(
            s.save_preparation(&a, &(payload.clone() + " "))
                .unwrap_err()
                .kind,
            ErrorKind::ResourceLimit
        );
        s.save_preparation(&a, &payload).unwrap();
        let read = s.preparation(&c.id, &r.id).unwrap();
        assert_eq!(read.as_deref(), Some(payload.as_str()));
        assert!(serde_json::to_vec(&read).unwrap().len() <= output_bytes);
    }
}

#[test]
fn company_hint_is_durable_and_part_of_request_idempotency() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = legacy_store(&path, &fin);
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let mut req = request(&c, "one");
    req.company_hint = Some("Apple / AAPL".into());
    let r = s.admit(&epoch, &req).unwrap();
    assert_eq!(r.company_hint, req.company_hint);
    assert_eq!(s.admit(&epoch, &req).unwrap(), r);
    req.company_hint = Some("Microsoft / MSFT".into());
    assert_eq!(s.admit(&epoch, &req).unwrap_err().kind, ErrorKind::Conflict);
    drop(s);
    let s = legacy_store(&path, &fin);
    assert_eq!(s.run(&c.id, &r.id).unwrap().company_hint, r.company_hint);
    // Older persisted runs deserialize without a hint and keep their original wire representation.
    let mut legacy = serde_json::to_value(&r).unwrap();
    legacy.as_object_mut().unwrap().remove("company_hint");
    let old: RunRecord = serde_json::from_value(legacy.clone()).unwrap();
    assert_eq!(old.company_hint, None);
    assert_eq!(serde_json::to_value(old).unwrap(), legacy);
}

#[test]
fn long_history_leaves_space_for_prepared_evidence() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let mut s = store(
        &path,
        &dir.path().join("fin.sqlite"),
        ConversationLimits {
            context_bytes: 8192,
            selected_bytes: 4096,
            ..ConversationLimits::default()
        },
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    for i in 0..8 {
        let mut req = request(&c, &format!("turn-{i}"));
        req.text = "Explain the reported financial evidence. ".repeat(20);
        let run = s.admit(&epoch, &req).unwrap();
        let attempt = s.start(&epoch, &c.id, &run.id).unwrap();
        s.finish_run(
            &attempt,
            &RunCompletion::Completed {
                text: "This answer cites saved financial evidence. ".repeat(20),
            },
        )
        .unwrap();
    }
    let run = s.admit(&epoch, &request(&c, "followup")).unwrap();
    assert!(run.input.omitted_messages > 0);
    assert!(8192 - run.input.serialized.len() >= 8192 / 3);
    let attempt = s.start(&epoch, &c.id, &run.id).unwrap();
    let package = serde_json::json!({"evidence": "a".repeat(2500)}).to_string();
    s.save_preparation(&attempt, &package).unwrap();
    assert!(run.input.serialized.len() + package.len() <= 8192);
    assert_eq!(s.preparation(&c.id, &run.id).unwrap(), Some(package));
}
