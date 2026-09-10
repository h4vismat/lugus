use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Write},
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};
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
        assert!(output.status.success());
        Self { root }
    }
    fn p(&self, name: &str) -> String {
        self.root.path().join(name).to_str().unwrap().into()
    }
    fn write(&self, name: &str, v: Value) {
        std::fs::write(self.p(name), v.to_string()).unwrap();
    }
    fn call(&self, command: &str, args: &[&str]) -> Value {
        self.config_call(command, "offline.json", args)
    }
    fn config_call(&self, command: &str, config: &str, args: &[&str]) -> Value {
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args(["conversation", command, &self.p(config)])
            .args(args)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{command}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        serde_json::from_slice(
            output
                .stdout
                .split(|b| *b == b'\n')
                .rfind(|v| !v.is_empty())
                .unwrap(),
        )
        .unwrap()
    }
    fn create(&self, id: &str) -> Value {
        self.write(
            "create.json",
            json!({"request_id":id,"title":"Apple research"}),
        );
        self.call("create", &[&self.p("create.json")])
    }
    fn send(&self, c: &str, key: &str, selected: Value) {
        self.write("send.json",json!({"conversation_id":c,"request_id":key,"text":"Research Apple/AAPL", "selected":selected}));
    }
    fn counts(&self) -> Vec<i64> {
        let db = rusqlite::Connection::open(self.p("application.sqlite")).unwrap();
        [
            "conversations",
            "conversation_messages",
            "conversation_runs",
            "conversation_tools",
            "conversation_activity",
            "app_records",
            "dataset_rows",
            "binding_history",
            "view_requests",
        ]
        .iter()
        .map(|table| {
            db.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))
                .unwrap()
        })
        .collect()
    }
    fn fail(&self, command: &str, args: &[&str]) -> Value {
        let before = self.counts();
        let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args(["conversation", command, &self.p("offline.json")])
            .args(args)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert_eq!(
            before,
            self.counts(),
            "rejected command wrote durable records"
        );
        serde_json::from_slice(&output.stderr).unwrap()
    }
}
fn legacy(f: &Fixture, command: &str, workspace: &str, request: &str, args: &[&str]) -> Value {
    let output = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args([command, &f.p("offline.json"), workspace, request])
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
fn id(v: &Value) -> &str {
    v["id"].as_str().unwrap()
}
fn items(v: &Value) -> &[Value] {
    v["items"].as_array().unwrap()
}
#[test]
fn actual_research_restores_offline_and_continues_without_replay() {
    let f = Fixture::new();
    let c = f.create("first");
    let cid = id(&c);
    assert_eq!(f.call("show", &[cid]), c);
    assert_eq!(
        items(&f.call("list", &["0", "10"])),
        std::slice::from_ref(&c)
    );
    assert_eq!(f.call("workspace", &[cid])["view_ids"], json!([]));
    assert!(items(&f.call("messages", &[cid, "0", "100"])).is_empty());
    assert!(!std::path::Path::new(&f.p("thesis.sqlite")).exists());
    f.send(cid, "research", json!([]));
    let first = f.config_call(
        "send",
        "config.json",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    assert_eq!(first["event"], "terminal");
    assert_eq!(first["run"]["status"], "completed");
    let rid = id(&first["run"]);
    let tools = f.call("tools", &[cid, rid, "0", "100"]);
    assert_eq!(items(&tools).len(), 12);
    let messages = f.call("messages", &[cid, "0", "100"]);
    assert_eq!(items(&messages).len(), 2);
    let answer: Value =
        serde_json::from_str(items(&messages)[1]["text"].as_str().unwrap()).unwrap();
    assert_eq!(answer["subject_kind"], "conversation");
    let receipts = &answer["receipts"];
    let dataset = id(&receipts["dataset"]);
    assert_eq!(
        receipts["fetch"]["command"]["query"]["instrument"]["value"],
        "AAPL"
    );
    let workspace = f.call("workspace", &[cid]);
    assert_eq!(workspace["view_ids"], json!([id(&receipts["view"])]));
    std::fs::remove_file(f.p("manifest.json")).unwrap();
    assert_eq!(
        messages,
        f.config_call("messages", "config.json", &[cid, "0", "100"])
    );
    assert_eq!(workspace, f.call("workspace", &[cid]));
    let counts = f.counts();
    let duplicate = f.call(
        "send",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    assert_eq!(duplicate["run"], first["run"]);
    assert_eq!(
        counts,
        f.counts(),
        "duplicate request replayed durable work"
    );
    let old_page = f.call("dataset", &[cid, dataset, "0", "1"]);
    assert_eq!(old_page["header"]["binding_id"], receipts["binding"]["id"]);
    f.send(cid,"followup",json!([{"kind":"dataset","id":dataset},{"kind":"binding","id":id(&receipts["binding"])},{"kind":"view","id":id(&receipts["view"])}]));
    let expected: Vec<_> = items(&messages).iter().map(|m| m["id"].clone()).collect();
    f.write(
        "continue.json",
        json!({"mode":"continue","expected_message_ids":expected,"expected_dataset_id":dataset}),
    );
    let continued = f.call("continue", &[&f.p("send.json"), &f.p("continue.json")]);
    assert_eq!(continued["run"]["status"], "completed");
    let frozen = f.call("context", &[cid, id(&continued["run"])]);
    assert_eq!(frozen["references"].as_array().unwrap().len(), 3);
    assert_eq!(tools, f.call("tools", &[cid, rid, "0", "100"]));
    assert!(items(&f.call("tools", &[cid, id(&continued["run"]), "0", "100"])).is_empty());
    // Supersede using the original saved source evidence, then freeze another dataset
    // and open a second view through the existing manual application commands.
    let arguments: Value =
        serde_json::from_str(items(&tools)[6]["intent"]["arguments"].as_str().unwrap()).unwrap();
    let mut binding = arguments["request"].clone();
    binding["supersedes"] = receipts["binding"]["id"].clone();
    f.write("supersede.json", binding);
    let ws = c["workspace_id"].as_str().unwrap();
    let replacement = legacy(&f, "bind", ws, "supersede", &[&f.p("supersede.json")]);
    assert_ne!(id(&replacement), id(&receipts["binding"]));
    let old_binding = legacy(
        &f,
        "read-binding",
        ws,
        "inspect",
        &[id(&receipts["binding"])],
    );
    assert_eq!(old_binding["status"]["status"], "superseded");
    f.write("projection.json", old_page["header"]["projection"].clone());
    let new_dataset = legacy(
        &f,
        "dataset",
        ws,
        "new-dataset",
        &[id(&receipts["fetch"]), &f.p("projection.json")],
    );
    assert_ne!(id(&new_dataset), dataset);
    let new_view = legacy(
        &f,
        "view",
        ws,
        "new-view",
        &[id(&new_dataset), "price_chart"],
    );
    let workspace = f.call("workspace", &[cid]);
    assert_eq!(workspace["view_ids"].as_array().unwrap().len(), 2);
    f.write("reorder.json",json!({"expected_revision":workspace["revision"],"mutation":{"operation":"reorder","view_ids":[id(&new_view),id(&receipts["view"])]}}));
    let workspace = f.call("layout", &[cid, &f.p("reorder.json")]);
    f.write("select.json",json!({"expected_revision":workspace["revision"],"mutation":{"operation":"select","view_id":id(&receipts["view"])}}));
    let workspace = f.call("layout", &[cid, &f.p("select.json")]);
    f.write("layout.json",json!({"expected_revision":workspace["revision"],"mutation":{"operation":"close","view_id":id(&receipts["view"])}}));
    assert_eq!(
        f.call("layout", &[cid, &f.p("layout.json")])["view_ids"],
        json!([id(&new_view)])
    );
    assert_eq!(
        f.call("view", &[cid, id(&receipts["view"])])["dataset_id"],
        dataset
    );
    assert_eq!(frozen, f.call("context", &[cid, id(&continued["run"])]));
    assert_eq!(old_page, f.call("dataset", &[cid, dataset, "0", "1"]));
}
#[test]
fn malformed_and_cross_scope_commands_do_not_write_records() {
    let f = Fixture::new();
    let c = f.create("first");
    let other = f.create("other");
    f.send(id(&c), "research", json!([]));
    let run = f.config_call(
        "send",
        "config.json",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    let messages = f.call("messages", &[id(&c), "0", "100"]);
    let answer: Value =
        serde_json::from_str(items(&messages)[1]["text"].as_str().unwrap()).unwrap();
    let dataset = id(&answer["receipts"]["dataset"]);
    f.fail("status", &[id(&other), id(&run["run"])]);
    f.fail("dataset", &[id(&other), dataset, "0", "1"]);
    f.send(
        id(&other),
        "foreign",
        json!([{"kind":"dataset","id":dataset}]),
    );
    f.fail(
        "send",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    f.write(
        "bad.json",
        json!({"request_id":"bad","title":"ok","unexpected":"secret"}),
    );
    let error = f.fail("create", &[&f.p("bad.json")]);
    assert_eq!(error["error"]["kind"], "invalid_input");
    std::fs::write(f.p("bad.json"), "x".repeat(1024 * 1024 + 1)).unwrap();
    assert_eq!(
        f.fail("create", &[&f.p("bad.json")])["error"]["kind"],
        "resource_limit"
    );
    f.send(id(&c), "large", json!([]));
    let mut request: Value =
        serde_json::from_slice(&std::fs::read(f.p("send.json")).unwrap()).unwrap();
    request["text"] = json!("\\".repeat(16384));
    f.write("send.json", request);
    f.fail(
        "send",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    f.fail("messages", &[id(&c), "0", "0"]);
    f.send(id(&c), "bad-fixture", json!([]));
    f.write("fixture.json",json!({"mode":"research","workflow":{"company_instance_id":"filings","market_instance_id":"market","input":"Apple","native_namespace":"yahoo:symbol","start":"2024-01-01","end":"2024-01-03","page_size":10,"unexpected":"private"}}));
    f.fail("send", &[&f.p("send.json"), &f.p("fixture.json")]);
    f.write(
        "layout.json",
        json!({"expected_revision":0,"mutation":{"operation":"reorder","view_ids":[]}}),
    );
    f.fail("layout", &[id(&c), &f.p("layout.json")]);
}
struct Running {
    child: Child,
    lines: BufReader<std::process::ChildStdout>,
}
impl Drop for Running {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Running {
    fn start(f: &Fixture) -> (Self, Value) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([
                "conversation",
                "send",
                &f.p("blocked.json"),
                &f.p("send.json"),
                &f.p("conversation-research.json"),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap());
        let mut line = String::new();
        lines.read_line(&mut line).unwrap();
        let receipt: Value = serde_json::from_str(&line).unwrap_or_else(|error| {
            use std::io::Read;
            let mut stderr = String::new();
            child
                .stderr
                .as_mut()
                .unwrap()
                .read_to_string(&mut stderr)
                .unwrap();
            panic!("{error}: {stderr}")
        });
        assert_eq!(receipt["event"], "admitted");
        (Self { child, lines }, receipt)
    }
}
fn pending(f: &Fixture, c: &str, r: &str) -> Value {
    let start = Instant::now();
    loop {
        let tools = f.call("tools", &[c, r, "0", "100"]);
        if items(&tools).len() == 8 {
            return tools;
        }
        assert!(
            start.elapsed() < Duration::from_secs(8),
            "pending tool not reached: {tools}"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
}
#[test]
fn real_process_kill_exclusive_recovery_and_owning_stdin_cancel() {
    let f = Fixture::new();
    let c = f.create("first");
    let cid = id(&c);
    f.send(cid, "complete", json!([]));
    let completed = f.config_call(
        "send",
        "config.json",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    let original = f.call("tools", &[cid, id(&completed["run"]), "0", "100"]);
    f.send(cid, "crash", json!([]));
    let (mut running, receipt) = Running::start(&f);
    let rid = id(&receipt["run"]);
    pending(&f, cid, rid);
    assert_eq!(f.fail("recover", &[])["error"]["kind"], "conflict");
    let partial = f.call("activity", &[cid, rid, "0", "100"]);
    assert!(items(&partial).iter().any(|a| {
        serde_json::from_str::<Value>(a["data"].as_str().unwrap())
            .is_ok_and(|event| event["type"] == "text_delta")
    }));
    let before = f.counts();
    writeln!(
        running.child.stdin.as_mut().unwrap(),
        "{}",
        "x".repeat(4097)
    )
    .unwrap();
    let mut error = String::new();
    running.lines.read_line(&mut error).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&error).unwrap()["error"]["kind"],
        "resource_limit"
    );
    drop(running.child.stdin.take());
    assert_eq!(
        f.call("status", &[cid, rid])["status"],
        "running",
        "EOF must not cancel"
    );
    assert_eq!(before, f.counts());
    let provider_pid = std::fs::read_to_string(f.p("market/pid")).unwrap();
    running.child.kill().unwrap();
    running.child.wait().unwrap();
    let exited = Instant::now();
    while Command::new("kill")
        .args(["-0", provider_pid.trim()])
        .stderr(Stdio::null())
        .status()
        .unwrap()
        .success()
    {
        assert!(
            exited.elapsed() < Duration::from_secs(3),
            "blocked fixture child survived owner pipe EOF"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(f.call("status", &[cid, rid])["status"], "running");
    let before = f.counts();
    f.call("recover", &[]);
    let after = f.counts();
    assert_eq!(before, after, "recovery inserts no turns/tools/activity");
    assert_eq!(f.call("status", &[cid, rid])["status"], "interrupted");
    let tools = f.call("tools", &[cid, rid, "0", "100"]);
    assert!(items(&tools)[7]["outcome"].is_null());
    assert!(items(&tools)[7]["finished_at"].is_null());
    assert_eq!(
        original,
        f.call("tools", &[cid, id(&completed["run"]), "0", "100"])
    );
    assert_eq!(items(&f.call("messages", &[cid, "0", "100"])).len(), 3);
    f.send(cid, "new-turn", json!([]));
    let next = f.config_call(
        "send",
        "config.json",
        &[&f.p("send.json"), &f.p("conversation-research.json")],
    );
    assert_eq!(next["run"]["status"], "completed");
    f.send(cid, "cancel-turn", json!([]));
    let (mut owner, receipt) = Running::start(&f);
    pending(&f, cid, id(&receipt["run"]));
    let before = f.counts();
    writeln!(owner.child.stdin.as_mut().unwrap(),"{}",json!({"command":"cancel","conversation_id":"another-conversation","run_id":id(&receipt["run"])})).unwrap();
    let mut line = String::new();
    owner.lines.read_line(&mut line).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&line).unwrap()["error"]["kind"],
        "scope_mismatch"
    );
    assert_eq!(before, f.counts());
    assert_eq!(
        f.call("status", &[cid, id(&receipt["run"])])["status"],
        "running"
    );
    writeln!(
        owner.child.stdin.as_mut().unwrap(),
        "{}",
        json!({"command":"cancel","conversation_id":cid,"run_id":id(&receipt["run"])})
    )
    .unwrap();
    let mut line = String::new();
    owner.lines.read_line(&mut line).unwrap();
    let terminal: Value = serde_json::from_str(&line).unwrap();
    assert_eq!(terminal["event"], "terminal");
    assert_eq!(terminal["run"]["status"], "interrupted");
    assert!(owner.child.wait().unwrap().success());
}

#[test]
fn terminal_process_exits_with_idle_open_stdin_and_views_can_precede_messages() {
    let f = Fixture::new();
    let c = f.create("views-first");
    let cid = id(&c);
    let ws = c["workspace_id"].as_str().unwrap();
    // A manual view can belong to a conversation before it has any model turns.
    let fetch = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args([
            "fetch",
            &f.p("config.json"),
            ws,
            "manual",
            &f.p("prices.json"),
        ])
        .output()
        .unwrap();
    assert!(fetch.status.success());
    let fetch: Value = serde_json::from_slice(&fetch.stdout).unwrap();
    let fetch = legacy(
        &f,
        "read-fetch",
        ws,
        "read",
        &[fetch["fetch_id"].as_str().unwrap()],
    );
    f.write("projection.json",json!({"kind":"prices","run_id":fetch["runs"][0]["id"],"query":fetch["command"]["query"],"series":"close"}));
    let dataset = legacy(
        &f,
        "dataset",
        ws,
        "dataset",
        &[id(&fetch), &f.p("projection.json")],
    );
    let view = legacy(&f, "view", ws, "view", &[id(&dataset), "price_chart"]);
    assert_eq!(f.call("workspace", &[cid])["view_ids"], json!([id(&view)]));
    assert!(items(&f.call("messages", &[cid, "0", "100"])).is_empty());
    assert!(items(&f.call("runs", &[cid, "0", "100"])).is_empty());
    for (key, fixture, status) in [
        ("success", "conversation-research.json", "completed"),
        ("failure", "bad-continue.json", "failed"),
    ] {
        f.send(cid, key, json!([]));
        f.write(
            "bad-continue.json",
            json!({"mode":"continue","expected_message_ids":[],"expected_dataset_id":id(&dataset)}),
        );
        let mut child = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
            .args([
                "conversation",
                "send",
                &f.p("config.json"),
                &f.p("send.json"),
                &f.p(fixture),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let lines = BufReader::new(child.stdout.take().unwrap());
        let mut owner = Running { child, lines };
        let start = Instant::now();
        while owner.child.try_wait().unwrap().is_none() {
            assert!(
                start.elapsed() < Duration::from_secs(8),
                "idle stdin prevented exit"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        let mut output = String::new();
        std::io::Read::read_to_string(&mut owner.lines, &mut output).unwrap();
        let terminal: Value = serde_json::from_str(output.lines().last().unwrap()).unwrap();
        assert_eq!(terminal["run"]["status"], status);
    }
}

#[test]
fn unread_stdout_cannot_block_cancel_cleanup_or_process_exit() {
    let f = Fixture::new();
    for name in ["config.json", "offline.json", "blocked.json"] {
        let mut config: Value = serde_json::from_slice(&std::fs::read(f.p(name)).unwrap()).unwrap();
        config["limits"]["max_output_bytes"] = json!(1048576);
        f.write(name, config);
    }
    let c = f.create("backpressure");
    let cid = id(&c);
    for index in 0..5 {
        f.write("send.json", json!({"conversation_id":cid,"request_id":format!("history-{index}"),"text":"x".repeat(15000),"selected":[]}));
        let result = f.config_call(
            "send",
            "config.json",
            &[&f.p("send.json"), &f.p("conversation-research.json")],
        );
        assert_eq!(result["run"]["status"], "completed");
    }
    f.write(
        "send.json",
        json!({"conversation_id":cid,"request_id":"unread","text":"x".repeat(15000),"selected":[]}),
    );
    let mut child = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args([
            "conversation",
            "send",
            &f.p("blocked.json"),
            &f.p("send.json"),
            &f.p("conversation-research.json"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let lines = BufReader::new(child.stdout.take().unwrap());
    let mut owner = Running { child, lines };
    // Deliberately never consume this owner's stdout, including its admitted receipt.
    let start = Instant::now();
    let run = loop {
        let runs = f.call("runs", &[cid, "0", "100"]);
        if let Some(run) = items(&runs)
            .iter()
            .find(|run| run["request_id"] == "unread")
        {
            break run.clone();
        }
        assert!(start.elapsed() < Duration::from_secs(4));
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        run.to_string().len() > 65536,
        "admission must exceed pipe capacity"
    );
    pending(&f, cid, id(&run));
    // Fill the bounded diagnostic queue too: invalid controls must not prevent a
    // subsequent valid cancellation, even when their notifications cannot be delivered.
    for _ in 0..4 {
        writeln!(
            owner.child.stdin.as_mut().unwrap(),
            "{{\"command\":\"invalid\"}}"
        )
        .unwrap();
    }
    writeln!(
        owner.child.stdin.as_mut().unwrap(),
        "{}",
        json!({"command":"cancel","conversation_id":cid,"run_id":id(&run)})
    )
    .unwrap();
    let start = Instant::now();
    loop {
        let status = f.call("status", &[cid, id(&run)]);
        if status["status"] == "interrupted" {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(3),
            "unread stdout blocked durable cancellation: {}",
            status["status"]
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    let tools = f.call("tools", &[cid, id(&run), "0", "100"]);
    assert!(
        items(&tools).iter().all(|tool| !tool["outcome"].is_null()),
        "cancel must finish owned tool cleanup"
    );
    assert_eq!(items(&tools).len(), 8);
    let before = f.counts();
    let lease = loop {
        match lugus_app::conversations::LocalExecutionLease::acquire(f.p("application.sqlite")) {
            Ok(lease) => break lease,
            Err(error) => {
                assert!(
                    start.elapsed() < Duration::from_secs(4),
                    "output held lease after cancellation: {error}"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    assert!(
        owner.child.try_wait().unwrap().is_none(),
        "lease should release before output delivery times out"
    );
    drop(lease);
    f.call("recover", &[]); // Lease is released without draining the owner's stdout.
    assert_eq!(before, f.counts());
    while owner.child.try_wait().unwrap().is_none() {
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "unread stdout blocked process shutdown"
        );
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !owner.child.wait().unwrap().success(),
        "output delivery timeout must be a nonzero exit"
    );
    assert_eq!(f.call("status", &[cid, id(&run)])["status"], "interrupted");
}

#[test]
fn broken_stdout_closes_the_owning_host_without_waiting_for_turn_deadline() {
    let f = Fixture::new();
    let c = f.create("broken-output");
    let cid = id(&c);
    f.send(cid, "broken-output", json!([]));
    let mut child = Command::new(env!("CARGO_BIN_EXE_lugus-research"))
        .args([
            "conversation",
            "send",
            &f.p("blocked.json"),
            &f.p("send.json"),
            &f.p("conversation-research.json"),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    drop(child.stdout.take());
    let start = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if start.elapsed() > Duration::from_secs(4) {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("broken stdout prevented host cleanup");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!child.wait().unwrap().success());
    let mut stderr = String::new();
    std::io::Read::read_to_string(child.stderr.as_mut().unwrap(), &mut stderr).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(&stderr).unwrap()["error"]["kind"],
        "unavailable"
    );
    let runs = f.call("runs", &[cid, "0", "100"]);
    assert_eq!(items(&runs).len(), 1);
    assert!(matches!(
        items(&runs)[0]["status"].as_str(),
        Some("interrupted" | "failed")
    ));
    assert_eq!(items(&f.call("messages", &[cid, "0", "100"])).len(), 1);
    let tools = f.call("tools", &[cid, id(&items(&runs)[0]), "0", "100"]);
    assert!(items(&tools).iter().all(|tool| !tool["outcome"].is_null()));
    let counts = f.counts();
    f.call("recover", &[]);
    assert_eq!(counts, f.counts());
}
