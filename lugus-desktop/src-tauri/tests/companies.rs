use desktop_host::Bridge;
use lugus_app::ErrorKind;
use serde_json::{Value, json};

async fn fixture() -> (tempfile::TempDir, Bridge) {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("application.json"), json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}).to_string()).unwrap();
    std::fs::write(
        dir.path().join("desktop.json"),
        r#"{"application_config":"application.json"}"#,
    )
    .unwrap();
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    (dir, bridge)
}
async fn call(bridge: &Bridge, command: Value) -> Value {
    bridge
        .dispatch(&json!({"op":"company","command":command}).to_string())
        .await
        .unwrap()
}

#[tokio::test]
async fn company_briefs_reopen_with_history_and_reject_stale_edits() {
    let (dir, bridge) = fixture().await;
    let a = call(
        &bridge,
        json!({"kind":"create","request":"apple","name":"Apple","hint":"AAPL"}),
    )
    .await;
    let replay = call(
        &bridge,
        json!({"kind":"create","request":"apple","name":"Apple","hint":"AAPL"}),
    )
    .await;
    assert_eq!(a, replay);
    let b = call(
        &bridge,
        json!({"kind":"create","request":"msft","name":"Microsoft","hint":"MSFT"}),
    )
    .await;
    assert_ne!(a["conversation_id"], b["conversation_id"]);
    let saved = call(&bridge, json!({"kind":"save","company":a["id"],"revision":a["revision"],"thesis":"Pricing power","questions":"Can margins hold?"})).await;
    assert_eq!(saved["revision"], 1);
    let stale = bridge.dispatch(&json!({"op":"company","command":{"kind":"save","company":a["id"],"revision":0,"thesis":"stale","questions":""}}).to_string()).await.unwrap_err();
    assert_eq!(stale.kind, ErrorKind::Conflict);
    bridge.shutdown().await.unwrap();
    drop(bridge);
    let bridge = Bridge::open(&dir.path().join("desktop.json"), true)
        .await
        .unwrap();
    let reopened = call(&bridge, json!({"kind":"get","company":a["id"]})).await;
    assert_eq!(reopened["thesis"], "Pricing power");
    assert_eq!(reopened["questions"], "Can margins hold?");
    let other = call(&bridge, json!({"kind":"get","company":b["id"]})).await;
    assert_eq!(other["thesis"], "");
    let history = call(&bridge, json!({"kind":"history","company":a["id"]})).await;
    assert_eq!(history["revisions"][0]["thesis"], "Pricing power");
    assert_eq!(history["revisions"][1]["thesis"], "");
    assert_eq!(
        call(&bridge, json!({"kind":"list"})).await["items"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn companies_validate_names_and_reject_unowned_finding_sources() {
    let (_dir, bridge) = fixture().await;
    for name in ["", "\nBad"] {
        assert!(bridge.dispatch(&json!({"op":"company","command":{"kind":"create","request":"bad","name":name,"hint":"AAPL"}}).to_string()).await.is_err());
    }
    let a = call(
        &bridge,
        json!({"kind":"create","request":"apple","name":"Apple","hint":"AAPL"}),
    )
    .await;
    let error = bridge.dispatch(&json!({"op":"company","command":{"kind":"finding","company":a["id"],"revision":0,"message":"not-owned","text":"Invented finding"}}).to_string()).await.unwrap_err();
    assert_eq!(error.kind, ErrorKind::MissingData);
    assert_eq!(
        call(&bridge, json!({"kind":"get","company":a["id"]})).await["findings"],
        json!([])
    );
    bridge.shutdown().await.unwrap();
}

mod support;
#[tokio::test]
async fn agent_receives_frozen_brief_and_reviews_and_findings_remain_owned() {
    let factory = support::Factory::default();
    let prompts = factory.prompts.clone();
    let (root, bridge, host) = support::setup(factory).await;
    let bridge = bridge
        .with_company_store(&root.path().join("companies.sqlite"))
        .unwrap();
    let a = call(
        &bridge,
        json!({"kind":"create","request":"apple","name":"Apple","hint":"AAPL"}),
    )
    .await;
    let b = call(
        &bridge,
        json!({"kind":"create","request":"other","name":"Other","hint":"OTHER"}),
    )
    .await;
    let saved = call(&bridge, json!({"kind":"save","company":a["id"],"revision":0,"thesis":"Pricing power","questions":"Can margins hold?"})).await;
    let request = json!({"kind":"send","company":a["id"],"request":"review","text":"Review my thesis","review":true});
    let run = call(&bridge, request.clone()).await;
    let conversation = a["conversation_id"].as_str().unwrap();
    assert_eq!(
        host.wait(conversation, run["id"].as_str().unwrap())
            .await
            .unwrap()
            .status,
        lugus_app::conversations::RunStatus::Completed
    );
    let prompt: Value = serde_json::from_str(&prompts.lock().unwrap()[0]).unwrap();
    let brief: Value = serde_json::from_str(prompt["research_brief"].as_str().unwrap()).unwrap();
    assert_eq!(brief["thesis"], "Pricing power");
    assert_eq!(brief["revision"], 1);
    assert_eq!(prompt["new_message"]["text"], "Review my thesis");
    let edited = call(&bridge, json!({"kind":"save","company":a["id"],"revision":saved["revision"],"thesis":"New thesis","questions":""})).await;
    assert_eq!(call(&bridge, request).await["id"], run["id"]);
    assert_eq!(prompts.lock().unwrap().len(), 1);
    let messages = host
        .messages(
            conversation,
            lugus_app::PageRequest {
                offset: 0,
                limit: 100,
            },
        )
        .await
        .unwrap();
    let answer = messages
        .items
        .iter()
        .find(|m| m.role == lugus_app::conversations::MessageRole::Assistant)
        .unwrap();
    let workspace = host.workspace(conversation).await.unwrap();
    let expected_views = workspace.view_ids.clone();
    for view_id in workspace.view_ids {
        let current = host.workspace(conversation).await.unwrap();
        host.mutate_workspace(
            conversation,
            current.revision,
            lugus_app::conversations::WorkspaceMutation::Close { view_id },
        )
        .await
        .unwrap();
    }
    let finding = json!({"kind":"finding","company":a["id"],"revision":edited["revision"],"message":answer.id,"text":"I should examine pricing power further."});
    let accepted = call(&bridge, finding).await;
    assert_eq!(accepted["findings"][0]["message_id"], answer.id);
    assert_eq!(accepted["thesis"], "New thesis");
    for view in expected_views {
        assert!(
            accepted["findings"][0]["evidence"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["id"] == view)
        );
    }
    rusqlite::Connection::open(root.path().join("companies.sqlite"))
        .unwrap()
        .execute("UPDATE company_sends SET run=NULL", [])
        .unwrap();
    assert!(bridge.dispatch(&json!({"op":"company","command":{"kind":"finding","company":b["id"],"revision":0,"message":answer.id,"text":"Wrong company"}}).to_string()).await.is_err());
    let history = call(&bridge, json!({"kind":"history","company":a["id"]})).await;
    assert_eq!(history["reviews"][0]["status"], "completed");
    assert_eq!(history["reviews"][0]["text"], answer.text);
    // Six interrupted-before-admission requests must not hide an older completed review.
    let db = rusqlite::Connection::open(root.path().join("companies.sqlite")).unwrap();
    let template: String = db
        .query_row("SELECT frozen FROM company_sends LIMIT 1", [], |r| r.get(0))
        .unwrap();
    for i in 0..6 {
        let mut frozen: Value = serde_json::from_str(&template).unwrap();
        frozen["request_id"] = json!(format!("pending-{i}"));
        db.execute(
            "INSERT INTO company_sends VALUES (?,?,?,?,1,NULL,?)",
            rusqlite::params![
                a["id"].as_str().unwrap(),
                format!("pending-{i}"),
                "{}",
                frozen.to_string(),
                "2026-09-15T12:00:00Z"
            ],
        )
        .unwrap();
    }
    let first_page = call(&bridge, json!({"kind":"history","company":a["id"]})).await;
    assert_eq!(first_page["next_offset"], 5);
    let second_page = call(
        &bridge,
        json!({"kind":"history","company":a["id"],"offset":5}),
    )
    .await;
    assert_eq!(second_page["reviews"][1]["status"], "completed");

    let second = bridge.dispatch(&json!({"op":"send","conversation":a["conversation_id"],"request":"followup","text":"Follow up"}).to_string()).await.unwrap();
    host.wait(conversation, second["id"].as_str().unwrap())
        .await
        .unwrap();
    let prompt: Value = serde_json::from_str(&prompts.lock().unwrap()[1]).unwrap();
    let brief: Value = serde_json::from_str(prompt["research_brief"].as_str().unwrap()).unwrap();
    assert_eq!(brief["previous_review"]["run_id"], run["id"]);
    assert_eq!(
        brief["accepted_findings"][0]["text"],
        "I should examine pricing power further."
    );
    bridge.shutdown().await.unwrap();
}

#[tokio::test]
async fn research_brief_has_its_own_budget_separate_from_message_text() {
    let factory = support::Factory::default();
    let (_root, bridge, host) = support::setup(factory).await;
    let c = host.create("budget", "Budget").await.unwrap();
    let run = host
        .send(lugus_app::conversations::SendMessageRequest {
            research_brief: Some("b".repeat(20_000)),
            company_hint: Some("AAPL".into()),
            conversation_id: c.id.clone(),
            request_id: "budget-run".into(),
            text: "Question".into(),
            selected: vec![],
        })
        .await
        .unwrap();
    assert_eq!(
        host.wait(&c.id, &run.id).await.unwrap().status,
        lugus_app::conversations::RunStatus::Completed
    );
    bridge.shutdown().await.unwrap();
}
