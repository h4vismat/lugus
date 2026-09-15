#[allow(dead_code)]
mod conversations_support;
use conversations_support::store;
use lugus_app::{PageRequest, conversations::ConversationLimits, portfolio::*};
use serde_json::json;
#[test]
fn history_admission_is_idempotent_exclusive_and_revision_bound() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = store(&db, &fin, ConversationLimits::default());
    let p=s.portfolio_execute(&serde_json::from_value(json!({"request_id":"create","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}})).unwrap()).unwrap().portfolio_id;
    let request:PortfolioHistoryRequest=serde_json::from_value(json!({"request_id":"history","portfolio_id":p,"account_id":null,"expected_revision":"1","range":{"start":"2026-01-02","end":"2026-01-05"},"refresh":"force"})).unwrap();
    let doc = s.portfolio_document(&p).unwrap();
    let key = HistoryKey::new(&doc, &request, None).unwrap();
    let lease = s.portfolio_history_acquire(&p).unwrap();
    assert!(s.portfolio_history_acquire(&p).is_err());
    let (mut first, started) = s.portfolio_history_begin(&request, &key, &lease).unwrap();
    assert!(started);
    let (retry, started) = s.portfolio_history_begin(&request, &key, &lease).unwrap();
    assert!(!started);
    assert_eq!(first.id, retry.id);
    assert!(s.portfolio_history_read("wrong", &first.id).is_err());
    let point=serde_json::from_value(json!({"date":"2026-01-01","value":"0","deposits":"0","withdrawals":"0","opening_contribution":"0","portfolio_growth":null,"portfolio_return_percent":null,"segment_return_percent":null,"segment":0,"benchmark_return_percent":null,"hypothetical_value":null,"issues":[]})).unwrap();
    s.portfolio_history_save_rows(&first.id, 0, &[point])
        .unwrap();
    assert!(
        s.portfolio_history_page(
            &p,
            &first.id,
            PageRequest {
                offset: 0,
                limit: 2
            }
        )
        .is_err()
    );
    s.portfolio_execute(&serde_json::from_value(json!({"request_id":"rename","portfolio_id":p,"expected_revision":"1","mutation":{"kind":"rename_portfolio","name":"Changed"}})).unwrap()).unwrap();
    first.status = HistoryStatus::Partial;
    first.row_count = 1;
    let saved = s.portfolio_history_finish(&first).unwrap();
    assert_eq!(saved.status, HistoryStatus::StaleRevision);
    drop(lease);
    drop(s);
    let s = store(&db, &fin, ConversationLimits::default());
    assert_eq!(
        s.portfolio_history_read(&p, &first.id)
            .unwrap()
            .key
            .revision,
        1
    );
}
#[test]
fn a_new_owner_recovers_interrupted_jobs_without_touching_live_owners() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app");
    let fin = dir.path().join("fin");
    let mut s = store(&db, &fin, ConversationLimits::default());
    let p=s.portfolio_execute(&serde_json::from_value(json!({"request_id":"p","portfolio_id":null,"expected_revision":"0","mutation":{"kind":"create_portfolio","name":"P"}})).unwrap()).unwrap().portfolio_id;
    let mut request:PortfolioHistoryRequest=serde_json::from_value(json!({"request_id":"first","portfolio_id":p,"account_id":null,"expected_revision":"1","range":{"start":"2026-01-02","end":"2026-01-05"},"refresh":"force"})).unwrap();
    let key = HistoryKey::new(&s.portfolio_document(&p).unwrap(), &request, None).unwrap();
    let lease = s.portfolio_history_acquire(&p).unwrap();
    let (old, _) = s.portfolio_history_begin(&request, &key, &lease).unwrap();
    let mut other = store(&db, &fin, ConversationLimits::default());
    assert!(other.portfolio_history_acquire(&p).is_err());
    drop(lease);
    let lease = other.portfolio_history_acquire(&p).unwrap();
    request.request_id = "second".into();
    let (new, started) = other
        .portfolio_history_begin(&request, &key, &lease)
        .unwrap();
    assert!(started);
    assert_ne!(old.id, new.id);
    assert_eq!(
        other.portfolio_history_read(&p, &old.id).unwrap().status,
        HistoryStatus::Interrupted
    );
}
