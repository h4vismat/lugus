#[allow(dead_code)]
mod conversations_support;
use conversations_support::store;
use lugus_app::conversations::{ConversationLimits, ConversationStore};
#[test]
fn migration_preserves_existing_conversation_and_reopens() {
    let d = tempfile::tempdir().unwrap();
    let db = d.path().join("a");
    let fin = d.path().join("f");
    let mut s = store(&db, &fin, ConversationLimits::default());
    let c = s.create_conversation("original", "Keep me").unwrap();
    drop(s);
    // Downgrade only the new empty comparison schema to construct a real v8 fixture.
    let conn = rusqlite::Connection::open(&db).unwrap();
    conn.execute_batch("DROP TABLE IF EXISTS comparison_dependencies; DROP TABLE IF EXISTS comparison_entries; DROP TABLE IF EXISTS comparison_records; DROP TABLE IF EXISTS research_packages; DROP TABLE IF EXISTS comparison_jobs; PRAGMA user_version=8;").unwrap();
    drop(conn);
    let s = store(&db, &fin, ConversationLimits::default());
    assert_eq!(s.conversation(&c.id).unwrap().title, "Keep me");
    drop(s);
    let c = rusqlite::Connection::open(&db).unwrap();
    assert_eq!(
        c.query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        9
    );
    drop(c);
    let _s = store(&db, &fin, ConversationLimits::default());
}
