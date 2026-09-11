mod support;
use desktop_host::Bridge;
use lugus_app::{conversations::*, *};
use serde_json::json;
use support::*;

#[tokio::test]
async fn application_prepares_views_followup_freezes_selection_and_restart_restores_evidence() {
    let factory = Factory::default();
    let prompts = factory.prompts.clone();
    let (root, bridge, host) = setup(factory).await;
    let c = dispatch(
        &bridge,
        json!({"op":"create","request":"one","title":"Company research"}),
    )
    .await;
    let conversation = c["id"].as_str().unwrap();
    let command = json!({"op":"send","conversation":conversation,"request":"turn-one","text":"Research the company","company_hint":"Apple / AAPL"});
    let run = dispatch(&bridge, command.clone()).await;
    let done = host
        .wait(conversation, run["id"].as_str().unwrap())
        .await
        .unwrap();
    assert_eq!(done.status, RunStatus::Completed, "{:?}", done.error);
    assert_eq!(done.company_hint.as_deref(), Some("Apple / AAPL"));
    let mut changed_hint = command.clone();
    changed_hint["company_hint"] = json!("Microsoft / MSFT");
    assert_eq!(
        bridge
            .dispatch(&changed_hint.to_string())
            .await
            .unwrap_err()
            .kind,
        ErrorKind::Conflict
    );
    assert_eq!(dispatch(&bridge, command).await["id"], run["id"]);
    assert!(run.get("input").is_none());
    let workspace = dispatch(
        &bridge,
        json!({"op":"workspace","conversation":conversation}),
    )
    .await;
    assert_eq!(workspace["view_ids"].as_array().unwrap().len(), 2);
    let view = &workspace["view_ids"][1];
    let facts = &workspace["view_ids"][0];
    let prices = dispatch(
        &bridge,
        json!({"op":"read","conversation":conversation,"view":view}),
    )
    .await;
    assert!(prices.to_string().contains("101"));
    assert_eq!(prices["rows"].as_array().unwrap().len(), 10);
    let binding = dispatch(&bridge, json!({"op":"binding","conversation":conversation,"binding":prices["header"]["binding_id"]})).await;
    assert!(binding.to_string().contains("Apple Inc."));

    let data = dispatch(
        &bridge,
        json!({"op":"read","conversation":conversation,"view":facts}),
    )
    .await;
    assert!(data.to_string().contains("12345678901234567890.001"));
    let other = dispatch(
        &bridge,
        json!({"op":"create","request":"two","title":"Other"}),
    )
    .await;
    for op in ["view", "read"] {
        assert_eq!(
            bridge
                .dispatch(&json!({"op":op,"conversation":other["id"],"view":view}).to_string())
                .await
                .unwrap_err()
                .kind,
            ErrorKind::ScopeMismatch
        );
    }
    assert_eq!(bridge.dispatch(&json!({"op":"send","conversation":other["id"],"request":"wrong-scope","text":"Read this","selected":[{"kind":"view","id":view}]}).to_string()).await.unwrap_err().kind, ErrorKind::ScopeMismatch);
    let selected = dispatch(&bridge, json!({"op":"select","conversation":conversation,"view":facts,"revision":workspace["revision"]})).await;
    assert_eq!(selected["selected_view_id"], *facts);
    assert_eq!(bridge.dispatch(&json!({"op":"select","conversation":conversation,"view":view,"revision":workspace["revision"]}).to_string()).await.unwrap_err().kind, ErrorKind::Conflict);
    let receipt = dispatch(
        &bridge,
        json!({"op":"view","conversation":conversation,"view":view}),
    )
    .await;
    dispatch(&bridge, json!({"op":"presented","conversation":conversation,"view":view,"revision":receipt["descriptor_revision"],"status":"presented"})).await;
    assert_eq!(
        dispatch(
            &bridge,
            json!({"op":"view","conversation":conversation,"view":view})
        )
        .await["presentation"],
        "presented"
    );
    let activity = dispatch(
        &bridge,
        json!({"op":"activity","conversation":conversation,"run":run["id"]}),
    )
    .await;
    assert!(!activity["items"].as_array().unwrap().is_empty());
    let status = dispatch(
        &bridge,
        json!({"op":"status","conversation":conversation,"run":run["id"]}),
    )
    .await;
    assert_eq!(status["status"], "completed");
    assert!(status.get("input").is_none());
    let second = dispatch(&bridge, json!({"op":"send","conversation":conversation,"request":"turn-two","text":"Explain this price","selected":[{"kind":"view","id":view}]})).await;
    assert_eq!(
        host.wait(conversation, second["id"].as_str().unwrap())
            .await
            .unwrap()
            .status,
        RunStatus::Completed
    );
    let prompts = prompts.lock().unwrap().clone();
    assert_eq!(prompts.len(), 2);
    assert!(prompts[1].contains("The saved price and reported facts"));
    assert!(prompts[1].contains(view.as_str().unwrap()));
    let messages = dispatch(
        &bridge,
        json!({"op":"messages","conversation":conversation}),
    )
    .await;
    assert_eq!(messages["items"].as_array().unwrap().len(), 4);
    bridge.shutdown().await.unwrap();
    let restarted = Bridge::open(&root.path().join("desktop.json"), true)
        .await
        .unwrap();
    assert_eq!(
        dispatch(
            &restarted,
            json!({"op":"messages","conversation":conversation})
        )
        .await,
        messages
    );
    assert_eq!(
        dispatch(
            &restarted,
            json!({"op":"read","conversation":conversation,"view":view})
        )
        .await,
        prices
    );
    let restored_workspace = dispatch(
        &restarted,
        json!({"op":"workspace","conversation":conversation}),
    )
    .await;
    assert_eq!(dispatch(&restarted, json!({"op":"select","conversation":conversation,"view":view,"revision":restored_workspace["revision"]})).await["selected_view_id"], *view);
    dispatch(&restarted, json!({"op":"presented","conversation":conversation,"view":view,"revision":receipt["descriptor_revision"],"status":"presented"})).await;
    restarted.shutdown().await.unwrap();
}
#[tokio::test]
async fn cancellation_and_runtime_failure_have_durable_terminal_status() {
    for (hold, fail, expected) in [
        (true, false, RunStatus::Interrupted),
        (false, true, RunStatus::Failed),
    ] {
        let (_root, bridge, host) = setup(Factory {
            hold,
            fail,
            ..Default::default()
        })
        .await;
        let c = dispatch(&bridge, json!({"op":"create","request":"c","title":"Test"})).await;
        let conversation = c["id"].as_str().unwrap();
        let run = dispatch(
            &bridge,
            json!({"op":"send","conversation":conversation,"request":"r","text":"test"}),
        )
        .await;
        if hold {
            dispatch(
                &bridge,
                json!({"op":"cancel","conversation":conversation,"run":run["id"]}),
            )
            .await;
        }
        assert_eq!(
            host.wait(conversation, run["id"].as_str().unwrap())
                .await
                .unwrap()
                .status,
            expected
        );
        bridge.shutdown().await.unwrap();
    }
}

#[tokio::test]
async fn oversized_aggregate_run_history_pages_offline_without_losing_rows() {
    let (root, bridge, host) = setup(Factory {
        fail: true,
        ..Default::default()
    })
    .await;
    let c = dispatch(
        &bridge,
        json!({"op":"create","request":"large","title":"Long research"}),
    )
    .await;
    let conversation = c["id"].as_str().unwrap();
    let mut ids = Vec::new();
    let mut total_bytes = 0;
    for n in 0..20 {
        let run = dispatch(&bridge, json!({"op":"send","conversation":conversation,"request":format!("turn-{n}"),"text":"x".repeat(8192)})).await;
        let record = host
            .wait(conversation, run["id"].as_str().unwrap())
            .await
            .unwrap();
        total_bytes += serde_json::to_vec(&record).unwrap().len();
        ids.push(run["id"].clone());
    }
    assert!(
        total_bytes > 1024 * 1024,
        "test must exceed aggregate host page budget"
    );
    assert_eq!(
        host.runs(
            conversation,
            PageRequest {
                offset: 0,
                limit: 100
            }
        )
        .await
        .unwrap_err()
        .kind,
        ErrorKind::ResourceLimit
    );
    bridge.shutdown().await.unwrap();
    let reopened = Bridge::open(&root.path().join("desktop.json"), true)
        .await
        .unwrap();
    let mut offset = 0;
    let mut restored = Vec::new();
    loop {
        let page = dispatch(
            &reopened,
            json!({"op":"runs","conversation":conversation,"offset":offset}),
        )
        .await;
        let rows = page["items"].as_array().unwrap();
        assert!(!rows.is_empty());
        assert!(rows.iter().all(|r| r.get("input").is_none()));
        restored.extend(rows.iter().map(|r| r["id"].clone()));
        let Some(next) = page["next_offset"].as_u64() else {
            break;
        };
        assert_eq!(next as usize, offset + rows.len());
        offset = next as usize;
    }
    assert_eq!(restored, ids);
    reopened.shutdown().await.unwrap();
}
