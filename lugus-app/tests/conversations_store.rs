mod conversations_support;
use conversations_support::*;
use lugus_app::{conversations::*, *};

#[test]
fn admission_is_atomic_idempotent_and_completion_cannot_be_downgraded() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    assert_eq!(s.create_conversation("create", "Research").unwrap(), c);
    assert_eq!(
        s.create_conversation("create", "Different")
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert!(s.workspace(&c.id).unwrap().view_ids.is_empty());
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let req = request(&c, "send");
    let a = s.admit(&epoch, &req).unwrap();
    assert_eq!(s.admit(&epoch, &req).unwrap(), a);
    assert_eq!(
        s.admit(&epoch, &request(&c, "another")).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 1);
    let attempt = s.start(&epoch, &c.id, &a.id).unwrap();
    let done = RunCompletion::Completed {
        text: "Answer".into(),
    };
    let result = s.finish_run(&attempt, &done).unwrap();
    assert_eq!(result.status, RunStatus::Completed);
    assert_eq!(s.finish_run(&attempt, &done).unwrap(), result);
    assert_eq!(
        s.finish_run(&attempt, &RunCompletion::Interrupted)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.admit(&epoch, &req).unwrap(), result);
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 2);
    let b = s.admit(&epoch, &request(&c, "next")).unwrap();
    assert_eq!(b.input.message_ids.len(), 3);
    let reader = store(&path, &fin, ConversationLimits::default());
    assert_eq!(
        reader.run(&c.id, &b.id).unwrap().status,
        RunStatus::Admitted
    );
}
#[test]
fn epoch_recovery_preserves_unknown_tools_and_fences_old_writes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let attempt = s.start(&epoch, &c.id, &r.id).unwrap();
    let tool = ToolIntent {
        call_id: "call-1".into(),
        name: "research.lookup".into(),
        arguments: "{}".into(),
        result_capacity: 1024,
    };
    assert!(matches!(
        s.begin_tool(&attempt, &tool).unwrap(),
        BeginTool::Dispatch(_)
    ));
    assert_eq!(
        s.begin_tool(&attempt, &tool).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.activate(&lease).unwrap_err().kind, ErrorKind::Conflict);
    drop(lease);
    assert_eq!(
        LocalExecutionLease::acquire(&path).unwrap_err().kind,
        ErrorKind::Conflict
    );
    drop(epoch);
    assert_eq!(
        LocalExecutionLease::acquire(&path).unwrap_err().kind,
        ErrorKind::Conflict
    );
    drop(attempt);
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let next = s.activate(&lease).unwrap();
    assert!(next.generation() > 1);
    assert_eq!(s.run(&c.id, &r.id).unwrap().status, RunStatus::Interrupted);

    let records = s.tool_records(&c.id, &r.id, page()).unwrap();
    assert!(records.items[0].outcome.is_none());
}
#[test]
fn repository_scope_and_page_preflight_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let other = store(
        &path,
        &dir.path().join("other.sqlite"),
        ConversationLimits::default(),
    );
    assert_eq!(
        other.conversation(&c.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        other.workspace(&c.id).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    assert_eq!(
        s.conversations(PageRequest {
            offset: 0,
            limit: 101
        })
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE conversations SET payload=?1 WHERE id=?2",
        rusqlite::params!["x".repeat(2_000_000), c.id],
    )
    .unwrap();
    assert_eq!(
        s.conversation(&c.id).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}

#[test]
fn journal_budgets_reserve_results_and_leave_terminal_capacity() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let limits = ConversationLimits {
        activity_events: 1,
        tool_calls: 1,
        ..ConversationLimits::default()
    };
    let mut s = store(&path, &fin, limits);
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    s.append_activity(&a, "text", "partial").unwrap();
    assert_eq!(
        s.append_activity(&a, "text", "more").unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    let intent = ToolIntent {
        call_id: "call".into(),
        name: "lookup".into(),
        arguments: "{}".into(),
        result_capacity: 512,
    };
    s.begin_tool(&a, &intent).unwrap();
    let result = ToolOutcome::Returned {
        result: lugus_agent::ToolResult {
            success: true,
            content: "{\"saved\":true}".into(),
        },
    };
    let receipt = s.finish_tool(&a, "call", &result).unwrap();
    assert_eq!(s.finish_tool(&a, "call", &result).unwrap(), receipt);
    assert!(matches!(
        s.begin_tool(&a, &intent).unwrap(),
        BeginTool::Recorded(_)
    ));
    assert_eq!(
        s.begin_tool(
            &a,
            &ToolIntent {
                arguments: "{\"other\":true}".into(),
                ..intent.clone()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        s.begin_tool(
            &a,
            &ToolIntent {
                call_id: "other".into(),
                ..intent
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    s.finish_run(
        &a,
        &RunCompletion::Completed {
            text: "Final".into(),
        },
    )
    .unwrap();
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 2);
}
#[test]
fn undersized_tool_result_reservation_rejects_before_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    assert_eq!(
        s.begin_tool(
            &a,
            &ToolIntent {
                call_id: "call".into(),
                name: "lookup".into(),
                arguments: "{}".into(),
                result_capacity: 1
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert!(
        s.tool_records(&c.id, &r.id, page())
            .unwrap()
            .items
            .is_empty()
    );
}
#[test]
fn completion_and_tool_failures_are_sanitized_and_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    let err = AppError::new(ErrorKind::Unavailable, "SECRET", true);
    s.begin_tool(
        &a,
        &ToolIntent {
            call_id: "call".into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            result_capacity: 512,
        },
    )
    .unwrap();
    let tool = s
        .finish_tool(&a, "call", &ToolOutcome::Failed { error: err.clone() })
        .unwrap();
    assert!(!serde_json::to_string(&tool).unwrap().contains("SECRET"));
    let failed = s
        .finish_run(&a, &RunCompletion::Failed { error: err })
        .unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert!(!serde_json::to_string(&failed).unwrap().contains("SECRET"));
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 1);
}
#[test]
fn stale_epoch_and_foreign_store_tokens_cannot_mutate() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    let mut other = store(
        &dir.path().join("other.sqlite"),
        &fin,
        ConversationLimits::default(),
    );
    assert_eq!(
        other.activate(&lease).unwrap_err().kind,
        ErrorKind::ScopeMismatch
    );
    // Simulates an externally advanced durable epoch; all callbacks must recheck the database.
    rusqlite::Connection::open(&path)
        .unwrap()
        .execute("UPDATE conversation_config SET epoch=epoch+1", [])
        .unwrap();
    assert_eq!(
        s.append_activity(&a, "text", "late").unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        s.finish_run(
            &a,
            &RunCompletion::Completed {
                text: "late".into()
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        s.begin_tool(
            &a,
            &ToolIntent {
                call_id: "call".into(),
                name: "lookup".into(),
                arguments: "{}".into(),
                result_capacity: 512
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
}
#[test]
fn simultaneous_connections_admit_one_winner_without_extra_message() {
    use std::sync::{Arc, Barrier};
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let barrier = Arc::new(Barrier::new(2));
    let handles = (0..2)
        .map(|i| {
            let mut s = store(&path, &fin, ConversationLimits::default());
            let c = c.clone();
            let epoch = epoch.clone();
            let barrier = barrier.clone();
            std::thread::spawn(move || {
                barrier.wait();
                s.admit(&epoch, &request(&c, &format!("send-{i}")))
            })
        })
        .collect::<Vec<_>>();
    let results = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(results.iter().filter(|r| r.is_ok()).count(), 1);
    assert_eq!(
        results
            .iter()
            .filter_map(|r| r.as_ref().err())
            .next()
            .unwrap()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 1);
}
#[test]
fn legacy_open_loads_persisted_limits_without_recovery() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let limits = ConversationLimits {
        open_views: 2,
        ..ConversationLimits::default()
    };
    let mut s = store(&path, &fin, limits.clone());
    let c = s.create_conversation("create", "Research").unwrap();
    let reader = legacy_store(&path, &fin);
    assert_eq!(reader.conversation_limits(), &limits);
    assert_eq!(reader.conversation(&c.id).unwrap(), c);
}
#[test]
fn workspace_transitions_preserve_selection_and_reject_stale_or_invalid_sets() {
    let limits = ConversationLimits::default();
    let state = WorkspaceState {
        conversation_id: "c".into(),
        workspace_id: "w".into(),
        revision: 4,
        view_ids: vec!["a".into(), "b".into(), "c".into()],
        selected_view_id: Some("b".into()),
    };
    assert_eq!(
        workspace_transition(
            &state,
            3,
            &WorkspaceMutation::Close {
                view_id: "b".into()
            },
            &limits
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    for ids in [vec!["a", "a", "c"], vec!["a", "b"], vec!["a", "b", "x"]] {
        assert!(
            workspace_transition(
                &state,
                4,
                &WorkspaceMutation::Reorder {
                    view_ids: ids.into_iter().map(String::from).collect()
                },
                &limits
            )
            .is_err()
        );
    }
    let reordered = workspace_transition(
        &state,
        4,
        &WorkspaceMutation::Reorder {
            view_ids: vec!["c".into(), "b".into(), "a".into()],
        },
        &limits,
    )
    .unwrap();
    assert_eq!(reordered.selected_view_id.as_deref(), Some("b"));
    assert_eq!(reordered.revision, 5);
    let closed = workspace_transition(
        &state,
        4,
        &WorkspaceMutation::Close {
            view_id: "b".into(),
        },
        &limits,
    )
    .unwrap();
    assert_eq!(closed.selected_view_id.as_deref(), Some("c"));
    let closed = workspace_transition(
        &closed,
        5,
        &WorkspaceMutation::Close {
            view_id: "c".into(),
        },
        &limits,
    )
    .unwrap();
    assert_eq!(closed.selected_view_id.as_deref(), Some("a"));
    let closed = workspace_transition(
        &closed,
        6,
        &WorkspaceMutation::Close {
            view_id: "a".into(),
        },
        &limits,
    )
    .unwrap();
    assert_eq!(closed.selected_view_id, None);
}
#[test]
fn fresh_workspace_identity_cannot_adopt_a_legacy_workspace() {
    struct CollisionIds;
    impl IdSource for CollisionIds {
        fn next_id(&self) -> String {
            "legacy-workspace".into()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let _s = store(&path, &fin, ConversationLimits::default());
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute("INSERT INTO app_records VALUES('old','legacy-workspace','legacy-repo','fetch','unchanged')",[]).unwrap();
    let mut s = SqliteApplicationStore::open(
        &path,
        Box::new(lugus_financial::storage::SqliteRepository::open(&fin).unwrap()),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(CollisionIds),
    )
    .unwrap();
    assert_eq!(
        s.create_conversation("create", "Research")
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert!(s.conversations(page()).unwrap().items.is_empty());
}
#[test]
fn v2_migration_keeps_raw_evidence_and_binding_rows() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute_batch("CREATE TABLE app_records(id TEXT PRIMARY KEY,workspace TEXT NOT NULL,repository TEXT NOT NULL,category TEXT NOT NULL,payload TEXT NOT NULL);CREATE TABLE dataset_rows(dataset_id TEXT NOT NULL,ordinal INTEGER NOT NULL,payload TEXT NOT NULL,observation_id INTEGER,PRIMARY KEY(dataset_id,ordinal));CREATE TABLE view_requests(workspace TEXT NOT NULL,request TEXT NOT NULL,input TEXT NOT NULL,view_id TEXT NOT NULL,PRIMARY KEY(workspace,request));CREATE TABLE binding_history(sequence INTEGER PRIMARY KEY,binding_id TEXT,payload TEXT);PRAGMA application_id=1280657235;PRAGMA user_version=2;INSERT INTO app_records VALUES('old','legacy','repo','view',' { \"original\" : true } ');INSERT INTO binding_history VALUES(1,'binding','raw binding history');INSERT INTO dataset_rows VALUES('dataset',0,'raw dataset row',42);").unwrap();
    let s = legacy_store(&path, &fin);
    assert!(s.conversations(page()).unwrap().items.is_empty());
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        3
    );
    for (query, want) in [
        (
            "SELECT payload FROM app_records",
            " { \"original\" : true } ",
        ),
        ("SELECT payload FROM binding_history", "raw binding history"),
        ("SELECT payload FROM dataset_rows", "raw dataset row"),
    ] {
        assert_eq!(
            sql.query_row(query, [], |r| r.get::<_, String>(0)).unwrap(),
            want
        );
    }
}
#[test]
fn duplicate_lookup_survives_full_capacity_and_start_claims_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(
        &path,
        &fin,
        ConversationLimits {
            active_runs: 1,
            ..ConversationLimits::default()
        },
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let other = s.create_conversation("other", "Other").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let req = request(&c, "send");
    let r = s.admit(&epoch, &req).unwrap();
    assert_eq!(s.lookup_request(&req).unwrap(), Some(r.clone()));
    assert_eq!(
        s.admit(&epoch, &request(&other, "send")).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    assert_eq!(
        s.start(&epoch, &c.id, &r.id).unwrap_err().kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        s.lookup_request(&req).unwrap().unwrap().status,
        RunStatus::Running
    );
    s.begin_tool(
        &a,
        &ToolIntent {
            call_id: "call".into(),
            name: "fetch".into(),
            arguments: "{}".into(),
            result_capacity: 512,
        },
    )
    .unwrap();
    let outcome = ToolOutcome::Returned {
        result: lugus_agent::ToolResult {
            success: false,
            content: "{\"receipt\":\"saved-partial-evidence\"}".into(),
        },
    };
    s.finish_tool(&a, "call", &outcome).unwrap();
    assert_eq!(
        s.tool_records(&c.id, &r.id, page()).unwrap().items[0].outcome,
        Some(outcome)
    );
}
#[test]
fn durable_message_input_and_finished_tool_records_are_immutable() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    s.begin_tool(
        &a,
        &ToolIntent {
            call_id: "call".into(),
            name: "lookup".into(),
            arguments: "{}".into(),
            result_capacity: 512,
        },
    )
    .unwrap();
    s.finish_tool(
        &a,
        "call",
        &ToolOutcome::Returned {
            result: lugus_agent::ToolResult {
                success: true,
                content: "saved".into(),
            },
        },
    )
    .unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    for statement in [
        "UPDATE conversation_messages SET payload='changed'",
        "DELETE FROM conversation_messages",
        "UPDATE conversation_runs SET input='changed'",
        "UPDATE conversation_runs SET request_input='changed'",
        "UPDATE conversation_tools SET intent='changed'",
        "UPDATE conversation_tools SET payload='changed'",
    ] {
        assert!(sql.execute(statement, []).is_err(), "{statement}");
    }
}
#[test]
fn page_envelopes_and_run_snapshot_reserves_fail_before_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(
        &path,
        &fin,
        ConversationLimits {
            page_bytes: 2048,
            ..ConversationLimits::default()
        },
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    assert_eq!(
        s.admit(&epoch, &request(&c, "send")).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert!(s.messages(&c.id, page()).unwrap().items.is_empty());
    assert!(s.runs(&c.id, page()).unwrap().items.is_empty());
    let tiny = ConversationLimits {
        message_bytes: 250,
        ..ConversationLimits::default()
    };
    let mut s = store(&dir.path().join("tiny.sqlite"), &fin, tiny);
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(s.execution_store_key()).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let mut req = request(&c, "send");
    req.text = "\"".repeat(85);
    assert_eq!(
        s.admit(&epoch, &req).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
    assert!(s.messages(&c.id, page()).unwrap().items.is_empty());
}
#[test]
fn journal_page_validation_precedes_any_run_payload_decode() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE conversation_runs SET payload='malformed' WHERE id=?1",
        [&r.id],
    )
    .unwrap();
    assert_eq!(
        s.activity(
            &c.id,
            &r.id,
            PageRequest {
                offset: 0,
                limit: 0
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert!(s.activity(&c.id, &r.id, page()).unwrap().items.is_empty());
}
#[test]
fn idempotent_create_and_completion_do_not_need_new_ids() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    };
    struct Ids {
        broken: Arc<AtomicBool>,
        counter: AtomicU64,
    }
    impl IdSource for Ids {
        fn next_id(&self) -> String {
            if self.broken.load(Ordering::SeqCst) {
                String::new()
            } else {
                format!("id-{}", self.counter.fetch_add(1, Ordering::SeqCst))
            }
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let broken = Arc::new(AtomicBool::new(false));
    let mut s = SqliteApplicationStore::open(
        &path,
        Box::new(
            lugus_financial::storage::SqliteRepository::open(dir.path().join("fin.sqlite"))
                .unwrap(),
        ),
        Limits::default(),
        Box::new(SystemClock),
        Box::new(Ids {
            broken: broken.clone(),
            counter: AtomicU64::new(1),
        }),
    )
    .unwrap();
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    let done = RunCompletion::Completed {
        text: "Answer".into(),
    };
    let complete = s.finish_run(&a, &done).unwrap();
    broken.store(true, Ordering::SeqCst);
    assert_eq!(s.create_conversation("create", "Research").unwrap(), c);
    assert_eq!(s.finish_run(&a, &done).unwrap(), complete);
}
#[test]
fn aggregate_journal_bytes_and_result_reservation_block_excess_effects() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(
        &path,
        &fin,
        ConversationLimits {
            tool_total_bytes: 1100,
            tool_record_bytes: 1000,
            activity_bytes: 200,
            ..ConversationLimits::default()
        },
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let r = s.admit(&epoch, &request(&c, "send")).unwrap();
    let a = s.start(&epoch, &c.id, &r.id).unwrap();
    let intent = ToolIntent {
        call_id: "call".into(),
        name: "fetch".into(),
        arguments: "{}".into(),
        result_capacity: 512,
    };
    s.begin_tool(&a, &intent).unwrap();
    assert_eq!(
        s.begin_tool(
            &a,
            &ToolIntent {
                call_id: "second".into(),
                ..intent
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert_eq!(
        s.finish_tool(
            &a,
            "call",
            &ToolOutcome::Returned {
                result: lugus_agent::ToolResult {
                    success: true,
                    content: "x".repeat(900)
                }
            }
        )
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    assert!(
        s.tool_records(&c.id, &r.id, page()).unwrap().items[0]
            .outcome
            .is_none()
    );
    s.finish_tool(
        &a,
        "call",
        &ToolOutcome::Failed {
            error: AppError::new(ErrorKind::ResourceLimit, "too large", false),
        },
    )
    .unwrap();
    s.append_activity(&a, "text", &"x".repeat(40)).unwrap();
    assert_eq!(
        s.append_activity(&a, "text", &"x".repeat(40))
            .unwrap_err()
            .kind,
        ErrorKind::ResourceLimit
    );
    s.finish_run(
        &a,
        &RunCompletion::Completed {
            text: "Final".into(),
        },
    )
    .unwrap();
}

#[test]
fn admitted_failure_is_atomic_fenced_and_never_downgrades_an_attempt() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let financial = dir.path().join("financial.sqlite");
    let mut s = store(&path, &financial, ConversationLimits::default());
    let c = s.create_conversation("create", "Research").unwrap();
    let other = s.create_conversation("other", "Other").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let error = AppError::new(ErrorKind::Storage, "private diagnostic", false);
    let admitted = s.admit(&epoch, &request(&c, "one")).unwrap();
    assert_eq!(
        s.fail_admission(&epoch, &other.id, &admitted.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    let mut foreign_repository = store(
        &path,
        &dir.path().join("foreign.sqlite"),
        ConversationLimits::default(),
    );
    assert_eq!(
        foreign_repository
            .fail_admission(&epoch, &c.id, &admitted.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    let mut foreign_store = store(
        &dir.path().join("other-app.sqlite"),
        &financial,
        ConversationLimits::default(),
    );
    assert_eq!(
        foreign_store
            .fail_admission(&epoch, &c.id, &admitted.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
    let failed = s
        .fail_admission(&epoch, &c.id, &admitted.id, &error)
        .unwrap();
    assert_eq!(failed.status, RunStatus::Failed);
    assert!(failed.finished_at.is_some());
    assert_eq!(failed.input, admitted.input);
    assert_eq!(failed.error.as_ref().unwrap().kind, ErrorKind::Storage);
    assert!(
        !failed
            .error
            .as_ref()
            .unwrap()
            .message
            .contains("private diagnostic")
    );
    assert_eq!(
        s.fail_admission(&epoch, &c.id, &admitted.id, &error)
            .unwrap(),
        failed
    );
    assert_eq!(
        s.fail_admission(
            &epoch,
            &c.id,
            &admitted.id,
            &AppError::new(ErrorKind::Timeout, "different failure", false)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 1);
    assert_eq!(
        s.start(&epoch, &c.id, &admitted.id).unwrap_err().kind,
        ErrorKind::Conflict
    );

    let mut running = s.admit(&epoch, &request(&c, "two")).unwrap();
    let attempt = s.start(&epoch, &c.id, &running.id).unwrap();
    // This is the sole change applied by successful start: hosts need no fallible post-start read.
    running.status = RunStatus::Running;
    assert_eq!(s.run(&c.id, &running.id).unwrap(), running);
    assert_eq!(
        s.fail_admission(&epoch, &c.id, &running.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    let completed = s
        .finish_run(
            &attempt,
            &RunCompletion::Completed {
                text: "Answer".into(),
            },
        )
        .unwrap();
    assert_eq!(
        s.fail_admission(&epoch, &c.id, &running.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(s.run(&c.id, &running.id).unwrap(), completed);

    let stale = s.admit(&epoch, &request(&other, "three")).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute("UPDATE conversation_config SET epoch=epoch+1", [])
        .unwrap();
    assert_eq!(
        s.fail_admission(&epoch, &other.id, &stale.id, &error)
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(
        s.run(&other.id, &stale.id).unwrap().status,
        RunStatus::Admitted
    );
}

#[test]
fn admitted_failure_rolls_back_if_terminal_metadata_cannot_commit() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let mut s = store(
        &path,
        &dir.path().join("financial.sqlite"),
        ConversationLimits::default(),
    );
    let c = s.create_conversation("create", "Research").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let admitted = s.admit(&epoch, &request(&c, "one")).unwrap();
    let raw = rusqlite::Connection::open(&path).unwrap();
    raw.execute_batch("CREATE TRIGGER reject_completion BEFORE UPDATE OF completion ON conversation_runs BEGIN SELECT RAISE(ABORT, 'fixture'); END;").unwrap();
    assert_eq!(
        s.fail_admission(
            &epoch,
            &c.id,
            &admitted.id,
            &AppError::new(ErrorKind::Storage, "start failed", false)
        )
        .unwrap_err()
        .kind,
        ErrorKind::Storage
    );
    assert_eq!(s.run(&c.id, &admitted.id).unwrap(), admitted);
    assert_eq!(s.messages(&c.id, page()).unwrap().items.len(), 1);
}

#[test]
fn external_metadata_commit_during_preparation_conflicts_without_admitting_message() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::time::Duration;
    struct PreparationClock {
        armed: Arc<AtomicBool>,
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
    }
    impl Clock for PreparationClock {
        fn now(&self) -> chrono::DateTime<chrono::Utc> {
            if self.armed.swap(false, Ordering::SeqCst) {
                self.entered.send(()).unwrap();
                self.release.recv_timeout(Duration::from_secs(3)).unwrap();
            }
            chrono::Utc::now()
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let financial = dir.path().join("financial.sqlite");
    let armed = Arc::new(AtomicBool::new(false));
    let (entered, preparing) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let mut preparing_store = SqliteApplicationStore::open(
        &path,
        Box::new(lugus_financial::storage::SqliteRepository::open(&financial).unwrap()),
        Limits::default(),
        Box::new(PreparationClock {
            armed: armed.clone(),
            entered,
            release: resume,
        }),
        Box::new(RandomIds::new().unwrap()),
    )
    .unwrap();
    let c = preparing_store
        .create_conversation("create", "Research")
        .unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = preparing_store.activate(&lease).unwrap();
    let mut external = store(&path, &financial, ConversationLimits::default());
    let before = external.workspace(&c.id).unwrap();
    let req = request(&c, "one");
    armed.store(true, Ordering::SeqCst);
    let preparation = std::thread::spawn(move || {
        let result = preparing_store.admit(&epoch, &req);
        (preparing_store, epoch, req, result)
    });
    // now() occurs after the initial snapshot transaction and selected-reference
    // reads, before context serialization and the final immediate transaction.
    preparing.recv_timeout(Duration::from_secs(3)).unwrap();
    let committed = external
        .mutate_workspace(
            &c.id,
            before.revision,
            &WorkspaceMutation::Reorder { view_ids: vec![] },
        )
        .unwrap();
    assert_eq!(committed.revision, before.revision + 1);
    release.send(()).unwrap();
    let (mut preparing_store, epoch, req, result) = preparation.join().unwrap();
    assert_eq!(result.unwrap_err().kind, ErrorKind::Conflict);
    assert!(external.messages(&c.id, page()).unwrap().items.is_empty());
    assert!(external.runs(&c.id, page()).unwrap().items.is_empty());
    assert_eq!(preparing_store.workspace(&c.id).unwrap(), committed);
    let retry = preparing_store.admit(&epoch, &req).unwrap();
    assert_eq!(retry.status, RunStatus::Admitted);
    assert_eq!(preparing_store.admit(&epoch, &req).unwrap(), retry);
    assert_eq!(external.messages(&c.id, page()).unwrap().items.len(), 1);
    assert_eq!(external.runs(&c.id, page()).unwrap().items.len(), 1);
}
