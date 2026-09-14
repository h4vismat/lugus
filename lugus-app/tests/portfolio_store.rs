#[allow(dead_code)]
mod conversations_support;
use conversations_support::store;
use lugus_app::{ErrorKind, conversations::ConversationLimits, portfolio::*};
use serde_json::{Value, json};
fn command(request: &str, p: Option<&str>, revision: u64, mutation: Value) -> PortfolioCommand {
    serde_json::from_value(json!({"request_id":request,"portfolio_id":p,"expected_revision":revision.to_string(),"mutation":mutation})).unwrap()
}
#[test]
fn atomic_writes_replay_and_preserve_history() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("app.db");
    let fin = dir.path().join("fin.db");
    let mut s = store(&db, &fin, ConversationLimits::default());
    let create = command(
        "create",
        None,
        0,
        json!({"kind":"create_portfolio","name":"Investments"}),
    );
    let receipt = s.portfolio_execute(&create).unwrap();
    let p = receipt.portfolio_id.clone();
    assert_eq!(s.portfolio_execute(&create).unwrap(), receipt);
    let a = command(
        "account",
        Some(&p),
        1,
        json!({"kind":"create_account","name":"Broker","start":"2026-01-01","opening":{"kind":"full_history"},"events":[{"id":"deposit","date":"2026-01-01","order":0,"kind":{"kind":"deposit","amount":"200"}}]}),
    );
    let ar = s.portfolio_execute(&a).unwrap();
    let account = ar.account_ids[0].clone();
    let bad = command(
        "bad",
        Some(&p),
        2,
        json!({"kind":"edit_events","account_id":account,"edits":[{"kind":"append","event":{"id":"withdraw","date":"2026-01-01","order":1,"kind":{"kind":"withdrawal","amount":"201"}}}]}),
    );
    assert_eq!(
        s.portfolio_execute(&bad).unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    let view = s.portfolio_overview(&p, None).unwrap();
    assert_eq!(view.revision, 2);
    assert_eq!(view.valuation.cash.to_string(), "200");
    assert_eq!(
        s.portfolio_execute(&command(
            "stale",
            Some(&p),
            1,
            json!({"kind":"rename_portfolio","name":"Changed"})
        ))
        .unwrap_err()
        .kind,
        ErrorKind::Conflict
    );
    drop(s);
    let s = store(&db, &fin, ConversationLimits::default());
    assert_eq!(
        s.portfolio_overview(&p, None)
            .unwrap()
            .valuation
            .cash
            .to_string(),
        "200"
    );
}
#[test]
fn snapshot_is_immutable_and_scoped() {
    use lugus_app::conversations::ConversationStore;
    let dir = tempfile::tempdir().unwrap();
    let mut s = store(
        &dir.path().join("app"),
        &dir.path().join("fin"),
        ConversationLimits::default(),
    );
    let c = s.create_conversation("c", "Chat").unwrap();
    let p = s
        .portfolio_execute(&command(
            "p",
            None,
            0,
            json!({"kind":"create_portfolio","name":"P"}),
        ))
        .unwrap()
        .portfolio_id;
    let snap = s
        .portfolio_snapshot(&SnapshotRequest {
            request_id: "snap".into(),
            portfolio_id: p.clone(),
            account_id: None,
            expected_revision: 1,
            conversation_id: c.id.clone(),
        })
        .unwrap();
    s.portfolio_execute(&command(
        "rename",
        Some(&p),
        1,
        json!({"kind":"rename_portfolio","name":"New"}),
    ))
    .unwrap();
    assert_eq!(s.portfolio_read_snapshot(&c.id, &snap.id).unwrap(), snap);
    assert_eq!(
        s.portfolio_read_snapshot("wrong", &snap.id)
            .unwrap_err()
            .kind,
        ErrorKind::ScopeMismatch
    );
}
#[test]
fn selected_portfolio_reference_freezes_validated_evidence() {
    use lugus_app::conversations::*;
    let dir = tempfile::tempdir().unwrap();
    let mut s = store(
        &dir.path().join("app"),
        &dir.path().join("fin"),
        ConversationLimits::default(),
    );
    let c = s.create_conversation("c", "Chat").unwrap();
    let p = s
        .portfolio_execute(&command(
            "p",
            None,
            0,
            json!({"kind":"create_portfolio","name":"P"}),
        ))
        .unwrap()
        .portfolio_id;
    let snap = s
        .portfolio_snapshot(&SnapshotRequest {
            request_id: "snap".into(),
            portfolio_id: p,
            account_id: None,
            expected_revision: 1,
            conversation_id: c.id,
        })
        .unwrap();
    let frozen = FrozenReference::from_portfolio(&snap, &ConversationLimits::default()).unwrap();
    frozen.validate(&ConversationLimits::default()).unwrap();
    assert_eq!(
        frozen.reference,
        SelectedReference::Portfolio { id: snap.id }
    );
}
#[test]
fn a_voided_event_identifier_cannot_be_reused_as_a_new_transaction() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = store(
        &dir.path().join("app"),
        &dir.path().join("fin"),
        ConversationLimits::default(),
    );
    let p = s
        .portfolio_execute(&command(
            "p",
            None,
            0,
            json!({"kind":"create_portfolio","name":"P"}),
        ))
        .unwrap()
        .portfolio_id;
    let event =
        json!({"id":"d","date":"2026-01-01","order":0,"kind":{"kind":"deposit","amount":"10"}});
    let a=s.portfolio_execute(&command("a",Some(&p),1,json!({"kind":"create_account","name":"A","start":"2026-01-01","opening":{"kind":"full_history"},"events":[event]}))).unwrap().account_ids[0].clone();
    s.portfolio_execute(&command(
        "void",
        Some(&p),
        2,
        json!({"kind":"edit_events","account_id":a,"edits":[{"kind":"void","id":"d"}]}),
    ))
    .unwrap();
    let result = s.portfolio_execute(&command(
        "reuse",
        Some(&p),
        3,
        json!({"kind":"edit_events","account_id":a,"edits":[{"kind":"append","event":event}]}),
    ));
    assert!(result.is_err());
}
#[test]
fn split_corrections_change_all_linked_accounts_atomically() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = store(
        &dir.path().join("app"),
        &dir.path().join("fin"),
        ConversationLimits::default(),
    );
    let p = s
        .portfolio_execute(&command(
            "p",
            None,
            0,
            json!({"kind":"create_portfolio","name":"P"}),
        ))
        .unwrap()
        .portfolio_id;
    let i = s
        .portfolio_execute(&command(
            "i",
            Some(&p),
            1,
            json!({"kind":"create_instrument","name":"Stock","symbol":"TEST","asset_kind":"stock"}),
        ))
        .unwrap()
        .instrument_ids[0]
        .clone();
    for (n, revision) in [(1, 2), (2, 3)] {
        s.portfolio_execute(&command(&format!("account{n}"),Some(&p),revision,json!({"kind":"create_account","name":format!("Account{n}"),"start":"2026-01-01","opening":{"kind":"existing","cash":"0","lots":[{"id":format!("lot{n}"),"instrument_id":i,"acquired":"2025-01-01","tie_order":0,"quantity":"10","basis":"100","simplified":false,"date_assumed":false}]},"events":[]}))).unwrap();
    }
    s.portfolio_execute(&command("split",Some(&p),4,json!({"kind":"apply_split","instrument_id":i,"date":"2026-01-02","order":0,"numerator":2,"denominator":1}))).unwrap();
    let doc = s.portfolio_document(&p).unwrap();
    let action = match &doc.accounts[0].ledger.events[0].kind {
        lugus_portfolio::EventKind::Split { action_id, .. } => action_id.clone(),
        _ => panic!("split event"),
    };
    s.portfolio_execute(&command("correct",Some(&p),5,json!({"kind":"replace_split","action_id":action,"date":"2026-01-02","order":0,"numerator":3,"denominator":1}))).unwrap();
    let view = s.portfolio_overview(&p, None).unwrap();
    assert_eq!(view.valuation.holdings[0].quantity.to_string(), "60");
    assert_eq!(view.valuation.holdings[0].basis.to_string(), "200");
    s.portfolio_execute(&command(
        "void",
        Some(&p),
        6,
        json!({"kind":"void_split","action_id":action}),
    ))
    .unwrap();
    assert_eq!(
        s.portfolio_overview(&p, None).unwrap().valuation.holdings[0]
            .quantity
            .to_string(),
        "20"
    );
}
#[test]
fn selecting_an_account_does_not_include_other_accounts_instruments() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = store(
        &dir.path().join("app"),
        &dir.path().join("fin"),
        ConversationLimits::default(),
    );
    let p = s
        .portfolio_execute(&command(
            "p",
            None,
            0,
            json!({"kind":"create_portfolio","name":"P"}),
        ))
        .unwrap()
        .portfolio_id;
    s.portfolio_execute(&command("i",Some(&p),1,json!({"kind":"create_instrument","name":"Unrelated","symbol":"OTHER","asset_kind":"stock"}))).unwrap();
    let a=s.portfolio_execute(&command("a",Some(&p),2,json!({"kind":"create_account","name":"Cash only","start":"2026-01-01","opening":{"kind":"full_history"},"events":[]}))).unwrap().account_ids[0].clone();
    assert!(
        s.portfolio_overview(&p, Some(&a))
            .unwrap()
            .instruments
            .is_empty()
    );
}
