# Lugus Codex Harness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking. The user selected subagent-driven execution.

**Goal:** Deliver a runtime-independent Rust harness that starts local Codex sessions, supplies application context and tools, streams events, and cancels reliably.

**Architecture:** Pure Lugus contracts define execution and tool boundaries. A Codex App Server adapter translates those contracts to a versioned stdio protocol and owns its child process; the application owns the adapter's lifetime. Persistent thesis knowledge and scheduling are subsequent milestones consuming these contracts, not hidden responsibilities of the runtime.

**Tech Stack:** Rust 2024, Tokio, serde/serde_json, thiserror, async-trait; the installed Codex CLI as an external process. Reuse dependency versions already resolved by the workspace. Use tempfile for deterministic integration tests and Python 3 for the fake process, matching existing financial tests.

**Spec:** [Approved agent architecture](../specs/2026-09-09-agent-harness-design.md).

## Global Constraints

- Codex is the initial runtime.
- Other agent runtimes must remain possible through adapters.
- Runtime sessions are disposable.
- Unattended reviews run only while Lugus is open.
- No always-running operating-system service is required.
- Domain types contain no Codex protocol types or provider SDK objects.
- Do not copy credentials into Lugus knowledge records or logs.
- Do not equate unattended work with unrestricted shell or filesystem access.
- A completed runtime turn without a valid persisted assessment does not complete a review.
- Schema validation establishes structure and referential integrity, not factual correctness.

## Scope and sequencing

The approved architecture contains three independently testable subsystems. Build them in this order, with a separate detailed implementation plan for each:

1. **Runtime harness (this plan):** interactive/headless context input, tool callbacks, structured progress, authentication status, cancellation, bounded process I/O, and a runnable example. It proves that Lugus can host Codex; it does not claim persistent memory or monitoring.
2. **Durable knowledge:** SQLite-owned thesis interpretations, captured evidence, assessments, shared findings/lessons, revisioned graph edges, text retrieval, optimistic concurrency, and idempotent writes. Acceptance: a new process retrieves knowledge from a previous session and conflicting revisions fail visibly.
3. **Evidence-driven reviews:** financial-source integration and captured external documents, dependency resolution, due dates, research refresh, durable trigger coalescing, completion transactions, shutdown/restart recovery, and meaningful-change events. Acceptance: changes cause one affected review, updates during a run remain pending, and interrupted work recovers without duplicate assessments.

The second and third plans must cover the remaining spec acceptance scenarios before the overall first product milestone can be called complete. No UI, portfolio accounting, trading, vector service, or separate MCP server is included here.

## Repository facts and protocol decision

All implementation paths below are relative to `/Users/havismat/lugus`, the Cargo workspace root. `lugus-agent` currently contains only Cargo metadata and the default add-function test. `lugus-financial` already has pure domain functions, async provider traits, process adapters, and a SQLite repository. Read those conventions; do not couple the new adapter to its protocol implementation.

The working directory for this planning session is `lugus-financial`, so this plan is saved beside the approved spec. The implementation belongs in sibling `lugus-agent`. Ensure the execution workspace permits those edits before starting.

Read-only investigation on 2026-09-09:

- `codex --version` returned `codex-cli 0.153.4`.
- `codex app-server generate-json-schema --experimental --out /tmp/lugus-codex-01534-schema` succeeded.
- `ThreadStartParams.dynamicTools`, `DynamicToolCallParams`, `DynamicToolCallResponse`, and `TurnInterruptParams` exist in that generated schema.
- Tool calls arrive as `item/tool/call`; text results use `contentItems: [{type: "inputText", text: ...}]` and `success`.
- The public App Server documentation confirms that dynamic tools require `initialize.params.capabilities.experimentalApi = true`.

**Selected binding:** App Server dynamic tools over stdio. Keep this experimental binding inside the Codex adapter. Initially accept the verified version `0.153.4`; extending the supported-version list requires schema and integration verification. Do not silently fall back to terminal scraping or unrestricted execution. No authenticated run has been performed during planning.

Sources: [App Server](https://learn.chatgpt.com/docs/app-server), [provider configuration](https://learn.chatgpt.com/docs/config-file/config-advanced).

## File map

| Path | Responsibility |
|---|---|
| `lugus-agent/src/lib.rs` | Public exports only |
| `lugus-agent/src/error.rs` | Typed configuration, protocol, timeout, process, authentication, cancellation, and tool errors |
| `lugus-agent/src/runtime.rs` | Runtime-independent request, event, tool, and execution contracts |
| `lugus-agent/src/tools.rs` | Tool registry validation and execution policy |
| `lugus-agent/src/codex/mod.rs` | Adapter lifecycle and public configuration |
| `lugus-agent/src/codex/protocol.rs` | Wire structs and message classification |
| `lugus-agent/src/codex/process.rs` | Owned child, bounded I/O, deadlines, and reaping |
| `lugus-agent/src/codex/session.rs` | Handshake, thread/turn execution, callbacks, and cancellation |
| `lugus-agent/tests/fixtures/codex_server.py` | Deterministic local fake Codex executable |
| `lugus-agent/tests/fixtures/codex-0.153.4/` | Selected generated schemas and provenance file |
| `lugus-agent/tests/runtime_contract.rs` | Request/tool validation |
| `lugus-agent/src/codex/protocol.rs` (unit tests) | Wire mapping and unknown-message policy |
| `lugus-agent/src/codex/process.rs` (unit tests) | Process failure, bounded I/O, cleanup |
| `lugus-agent/tests/codex_session.rs` | Execution, tool callback, cancellation scenarios |
| `lugus-agent/examples/codex_session.rs` | Explicitly invoked live integration example |
| `lugus-agent/README.md` | Setup, boundaries, commands, and verification |

## Shared contracts

Task 1 owns the following public types. Later tasks must use these names consistently. Derive Debug/Clone/serde traits where appropriate; do not derive serialization for process handles, channels, or credential-bearing configuration.

```rust
pub type Result<T> = std::result::Result<T, crate::error::Error>;

pub struct RunRequest {
    pub run_id: String,
    pub thesis_id: String,
    pub instructions: String,
    pub context: String,
    pub prompt: String,
    pub tools: Vec<ToolSpec>,
    pub limits: RunLimits,
}
pub struct RunLimits {
    pub timeout: std::time::Duration,
    pub max_tool_calls: usize,
    pub max_tool_result_bytes: usize,
}
pub struct ToolSpec {
    pub name: String,
    pub description: String,
    pub input_schema: serde_json::Value,
}
pub struct ToolCall {
    pub run_id: String,
    pub call_id: String,
    pub name: String,
    pub arguments: serde_json::Value,
}
pub struct ToolResult {
    pub success: bool,
    pub content: String,
}
pub enum RuntimeEvent {
    Started { run_id: String },
    TextDelta { text: String },
    ToolStarted { call_id: String, name: String },
    ToolFinished { call_id: String, success: bool },
    Usage { input_tokens: u64, output_tokens: u64 },
}
pub enum RunOutcome { Completed, Cancelled }
pub struct RunReport {
    pub run_id: String,
    pub outcome: RunOutcome,
    pub final_text: String,
}
#[async_trait::async_trait]
pub trait ToolExecutor: Send + Sync {
    async fn execute(&self, call: ToolCall) -> ToolResult;
}
#[async_trait::async_trait]
pub trait AgentRuntime: Send {
    async fn run(
        &mut self,
        request: RunRequest,
        tools: &dyn ToolExecutor,
        events: tokio::sync::mpsc::Sender<RuntimeEvent>,
        cancel: tokio::sync::watch::Receiver<bool>,
    ) -> Result<RunReport>;
    async fn close(&mut self) -> Result<()>;
}
```

All error cases return `Result::Err`; this milestone does not persist a run-state machine. `RunOutcome::Completed` means runtime completion only. Assessments and durable review completion belong to subsequent application coordination.

## Task 1: Public runtime and tool contracts

**Files:** modify `lugus-agent/Cargo.toml`, `src/lib.rs`; create `src/error.rs`, `src/runtime.rs`, `src/tools.rs`, `tests/runtime_contract.rs`.

**Interfaces:** produces the shared contracts above and `validate_request(request: &RunRequest) -> Result<()>`.

- [x] Add a failing test using an explicitly constructed request:

```rust
#[test]
fn rejects_duplicate_tool_names() {
    let tool = ToolSpec {
        name: "lugus_recall".into(),
        description: "Read stored knowledge".into(),
        input_schema: serde_json::json!({"type":"object","properties":{}}),
    };
    let request = RunRequest {
        run_id: "run-1".into(), thesis_id: "thesis-1".into(),
        instructions: "Analyze evidence".into(), context: "No prior assessment".into(),
        prompt: "Review this thesis".into(), tools: vec![tool.clone(), tool],
        limits: RunLimits { timeout: std::time::Duration::from_secs(30),
            max_tool_calls: 8, max_tool_result_bytes: 4096 },
    };
    assert!(validate_request(&request).is_err());
}
```

- [x] Run `cargo test -p lugus-agent --test runtime_contract`; expect unresolved contracts before implementation.
- [x] Implement the shared contracts and typed errors. Validate nonempty identifiers/prompt, positive timeout/result bounds, unique tool names matching `[A-Za-z0-9_-]{1,64}`, and object-shaped input schemas. Empty context is allowed. Zero permitted tool calls is allowed for a tool-free run.
- [x] Add focused cases for empty IDs, invalid tool names, and a valid tool-free request. Run the same test command and require success.
- [x] Commit only these files: `feat(agent): define runtime and tool contracts`.

## Task 2: Versioned Codex protocol mapping

**Files:** create `src/codex/mod.rs`, `src/codex/protocol.rs`, `tests/codex_protocol.rs`, `tests/fixtures/codex-0.153.4/`; export `codex` from `src/lib.rs`.

**Interfaces:** `classify_message(value: serde_json::Value) -> Result<WireMessage>`; `tool_response(id: serde_json::Value, result: &ToolResult) -> serde_json::Value`. `WireMessage` is adapter-private and has Response, Request, and Notification variants preserving the request ID as JSON.

- [x] Generate the schema from the explicitly selected executable, copy only the initialize, thread start, turn start/interrupt, dynamic tool, account read, and turn completion schemas into fixtures. Record version and generation command. Inspect every referenced definition; preserve self-contained definitions.
- [x] Add and run the failing protocol test:

```rust
#[test]
fn encodes_dynamic_tool_result() {
    let result = ToolResult { success: true, content: "saved".into() };
    assert_eq!(tool_response(serde_json::json!(42), &result), serde_json::json!({
        "id":42,"result":{"contentItems":[{"type":"inputText","text":"saved"}],
        "success":true}
    }));
}
```

- [x] Implement wire encoding without the `jsonrpc` header, as required by this protocol. Classify messages by `id`, `method`, and `result`/`error`; reject ambiguous envelopes. Accept both numeric and string server request IDs. Unknown notifications are ignored; unknown requests receive an explicit method-not-found error. Do not ignore malformed responses to an outstanding request.
- [x] Verify required fields against generated schema and add cases for malformed tool calls, string IDs, and unknown notifications. Keep sensitive payloads out of error display text.
- [x] Run `cargo test -p lugus-agent --lib codex::protocol`; commit `feat(agent): map supported Codex app-server protocol`.

## Task 3: Bounded child-process transport

**Files:** create `src/codex/process.rs`, `tests/fixtures/codex_server.py`, `tests/codex_process.rs`; extend private codex module exports.

**Interfaces:** adapter-private `CodexProcess::spawn(command: &std::path::Path, args: &[String], cwd: &std::path::Path, limits: TransportLimits) -> Result<Self>`; async `send(&mut self, message: &serde_json::Value) -> Result<()>`, `receive(&mut self) -> Result<serde_json::Value>`, and `close(&mut self) -> Result<()>`. `TransportLimits` has max_frame_bytes, request_timeout, shutdown_grace; defaults are 8 MiB, 30 seconds, and 2 seconds. Tests override durations.

- [x] Add the fake executable with scenarios selected through argv, not shell interpolation. Its initial oversized-frame behavior is:

```python
import json
import sys

scenario = sys.argv[1]
if scenario == "oversized":
    sys.stdout.write("x" * 2048 + "\n")
    sys.stdout.flush()
elif scenario == "echo":
    for line in sys.stdin:
        message = json.loads(line)
        print(json.dumps({"id": message["id"], "result": {}}), flush=True)
```

- [x] Write a test spawning `python3` with the fixture path and `oversized`, setting `max_frame_bytes = 1024`; assert `receive()` returns the typed frame-limit error and `close()` succeeds. Run `cargo test -p lugus-agent --lib codex::process` and observe failure before implementing transport.
- [x] Implement `tokio::process::Command` with argument arrays and piped stdin/stdout/stderr. Bound bytes before newline decoding rather than reading an unbounded line. Drain stderr with a bounded discarded/redacted tail; do not echo it automatically. Own all I/O tasks and join them during close.
- [x] Bound writes/reads with deadlines. On EOF, invalid UTF-8, malformed JSON, or exceeded frame size, return explicit errors and make the session unusable. `close()` closes stdin, waits for grace, kills if needed, and always waits/reaps. Enable kill-on-drop as a last-resort cleanup, not as a replacement for explicit close.
- [x] Extend the fixture for partial frames, malformed JSON, stderr flooding, and a process that ignores stdin EOF. Verify deadlines and reaping. Run the process tests; commit `feat(agent): supervise bounded Codex process transport`.

## Task 4: Session startup and tool dispatch

**Files:** create `src/codex/session.rs`, `tests/codex_session.rs`; extend `src/codex/mod.rs`, `tests/fixtures/codex_server.py`.

**Interfaces:** public `CodexConfig { executable: PathBuf, workspace: PathBuf, model: Option<String>, model_provider: Option<String> }`; `CodexRuntime::connect(config: CodexConfig) -> Result<Self>` (async); implements `AgentRuntime`. Public `CodexRuntime::account_status(&mut self) -> Result<AccountStatus>` where AccountStatus is Ready or LoginRequired. No account email or token is part of this status.

- [x] Add a fake-server scenario requiring this exact handshake before accepting a thread:

```json
{"id":1,"method":"initialize","params":{"clientInfo":{"name":"lugus","version":"0.1.0"},"capabilities":{"experimentalApi":true}}}
```

- [x] Test a fake session that rejects omitted initialization, requests `lugus_recall`, receives a host result, streams a message, and completes. The host executor returns `ToolResult { success: true, content: "Stored finding from thesis A".into() }`. Assert exactly one host call and the final text, not merely a completed subprocess.
- [x] Implement version probing through `--version` with bounded transport-independent output. Start `app-server` on stdio, initialize, send initialized notification, and inspect account readiness using the installed account-read schema. Do not initiate login automatically in unattended mode.
- [x] For each run create a fresh thread with dynamic tools, model/provider overrides when configured, the configured workspace, and Lugus instructions/context. Start a turn and correlate all responses using a monotonically increasing client request ID. Store thread/turn IDs only inside the adapter.
- [x] Bind incoming tool calls to the active thread/turn. Generate `ToolCall.run_id` from host state, never model arguments. Validate the tool name against the registered list and typed arguments inside the host executor. Enforce call/result bounds. Unknown tools return `success:false`; wrong-thread calls fail the protocol session.
- [x] Map text deltas and final assistant text without duplicating streamed content. Emit Started/ToolStarted/ToolFinished. Map usage only if the supported schema supplies reliable counters; do not fabricate zero usage when unavailable.
- [x] Run `cargo test -p lugus-agent --test codex_session`. Cover authentication-required state, RPC error, wrong-thread tool calls, and normal turn completion. Commit `feat(agent): execute Codex turns with Lugus tools`.

## Task 5: Unattended policy, cancellation, and failure behavior

**Files:** modify `src/codex/session.rs`, `src/codex/process.rs`, `src/tools.rs`, `tests/codex_session.rs`, `tests/fixtures/codex_server.py`.

**Interfaces:** consumes watch cancellation and RunLimits; produces Cancelled report on deliberate cancellation, typed Timeout on exhausted time, typed NeedsAttention on unsupported human-input/approval dependency.

- [x] Add a fake scenario that remains active until it receives:

```json
{"id":8,"method":"turn/interrupt","params":{"threadId":"thread-1","turnId":"turn-1"}}
```

- [x] Write a test that starts a run, waits for Started, sends `true` through the watch sender, and asserts Cancelled plus process cleanup. Add cancellation before startup and while a tool executor is pending. Run the tests and observe failure before implementing cancellation.
- [x] Use `tokio::select!` so run deadlines/cancellation continue to be observed while tools execute and event consumers are slow. Bound event delivery; a closed consumer fails explicitly rather than hanging. Do not assume dropping a tool future rolls back its external writes; later storage tools provide idempotency.
- [x] Configure `approvalPolicy: "never"` and the supported read-only sandbox for unattended sessions. Inspect the supported config schema and disable irrelevant inherited shell/extensions where supported; verify effective tool exposure in the live example. Do not mutate the user's global configuration. If required isolation cannot be expressed, return an unsupported-configuration error rather than widening permissions.
- [x] Answer approval requests with the schema's denial response. Human-input requests terminate the unattended task as NeedsAttention. All server requests receive a bounded response or terminate the transport; none wait silently for UI input.
- [x] On cancellation send turn/interrupt when IDs exist, allow the bounded grace period, then close/reap the owned process if it does not stop. A failed transport is not reused; reconnect explicitly for the next run.
- [x] Verify tool-call exhaustion, response-size exhaustion, timeout, consumer disconnect, process death, denied approval, repeated close, and cancellation races. Run `cargo test -p lugus-agent`; commit `feat(agent): enforce unattended execution and cancellation`.

## Task 6: Runnable integration and handoff to durable knowledge

**Files:** create `examples/codex_session.rs`, `README.md`; update `tests/codex_session.rs` if live inspection reveals a protocol mismatch.

**Interfaces:** example command accepts a dedicated workspace directory and prompt; it uses CodexRuntime plus a read-only demonstration tool. It does not save a pretend assessment or claim a memory system exists.

- [x] Implement the example's tool executor with this concrete result:

```rust
async fn execute(&self, call: ToolCall) -> ToolResult {
    match call.name.as_str() {
        "lugus_context" => ToolResult {
            success: true,
            content: "Demonstration context: thesis A depends on financing costs. This is application-supplied test data, not market evidence.".into(),
        },
        _ => ToolResult { success: false, content: "Unknown tool".into() },
    }
}
```

- [x] Run deterministic verification from the workspace:

```sh
cargo test -p lugus-agent
cargo clippy -p lugus-agent --all-targets -- -D warnings
cargo fmt -p lugus-agent -- --check
cargo test -p lugus-financial
```

- [x] Document setup with installed Codex authentication owned by Codex, supported version, dedicated working directory, and configurable provider. Make explicit that local storage/execution coordination can still send context to the selected remote model provider.
- [x] Run the live example only using the user's configured, authorized account. Have it call `lugus_context` and report its returned content. Capture sanitized protocol observations, confirm event delivery, and test interruption. If authentication is unavailable, record the live check as blocked and request the needed setup; do not label fixture success as verified live integration.
- [x] Check native web search in the selected configuration with a benign public-document lookup. Record whether available. Do not claim that search results have become durable evidence: captured documents and provenance storage belong to milestone 3. If native search is unavailable, record a research-adapter requirement for that plan.
- [x] Document that this harness runs one task at a time per adapter instance, does not store memories, does not schedule reviews, and returns runtime completion independently of future assessment validation.
- [x] Commit `docs(agent): demonstrate Codex context and tool integration` and report deterministic/live results separately.

## Self-review and full-spec coverage

| Approved requirement | Delivery |
|---|---|
| Replaceable runtime and functional boundaries | Tasks 1–2 |
| Codex tool binding and authentication status | Task 4 |
| Process lifetime, bounded I/O, and cancellation | Tasks 3 and 5 |
| Unattended policy and visible execution failures | Task 5 |
| Runnable integration and live verification | Task 6 |
| Thesis history, evidence versions, graph, learned lessons | Milestone 2, separate detailed plan |
| SQLite storage, idempotency, and revision conflicts | Milestone 2, separate detailed plan |
| Financial evidence, external document capture | Milestone 3, separate detailed plan |
| Trigger coalescing, due dates, recovery, material changes | Milestone 3, separate detailed plan |
| Cross-session knowledge and feedback learning acceptance | Milestones 2–3 acceptance tests |

This plan deliberately stops at a working runtime harness. The approved product architecture remains the parent scope; the next step after this milestone is the durable-knowledge implementation plan, using the verified tool contract rather than assuming session memory.

## Execution notes (2026-09-10)

Implementation is on `feat/agent-harness` in `/tmp/lugus-agent-harness`. All six tasks have completed implementation and independent review. The whole-branch review findings were corrected, including restoration of the thirty-second absolute RPC deadline. Protocol and process tests are private module unit tests rather than empty integration targets. Session helpers are separated into events, version, and policy modules. Reviews added regression coverage for malformed completion, parameterless unknown requests, partial-write cancellation, total read deadlines, inherited stderr, tool namespaces, message identity, process-start policy, per-run MCP refresh, and cancellation reaping. Runtime completion is still separate from durable assessment completion.

Final implementation validation: 59 agent tests, 2 example tests, and 36 financial tests passed; strict agent Clippy and formatting passed. On 2026-09-10 the runnable example verified a real Lugus tool callback, native webSearch started/completed metadata, and active-turn cancellation using existing Codex authentication in empty temporary workspaces. No credentials or user configuration were copied or changed. Whole-branch review identified startup cancellation, selected-provider authentication, and total RPC deadline gaps; all were corrected. A regression test verifies that a valid six-second startup response retains the previous thirty-second tolerance. The final workspace all-target suite passed all 97 tests.
