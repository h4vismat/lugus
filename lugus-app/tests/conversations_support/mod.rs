use lugus_app::conversations::*;
use lugus_app::*;
use lugus_financial::storage::SqliteRepository;
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
static IDS: AtomicU64 = AtomicU64::new(1);
struct TestIds;
impl IdSource for TestIds {
    fn next_id(&self) -> String {
        format!("id-{}", IDS.fetch_add(1, Ordering::SeqCst))
    }
}
pub fn store(path: &Path, financial: &Path, limits: ConversationLimits) -> SqliteApplicationStore {
    SqliteApplicationStore::open_with_conversation_limits(
        path,
        Box::new(SqliteRepository::open(financial).unwrap()),
        Limits::default(),
        limits,
        Box::new(TestClock),
        Box::new(TestIds),
    )
    .unwrap()
}
pub fn page() -> PageRequest {
    PageRequest {
        offset: 0,
        limit: 10,
    }
}
pub fn request(c: &Conversation, id: &str) -> SendMessageRequest {
    SendMessageRequest {
        research_brief: None,
        company_hint: None,
        conversation_id: c.id.clone(),
        request_id: id.into(),
        text: "Research Apple".into(),
        selected: vec![],
    }
}
pub fn legacy_store(path: &Path, financial: &Path) -> SqliteApplicationStore {
    SqliteApplicationStore::open(
        path,
        Box::new(SqliteRepository::open(financial).unwrap()),
        Limits::default(),
        Box::new(TestClock),
        Box::new(TestIds),
    )
    .unwrap()
}
