mod conversations_support;
mod support;
use conversations_support::{legacy_store, page, request, store};
use lugus_app::{conversations::*, *};

#[tokio::test]
async fn application_lists_newest_empty_chats_first_without_changing_original_order() {
    let h = support::Harness::new(&[], HostBounds::default(), Limits::default()).await;
    let first = h.app.create_conversation("first", "First").await.unwrap();
    let second = h.app.create_conversation("second", "Second").await.unwrap();
    let third = h.app.create_conversation("third", "Third").await.unwrap();
    assert_eq!(
        h.app.recent_conversations(page()).await.unwrap().items,
        vec![third.clone(), second.clone(), first.clone()]
    );
    assert_eq!(
        h.app.conversations(page()).await.unwrap().items,
        vec![first, second, third]
    );
    h.app.shutdown().await.unwrap();
}

#[test]
fn new_message_moves_old_chat_first_and_paging_survives_offline_reopening() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("financial.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let first = s.create_conversation("first", "First").unwrap();
    let second = s.create_conversation("second", "Second").unwrap();
    let third = s.create_conversation("third", "Third").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    s.admit(&epoch, &request(&first, "send")).unwrap();
    let one = s
        .recent_conversations(PageRequest {
            offset: 0,
            limit: 2,
        })
        .unwrap();
    assert_eq!(one.items, vec![first.clone(), third.clone()]);
    assert_eq!(one.next_offset, Some(2));
    let two = s
        .recent_conversations(PageRequest {
            offset: 2,
            limit: 2,
        })
        .unwrap();
    assert_eq!(two.items, vec![second.clone()]);
    assert_eq!(two.next_offset, None);
    let empty = s
        .recent_conversations(PageRequest {
            offset: 3,
            limit: 2,
        })
        .unwrap();
    assert!(empty.items.is_empty());
    assert_eq!(empty.next_offset, None);
    drop(s);
    let reopened = legacy_store(&path, &fin);
    assert_eq!(
        reopened.recent_conversations(page()).unwrap().items,
        vec![first, third, second]
    );
}

#[test]
fn recent_page_limits_are_enforced() {
    let dir = tempfile::tempdir().unwrap();
    let s = store(
        &dir.path().join("app.sqlite"),
        &dir.path().join("fin.sqlite"),
        ConversationLimits::default(),
    );
    for page in [
        PageRequest {
            offset: 0,
            limit: 0,
        },
        PageRequest {
            offset: usize::MAX,
            limit: 1,
        },
        PageRequest {
            offset: 0,
            limit: usize::MAX,
        },
    ] {
        assert_eq!(
            s.recent_conversations(page).unwrap_err().kind,
            ErrorKind::ResourceLimit
        );
    }
}

#[test]
fn newest_empty_chat_and_assistant_completion_each_become_most_recent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let mut s = store(
        &path,
        &dir.path().join("fin.sqlite"),
        ConversationLimits::default(),
    );
    let first = s.create_conversation("first", "First").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    let run = s.admit(&epoch, &request(&first, "send")).unwrap();
    let attempt = s.start(&epoch, &first.id, &run.id).unwrap();
    let empty = s.create_conversation("empty", "Empty").unwrap();
    assert_eq!(
        s.recent_conversations(page()).unwrap().items,
        vec![empty.clone(), first.clone()]
    );
    s.finish_run(
        &attempt,
        &RunCompletion::Completed {
            text: "Done".into(),
        },
    )
    .unwrap();
    assert_eq!(
        s.recent_conversations(page()).unwrap().items,
        vec![first, empty]
    );
}

#[test]
fn v4_migration_backfills_recency_without_changing_existing_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let fin = dir.path().join("fin.sqlite");
    let mut s = store(&path, &fin, ConversationLimits::default());
    let first = s.create_conversation("first", "First").unwrap();
    let second = s.create_conversation("second", "Second").unwrap();
    let third = s.create_conversation("third", "Third").unwrap();
    let lease = LocalExecutionLease::acquire(&path).unwrap();
    let epoch = s.activate(&lease).unwrap();
    s.admit(&epoch, &request(&first, "send")).unwrap();
    let original_messages = s.messages(&first.id, page()).unwrap();
    drop(s);
    let sql = rusqlite::Connection::open(&path).unwrap();
    // Remove exactly the v5 additions to reconstruct the previous on-disk schema.
    sql.execute_batch(
        "DROP TABLE portfolio_history_evidence; DROP TABLE portfolio_performance_days; DROP TABLE portfolio_history_jobs; DROP TABLE portfolio_snapshot_rows; DROP TABLE portfolio_snapshots; DROP TABLE portfolio_prices; DROP TABLE portfolio_refreshes; DROP TABLE portfolio_history; DROP TABLE portfolio_requests; DROP TABLE portfolio_event_ids; DROP TABLE portfolios; DROP TABLE conversation_preparations;
        DROP TRIGGER conversation_recency_create;
        DROP TRIGGER conversation_recency_message;
        DROP TABLE conversation_recency;
        DROP TABLE IF EXISTS comparison_dependencies; DROP TABLE IF EXISTS comparison_entries; DROP TABLE IF EXISTS comparison_records; DROP TABLE IF EXISTS research_packages; DROP TABLE IF EXISTS comparison_jobs; PRAGMA user_version=4;",
    )
    .unwrap();
    let reopened = legacy_store(&path, &fin);
    assert_eq!(
        reopened.recent_conversations(page()).unwrap().items,
        vec![first.clone(), third.clone(), second.clone()]
    );
    assert_eq!(
        reopened.messages(&first.id, page()).unwrap(),
        original_messages
    );
    assert_eq!(
        reopened.conversations(page()).unwrap().items,
        vec![first, second, third]
    );
    assert_eq!(
        sql.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
}

#[test]
fn recent_pages_are_repository_scoped_and_preflight_payload_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("app.sqlite");
    let mut first_store = store(
        &path,
        &dir.path().join("first.sqlite"),
        ConversationLimits::default(),
    );
    let own = first_store.create_conversation("own", "Own").unwrap();
    let mut other_store = store(
        &path,
        &dir.path().join("other.sqlite"),
        ConversationLimits::default(),
    );
    let other = other_store.create_conversation("other", "Other").unwrap();
    assert_eq!(
        first_store.recent_conversations(page()).unwrap().items,
        vec![own.clone()]
    );
    assert_eq!(
        other_store.recent_conversations(page()).unwrap().items,
        vec![other]
    );
    let sql = rusqlite::Connection::open(&path).unwrap();
    sql.execute(
        "UPDATE conversations SET payload=?1 WHERE id=?2",
        rusqlite::params![
            "x".repeat(ConversationLimits::default().page_bytes + 1),
            own.id
        ],
    )
    .unwrap();
    assert_eq!(
        first_store.recent_conversations(page()).unwrap_err().kind,
        ErrorKind::ResourceLimit
    );
}
