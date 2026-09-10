use serde_json::{Value, json};
use std::process::Command;

struct Fixture {
    root: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        let output = Command::new("python3")
            .args([
                concat!(env!("CARGO_MANIFEST_DIR"), "/examples/synthetic/setup.py"),
                root.path().to_str().unwrap(),
            ])
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        Self { root }
    }

    fn p(&self, name: &str) -> String {
        self.root.path().join(name).to_str().unwrap().into()
    }

    fn write(&self, name: &str, value: Value) {
        std::fs::write(self.p(name), value.to_string()).unwrap();
    }

    fn legacy(
        &self,
        command: &str,
        config: &str,
        workspace: &str,
        request: &str,
        args: &[&str],
    ) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([command, &self.p(config), workspace, request])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(&output.stdout).unwrap()
    }

    fn conversation(&self, command: &str, config: &str, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args(["conversation", command, &self.p(config)])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "conversation {command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(
            output
                .stdout
                .split(|byte| *byte == b'\n')
                .rfind(|line| !line.is_empty())
                .unwrap(),
        )
        .unwrap()
    }

    fn counts(&self) -> Vec<i64> {
        let database = rusqlite::Connection::open(self.p("application.sqlite")).unwrap();
        [
            "text_representations",
            "passage_requests",
            "conversation_runs",
            "conversation_messages",
            "conversation_tools",
        ]
        .iter()
        .map(|table| {
            database
                .query_row(&format!("SELECT count(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap()
        })
        .collect()
    }

    fn legacy_refusal(&self, command: &str, workspace: &str, request: &str, args: &[&str]) {
        let before = self.counts();
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([command, &self.p("offline.json"), workspace, request])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success(), "{command} unexpectedly succeeded");
        assert_eq!(before, self.counts(), "refused passage command wrote rows");
    }

    fn conversation_refusal(&self, request: &str, fixture: &str) {
        let before = self.counts();
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([
                "conversation",
                "send",
                &self.p("offline.json"),
                &self.p(request),
                &self.p(fixture),
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "foreign passage was admitted");
        assert_eq!(before, self.counts(), "refused turn wrote durable rows");
    }
}

fn id(value: &Value) -> &str {
    value["id"].as_str().unwrap()
}

#[test]
fn actual_html_passage_conversation_survives_revision_tab_close_and_provider_removal() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(
        "create",
        "offline.json",
        &[&fixture.p("conversation-create.json")],
    );
    let conversation_id = id(&conversation);
    let workspace_id = conversation["workspace_id"].as_str().unwrap();

    let original_fetch = fixture.legacy(
        "fetch",
        "passage-html.json",
        workspace_id,
        "fetch-original",
        &[&fixture.p("passage-document.json")],
    );
    assert_eq!(original_fetch["state"], "succeeded");
    let original_dataset = fixture.legacy(
        "dataset",
        "offline.json",
        workspace_id,
        "dataset-original",
        &[
            original_fetch["fetch_id"].as_str().unwrap(),
            &fixture.p("document-projection.json"),
        ],
    );
    let original_view = fixture.legacy(
        "view",
        "offline.json",
        workspace_id,
        "view-original",
        &[id(&original_dataset), "document"],
    );
    let original_header = fixture.legacy(
        "prepare-text",
        "offline.json",
        workspace_id,
        "prepare-original",
        &[id(&original_dataset)],
    );
    assert_eq!(original_header["decoder"], "utf-8");
    assert_eq!(
        fixture.legacy(
            "text-header",
            "offline.json",
            workspace_id,
            "header-original",
            &[id(&original_header)],
        )["id"],
        original_header["id"]
    );
    let original_text = fixture.legacy(
        "read-text",
        "offline.json",
        workspace_id,
        "read-original",
        &[
            id(&original_header),
            "0",
            original_header["text_bytes"]
                .as_u64()
                .unwrap()
                .to_string()
                .as_str(),
        ],
    );
    let quote = "Revenue & cash grew.";
    let start = original_text["text"].as_str().unwrap().find(quote).unwrap();
    let end = start + quote.len();
    fixture.write(
        "create-passage.json",
        json!({
            "representation_id": id(&original_header),
            "start": start,
            "end": end,
            "expected_text": quote,
        }),
    );
    let original_passage = fixture.legacy(
        "create-passage",
        "offline.json",
        workspace_id,
        "passage-original",
        &[&fixture.p("create-passage.json")],
    );
    assert_eq!(original_passage["quote"], quote);
    let original_checksum = original_passage["representation"]["document"]["checksum"]
        .as_str()
        .unwrap();
    let source = fixture.legacy(
        "resolve-passage",
        "offline.json",
        workspace_id,
        "resolve-original",
        &[id(&original_passage)],
    );
    assert_eq!(source["passage"]["quote"], quote);
    assert_eq!(
        source["passage"]["representation"]["document"]["checksum"],
        original_checksum
    );
    assert_eq!(
        source["passage"]["mappings"],
        json!([
            {"start":13,"end":20,"kind":"exact","source":{"node_id":13,"start":0,"end":7}},
            {"start":20,"end":21,"kind":"normalized","source":{"node_id":13,"start":7,"end":9}},
            {"start":21,"end":22,"kind":"exact","source":{"node_id":13,"start":9,"end":10}},
            {"start":22,"end":23,"kind":"normalized","source":{"node_id":13,"start":10,"end":12}},
            {"start":23,"end":28,"kind":"exact","source":{"node_id":13,"start":12,"end":17}},
            {"start":28,"end":32,"kind":"exact","source":{"node_id":15,"start":0,"end":4}},
            {"start":32,"end":33,"kind":"exact","source":{"node_id":16,"start":0,"end":1}},
        ])
    );
    assert_eq!(
        source["sources"],
        json!([
            {"node_id":13,"path":[1,1,1,0],"start":0,"end":7,"text":"Revenue"},
            {"node_id":13,"path":[1,1,1,0],"start":7,"end":9,"text":"  "},
            {"node_id":13,"path":[1,1,1,0],"start":9,"end":10,"text":"&"},
            {"node_id":13,"path":[1,1,1,0],"start":10,"end":12,"text":"\n "},
            {"node_id":13,"path":[1,1,1,0],"start":12,"end":17,"text":"cash "},
            {"node_id":15,"path":[1,1,1,1,0],"start":0,"end":4,"text":"grew"},
            {"node_id":16,"path":[1,1,1,2],"start":0,"end":1,"text":"."},
        ])
    );

    fixture.write(
        "passage-send.json",
        json!({
            "conversation_id": conversation_id,
            "request_id": "ask-original",
            "text": "What does this exact filing passage say?",
            "selected": [{"kind":"passage", "id":id(&original_passage)}],
        }),
    );
    fixture.write(
        "passage-runtime.json",
        json!({
            "mode":"passage",
            "expected_passage_id":id(&original_passage),
            "expected_quote":quote,
            "expected_document_checksum":original_checksum,
        }),
    );
    let completed = fixture.conversation(
        "send",
        "offline.json",
        &[
            &fixture.p("passage-send.json"),
            &fixture.p("passage-runtime.json"),
        ],
    );
    assert_eq!(completed["run"]["status"], "completed");
    let run_id = completed["run"]["id"].as_str().unwrap();
    let frozen_context =
        fixture.conversation("context", "offline.json", &[conversation_id, run_id]);
    let tools = fixture.conversation(
        "tools",
        "offline.json",
        &[conversation_id, run_id, "0", "10"],
    );
    assert_eq!(tools["items"].as_array().unwrap().len(), 1);
    assert_eq!(tools["items"][0]["intent"]["name"], "lugus_resolve_passage");
    let runtime_source: Value = serde_json::from_str(
        tools["items"][0]["outcome"]["result"]["content"]
            .as_str()
            .unwrap(),
    )
    .unwrap();
    assert_eq!(runtime_source, source);
    let messages = fixture.conversation("messages", "offline.json", &[conversation_id, "0", "10"]);
    let answer: Value =
        serde_json::from_str(messages["items"][1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer["receipts"]["passage"]["quote"], quote);
    assert_eq!(
        answer["receipts"]["passage"]["document_checksum"],
        original_checksum
    );
    assert!(
        answer["receipts"]["source"]["source_count"]
            .as_u64()
            .unwrap()
            >= 3
    );

    let revised_fetch = fixture.legacy(
        "fetch",
        "passage-revised.json",
        workspace_id,
        "fetch-revised",
        &[&fixture.p("passage-document.json")],
    );
    assert_eq!(revised_fetch["state"], "succeeded");
    let revised_dataset = fixture.legacy(
        "dataset",
        "offline.json",
        workspace_id,
        "dataset-revised",
        &[
            revised_fetch["fetch_id"].as_str().unwrap(),
            &fixture.p("document-projection.json"),
        ],
    );
    let revised_header = fixture.legacy(
        "prepare-text",
        "offline.json",
        workspace_id,
        "prepare-revised",
        &[id(&revised_dataset)],
    );
    assert_ne!(id(&revised_header), id(&original_header));
    assert_ne!(
        revised_header["document"]["checksum"],
        original_header["document"]["checksum"]
    );
    assert_eq!(
        revised_header["document"]["source_url"],
        original_header["document"]["source_url"]
    );
    fixture.write(
        "stale-passage.json",
        json!({
            "representation_id":id(&revised_header),
            "start":start,
            "end":end,
            "expected_text":quote,
        }),
    );
    fixture.legacy_refusal(
        "create-passage",
        workspace_id,
        "stale-passage",
        &[&fixture.p("stale-passage.json")],
    );
    let revised_text = fixture.legacy(
        "read-text",
        "offline.json",
        workspace_id,
        "read-revised",
        &[
            id(&revised_header),
            "0",
            revised_header["text_bytes"]
                .as_u64()
                .unwrap()
                .to_string()
                .as_str(),
        ],
    );
    let revised_quote = "Revenue & cash fell sharply.";
    let revised_start = revised_text["text"]
        .as_str()
        .unwrap()
        .find(revised_quote)
        .unwrap();
    fixture.write(
        "revised-passage.json",
        json!({
            "representation_id":id(&revised_header),
            "start":revised_start,
            "end":revised_start + revised_quote.len(),
            "expected_text":revised_quote,
        }),
    );
    let revised_passage = fixture.legacy(
        "create-passage",
        "offline.json",
        workspace_id,
        "passage-revised",
        &[&fixture.p("revised-passage.json")],
    );
    assert_ne!(id(&revised_passage), id(&original_passage));
    assert_eq!(revised_passage["quote"], revised_quote);
    assert_ne!(
        revised_passage["representation"]["document"]["checksum"],
        original_checksum
    );

    let workspace = fixture.conversation("workspace", "offline.json", &[conversation_id]);
    fixture.write(
        "close-view.json",
        json!({
            "expected_revision":workspace["revision"],
            "mutation":{"operation":"close", "view_id":id(&original_view)},
        }),
    );
    fixture.conversation(
        "layout",
        "offline.json",
        &[conversation_id, &fixture.p("close-view.json")],
    );
    std::fs::remove_file(fixture.p("manifest.json")).unwrap();

    let reopened_passage = fixture.legacy(
        "read-passage",
        "offline.json",
        workspace_id,
        "reopen-original",
        &[id(&original_passage)],
    );
    let reopened_source = fixture.legacy(
        "resolve-passage",
        "offline.json",
        workspace_id,
        "reopen-source",
        &[id(&original_passage)],
    );
    assert_eq!(reopened_passage, original_passage);
    assert_eq!(reopened_source, source);
    assert_eq!(
        fixture.conversation("context", "offline.json", &[conversation_id, run_id],),
        frozen_context
    );
    assert_eq!(
        fixture.conversation(
            "view",
            "offline.json",
            &[conversation_id, id(&original_view)],
        )["dataset_id"],
        original_dataset["id"]
    );
}

#[test]
fn malformed_oversized_and_foreign_passage_inputs_write_nothing() {
    let fixture = Fixture::new();
    let owner = fixture.conversation(
        "create",
        "offline.json",
        &[&fixture.p("conversation-create.json")],
    );
    fixture.write(
        "other-conversation.json",
        json!({"request_id":"other-conversation", "title":"Other filing"}),
    );
    let other = fixture.conversation(
        "create",
        "offline.json",
        &[&fixture.p("other-conversation.json")],
    );
    let workspace = owner["workspace_id"].as_str().unwrap();
    let fetch = fixture.legacy(
        "fetch",
        "passage-html.json",
        workspace,
        "fetch",
        &[&fixture.p("passage-document.json")],
    );
    let dataset = fixture.legacy(
        "dataset",
        "offline.json",
        workspace,
        "dataset",
        &[
            fetch["fetch_id"].as_str().unwrap(),
            &fixture.p("document-projection.json"),
        ],
    );
    let header = fixture.legacy(
        "prepare-text",
        "offline.json",
        workspace,
        "prepare",
        &[id(&dataset)],
    );
    fixture.write(
        "passage.json",
        json!({
            "representation_id":id(&header), "start":13, "end":33,
            "expected_text":"Revenue & cash grew.",
        }),
    );
    let passage = fixture.legacy(
        "create-passage",
        "offline.json",
        workspace,
        "passage",
        &[&fixture.p("passage.json")],
    );

    fixture.write(
        "malformed-passage.json",
        json!({
            "representation_id":id(&header), "start":13, "end":33,
            "expected_text":"Revenue & cash grew.", "workspace_id":"forged",
        }),
    );
    fixture.legacy_refusal(
        "create-passage",
        workspace,
        "malformed",
        &[&fixture.p("malformed-passage.json")],
    );
    std::fs::write(
        fixture.p("oversized-passage.json"),
        vec![b'x'; 1024 * 1024 + 1],
    )
    .unwrap();
    fixture.legacy_refusal(
        "create-passage",
        workspace,
        "oversized",
        &[&fixture.p("oversized-passage.json")],
    );
    fixture.legacy_refusal(
        "read-text",
        workspace,
        "oversized-range",
        &[id(&header), "0", "999999999999999999999"],
    );
    fixture.legacy_refusal(
        "create-passage",
        other["workspace_id"].as_str().unwrap(),
        "foreign-workspace",
        &[&fixture.p("passage.json")],
    );

    fixture.write(
        "foreign-send.json",
        json!({
            "conversation_id":id(&other), "request_id":"foreign-passage",
            "text":"Explain this passage.",
            "selected":[{"kind":"passage", "id":id(&passage)}],
        }),
    );
    fixture.write(
        "foreign-runtime.json",
        json!({
            "mode":"passage", "expected_passage_id":id(&passage),
            "expected_quote":"Revenue & cash grew.",
            "expected_document_checksum":passage["representation"]["document"]["checksum"],
        }),
    );
    fixture.conversation_refusal("foreign-send.json", "foreign-runtime.json");
}

#[test]
fn legacy_synthetic_document_mode_remains_plain_text() {
    let fixture = Fixture::new();
    let conversation = fixture.conversation(
        "create",
        "offline.json",
        &[&fixture.p("conversation-create.json")],
    );
    let workspace = conversation["workspace_id"].as_str().unwrap();
    let fetch = fixture.legacy(
        "fetch",
        "config.json",
        workspace,
        "legacy-document-fetch",
        &[&fixture.p("passage-document.json")],
    );
    let dataset = fixture.legacy(
        "dataset",
        "offline.json",
        workspace,
        "legacy-document-dataset",
        &[
            fetch["fetch_id"].as_str().unwrap(),
            &fixture.p("document-projection.json"),
        ],
    );
    let document = fixture.legacy(
        "document",
        "offline.json",
        workspace,
        "legacy-document-read",
        &[id(&dataset), "0", "256"],
    );
    assert_eq!(document["observation"]["media_type"], "text/plain");
    assert_eq!(
        document["bytes"],
        Value::Array(b"fixture document".iter().map(|byte| json!(byte)).collect())
    );
}
