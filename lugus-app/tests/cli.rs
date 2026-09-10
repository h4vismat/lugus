use serde_json::{Value, json};
use std::process::Command;
fn cli(args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
#[test]
fn cli_runs_manual_and_real_thesis_backed_agent_then_reopens_offline() {
    let root = tempfile::tempdir().unwrap();
    let p = |name: &str| root.path().join(name).to_str().unwrap().to_string();
    std::fs::write(p("manifest.json"), json!({"id":"worker-fixture","version":"1","protocol_version":1,"command":"python3","args":[format!("{}/tests/fixtures/worker.py",env!("CARGO_MANIFEST_DIR"))]}).to_string()).unwrap();
    let providers: Vec<_> = [("one","ok"),("two","ok"),("bad","startup_failure")].iter().map(|(id,mode)|json!({"instance_id":id,"manifest":"manifest.json","active":true,"config":{"mode":mode,"barrier":p(id)}})).collect();
    std::fs::write(p("config.json"), json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":providers}).to_string()).unwrap();
    let query = json!({"company":{"namespace":"sec:cik","value":"0000320193"},"filed_from":"2024-01-01","filed_to":"2024-12-31","forms":[],"page_size":10,"cursor":null});
    for id in ["one", "two", "bad"] {
        std::fs::write(
            p(&format!("{id}.json")),
            json!({"operation":"filings","instance_id":id,"query":query}).to_string(),
        )
        .unwrap();
    }
    // The market instrument is supplied explicitly; no company-to-market mapping is inferred.
    std::fs::write(p("two.json"), json!({"operation":"prices","instance_id":"two","query":{"instrument":{"namespace":"fixture:symbol","value":"EXPLICIT"},"start":"2024-01-01","end":"2024-01-03","page_size":10,"cursor":null}}).to_string()).unwrap();
    let manual = cli(&[
        "fetch",
        &p("config.json"),
        "workspace",
        "manual",
        &p("one.json"),
    ]);
    assert_eq!(manual["state"], "succeeded");
    let unavailable = cli(&[
        "fetch",
        &p("config.json"),
        "workspace",
        "unavailable",
        &p("bad.json"),
    ]);
    assert_eq!(unavailable["error"]["kind"], "unavailable");
    std::fs::write(
        p("thesis.txt"),
        "Fixture thesis: inspect exact provider filing evidence before accepting a table view.",
    )
    .unwrap();
    let thesis = cli(&[
        "thesis-create",
        &p("thesis.sqlite"),
        "stored-thesis",
        &p("thesis.txt"),
    ]);
    assert_eq!(thesis["revision"], 1);
    let agent = cli(&[
        "agent-fixture",
        &p("config.json"),
        "workspace",
        &p("thesis.sqlite"),
        "stored-thesis",
        &p("two.json"),
    ]);
    assert_eq!(agent["thesis"]["thesis_id"], "stored-thesis");
    assert_eq!(agent["report"]["outcome"], "completed");
    assert_eq!(agent["receipts"]["fetch"]["provider"]["instance_id"], "two");
    assert!(agent["receipts"]["view"]["presentation"].is_null());
    assert_eq!(agent["receipts"]["view"]["kind"], "price_chart");
    assert_eq!(agent["receipts"]["page"]["rows"][0]["value"], "101");
    assert_eq!(
        agent["receipts"]["fetch"]["command"]["query"]["instrument"]["value"],
        "EXPLICIT"
    );
    assert_eq!(
        agent["receipts"]["page"]["rows"].as_array().unwrap().len(),
        1
    );
    let dataset = agent["receipts"]["dataset"]["id"].as_str().unwrap();
    // No process manifest exists in this fresh configuration; only durable stores remain.
    std::fs::write(p("offline.json"),json!({"financial_path":"financial.sqlite","application_path":"application.sqlite","providers":[]}).to_string()).unwrap();
    std::fs::remove_file(p("manifest.json")).unwrap();
    let page = cli(&[
        "read",
        &p("offline.json"),
        "workspace",
        "offline",
        dataset,
        "0",
        "1",
    ]);
    assert_eq!(page["header"]["provider"]["instance_id"], "two");
    assert_eq!(page["rows"].as_array().unwrap().len(), 1);
    let fetch = cli(&[
        "read-fetch",
        &p("offline.json"),
        "workspace",
        "offline",
        manual["fetch_id"].as_str().unwrap(),
    ]);
    assert_eq!(fetch["provider"]["instance_id"], "one");
    let missing = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args([
            "agent-fixture",
            &p("offline.json"),
            "workspace",
            &p("thesis.sqlite"),
            "missing-thesis",
            &p("two.json"),
        ])
        .output()
        .unwrap();
    assert!(!missing.status.success());
}

#[test]
fn cli_automatically_binds_from_real_thesis_and_refuses_incompatible_or_ambiguous_evidence() {
    let root = tempfile::tempdir().unwrap();
    let p = |name: &str| root.path().join(name).to_str().unwrap().to_string();
    let setup = Command::new("python3")
        .args([
            &format!("{}/examples/synthetic/setup.py", env!("CARGO_MANIFEST_DIR")),
            root.path().to_str().unwrap(),
        ])
        .output()
        .unwrap();
    assert!(setup.status.success());
    cli(&[
        "thesis-create",
        &p("thesis.sqlite"),
        "binding-thesis",
        &p("thesis.txt"),
    ]);
    let agent = cli(&[
        "agent-fixture",
        &p("config.json"),
        "workspace",
        &p("thesis.sqlite"),
        "binding-thesis",
        &p("binding.json"),
    ]);
    let r = &agent["receipts"];
    assert_eq!(r["binding"]["policy"], "instrument-binding-v1");
    assert_eq!(r["dataset"]["binding_id"], r["binding"]["id"]);
    assert_eq!(r["fetch"]["binding_id"], r["binding"]["id"]);
    assert_eq!(r["fetch"]["provider"]["instance_id"], "market");
    assert_eq!(
        r["fetch"]["command"]["query"]["instrument"]["value"],
        "AAPL"
    );
    assert_eq!(r["view"]["kind"], "price_chart");
    for config in [
        "wrong-issuer.json",
        "wrong-exchange.json",
        "missing-evidence.json",
        "multiple-listings.json",
    ] {
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([
                "agent-fixture",
                &p(config),
                "workspace",
                &p("thesis.sqlite"),
                "binding-thesis",
                &p("binding.json"),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{config}");
    }
    std::fs::remove_file(p("manifest.json")).unwrap();
    let id = r["binding"]["id"].as_str().unwrap();
    let b = cli(&[
        "read-binding",
        &p("offline.json"),
        "workspace",
        "offline",
        id,
    ]);
    assert_eq!(b["record"]["id"], id);
    let history = cli(&[
        "binding-history",
        &p("offline.json"),
        "workspace",
        "history",
        id,
        "0",
        "10",
    ]);
    assert_eq!(history["events"].as_array().unwrap().len(), 1);
    let page = cli(&[
        "read",
        &p("offline.json"),
        "workspace",
        "page",
        r["dataset"]["id"].as_str().unwrap(),
        "0",
        "1",
    ]);
    assert_eq!(page["header"]["binding_id"], id);
}
