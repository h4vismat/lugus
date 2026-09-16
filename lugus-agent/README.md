# lugus-agent

`lugus-agent` provides disposable agent turns and durable, explicitly requested investment reviews. Its public `AgentRuntime` and `ToolExecutor` boundaries are provider-neutral. The adapters support `codex-cli 0.153.4` or `0.154.0`, and Claude Code `2.1.268`. Other CLI versions are rejected until their protocol compatibility is verified.

## Run the Claude Code example

Install Claude Code separately and sign in with its CLI. Lugus preserves that authentication environment without reading, copying, or storing credentials. `ClaudeRuntime::connect(ClaudeConfig { executable, workspace, model })` validates the workspace, optional model and CLI version. Every `run` launches a fresh print session; `close` prevents subsequent runs.

```sh
mkdir -p /tmp/lugus-claude-demo
cargo run -p lugus-agent --example claude_session -- \
  /tmp/lugus-claude-demo /path/to/claude
```

An optional third argument selects a model. The example supplies a fictional number through `synthetic_value`, prints streamed events and the final report, and uses no financial storage. Live verification on 2026-09-11 successfully dispatched the tool once, streamed the fictional result, and reported token usage with the installed CLI's existing authentication.

Claude receives instructions as its explicit system prompt and receives the context and user prompt through stdin. The adapter uses `--restricted`, `--strict-mcp-config`, `--disable-slash-commands`, `disableAllHooks`, no session persistence, and an explicit built-in tool list. `allow_web_search` enables only `WebSearch` and `WebFetch`; otherwise no built-in research or file/command tools are selected. Host tools are individually permitted, and other permission requests are automatically denied. User/project/local settings and their plugins do not load. Organization-managed policy remains authoritative, including mandatory managed hooks; use an installation whose organization policy is appropriate for the data supplied. `--bare` is deliberately avoided because it disables OAuth/keychain authentication, while `--safe-mode` also removes explicitly configured HTTP MCP servers.

The Rust MCP transport uses Hyper on a random `127.0.0.1` port with a fresh 256-bit bearer token, rejects every Origin header, caps HTTP bodies at 1 MiB, and owns all connection tasks. It supports the 2025 Streamable HTTP protocol versions through `2025-11-25`, JSON responses, initialization, tool discovery and invocation. Tool results return to their originating HTTP requests. Run limits apply before host callback dispatch; exhausted budgets return an MCP tool error without executing the callback. Native search does not consume the host callback budget. Inputs, output frames, accumulated text, connection count and HTTP request count are bounded. CLI stderr is discarded and raw authentication errors are not forwarded. Completion, errors, cancellation and timeout stop the transport and reap the CLI; dropping the run future kills the CLI and aborts the transport tasks.

Protocol references: [CLI reference](https://code.claude.com/docs/en/cli-reference), [stream message types](https://code.claude.com/docs/en/agent-sdk/typescript), [MCP Streamable HTTP](https://modelcontextprotocol.io/specification/2025-11-25/basic/transports). Deterministic tests use a Python subprocess fixture only; production has no Python or Node SDK dependency. Transport tests need permission to bind and connect to localhost.

## Run the Codex example

Install that Codex version separately and authenticate with Codex itself:

```sh
codex --version
codex login status
```

The harness does not start login, read tokens, copy credentials, or store account identity. A normal Codex installation can be authenticated with `codex login` before running the example.

Create a dedicated working directory that contains only files the demonstration may expose to its configured model provider. The adapter starts Codex with a read-only sandbox, disables inherited MCP servers and unrelated execution features, and does not modify global Codex configuration.

```sh
mkdir -p /tmp/lugus-codex-demo
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo \
  'Call lugus_context once, repeat its returned content, then explain which statement is demonstration data.'
```

The command prints runtime events, the terminal outcome, and the final assistant text. `lugus_context` returns fixed, application-supplied test data through a read-only callback. It does not query or write Lugus financial storage.

Codex uses its configured model and provider by default. Explicit overrides are available for installations that define them:

```sh
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo 'Call lugus_context and summarize it.' \
  --model MODEL --provider PROVIDER
```

Use `--codex PATH` when the supported executable is outside `PATH`. Even when Lugus stores data locally and coordinates execution locally, the prompt, application context, tool results, and readable workspace content can be sent to the selected remote model provider. Choose the workspace and provider accordingly.

To exercise cancellation deterministically, request a turn that will remain active and set a delay:

```sh
cargo run -p lugus-agent --example codex_session -- \
  /tmp/lugus-codex-demo 'Continue reasoning until interrupted.' \
  --cancel-after-ms 500
```

Cancellation sends Codex `turn/interrupt`, waits for a bounded grace period, and reaps the disposable process. A cancelled adapter instance is not reused.

## Runtime boundary

One `CodexRuntime` adapter instance runs one task at a time. The harness returns runtime completion independently of future assessment validation. The runtime adapter itself does not persist assessments. The `reviews` application layer below stores theses, frozen financial evidence and assessment history. Shared memory, automatic scheduling, document capture and durable web research remain later milestones.

Native web search is requested in the read-only Codex configuration, but actual availability depends on the selected model and provider. A public-document lookup can demonstrate that capability; its response remains transient. Capturing source documents and provenance belongs to milestone 3.

Tool lifecycle events describe adapter handling of an accepted flat call. `ToolStarted` is emitted before dispatch, so an unknown flat tool can produce `ToolStarted` and an unsuccessful `ToolFinished` without invoking the application executor. Interruption or a terminal failure after `ToolStarted` can prevent `ToolFinished`; the terminal run result is authoritative. A call rejected by a host limit emits neither lifecycle event because tool execution never began, and the run returns an explicit terminal error.

## Deterministic verification

From the workspace root:

```sh
cargo test -p lugus-agent
cargo clippy -p lugus-agent --all-targets -- -D warnings
cargo fmt -p lugus-agent -- --check
cargo test -p lugus-financial
```

These checks use a local protocol fixture and do not establish that a configured account, provider, or native web search works live.

## Live verification record

On 2026-09-10, the example was run with the user's existing authenticated Codex configuration and its default model/provider. No account identity or credential was captured.

- A fresh empty workspace run called `lugus_context` exactly once. It delivered `Started`, `ToolStarted`, `ToolFinished { success: true }`, text deltas, and a completed report. The final text included the complete fixed demonstration result.
- A separate fresh empty workspace run requested a native lookup of the official Rust `std::time::Duration` documentation. Sanitized protocol metadata recorded `item/started` with `item_type=webSearch`, followed by `item/completed` with `item_type=webSearch` and `action_type=search`. The turn returned the canonical page title and `https://doc.rust-lang.org/std/time/struct.Duration.html`, confirming that native search was available in this configuration. The response was transient and was not captured as evidence.
- A third fresh empty workspace run used `--cancel-after-ms 500`. It delivered `Started` and returned `Cancelled` without tool or text events, confirming interruption and process cleanup through the runnable interface.

The observed initialize, tool callback, text-delta, completion, and interruption shapes matched the pinned protocol fixture; no adapter change was required.


## Durable investment reviews

`reviews::ReviewCoordinator` runs a persisted review through `AgentRuntime`. Its `ReviewStore` port owns thesis revisions, review requests, immutable selected evidence and assessment history. `SqliteReviewStore` implements atomic persistence in a separate agent database. The financial adapter consumes the public financial repository snapshot API; it captures facts, filing metadata and ingestion scope, including historical revisions. It does not fetch documents, capture market-price data, or perform fresh research.

From the workspace root, create a text file containing your thesis, then run:

```sh
cargo run -p lugus-agent --example durable_review -- create ./agent.db thesis-1 ./thesis.txt
cargo run -p lugus-agent --example durable_review -- evidence ./financial.db ./provider.json ./query.json
cargo run -p lugus-agent --example durable_review -- queue ./agent.db review-1 thesis-1 1 ./financial.db ./provider.json ./query.json
mkdir -p /tmp/lugus-review-workspace
cargo run -p lugus-agent --example durable_review -- run ./agent.db review-1 /tmp/lugus-review-workspace
cargo run -p lugus-agent --example durable_review -- show ./agent.db thesis-1
```

`provider.json` and `query.json` contain the existing `lugus_financial::domain::ProviderIdentity` and `Query` JSON values for already-ingested data. Use the provider identity from your ingestion configuration. `evidence` enumerates captured payloads and their IDs. For large snapshots, pass a final `selected-ids.json` argument to `queue`: a JSON array of the exact desired evidence IDs. The CLI also includes the snapshot-scope record. Queueing allows at most 64 records (each at most 64 KiB) and rejects excess data instead of silently truncating it. A scope record that itself exceeds the item bound requires a narrower query.

Every CLI invocation reopens the database. `show` prints the current user thesis, saved assessments and the exact evidence retained with each assessed review; it requires no runtime or financial database. After ingesting changed data, enqueue a new review ID and run it. The new review captures the current prior assessment, while old reviews retain their original inputs. To edit the thesis, use `edit AGENT_DB THESIS_ID EXPECTED_REVISION TEXT_FILE`; original revisions remain preserved. User text is limited to 16 KiB and assessment drafts to 32 KiB; starting runtime context is limited to 64 KiB.

Only `run` calls the configured remote model. It requires an existing empty workspace and uses existing Codex authentication. Stored evidence and thesis text are sent to that model. Native web search is disabled for these runs. `RunRequest.allow_web_search` is an explicit capability; absent serialized values default to false, and the generic Codex example explicitly opts in. The verified Codex session override uses `web_search = "disabled"`, which removes the tool ([official configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference)).

### Completion and recovery

The agent reads selected evidence with `lugus_read_evidence` and persists a strict structured draft with `lugus_submit_assessment`. Lugus validates selected evidence references, expected thesis/history revisions and attempt ownership. Assessment insertion and review completion commit in one transaction. Validation establishes structure and provenance references, not factual correctness; unsupported questions and uncertainty remain visible.

Review IDs are caller-owned idempotency keys. Reusing an ID with different thesis/evidence inputs is rejected. Identical submission retries return the existing assessment; conflicting submissions cannot overwrite it. One review per thesis may run at a time. Runtime completion without a saved assessment is `failed`. Authentication/attention errors are `blocked`; cancellation is `interrupted`. A saved assessment stays `completed` even if the runtime subsequently fails or is cancelled. Changes to the thesis or its prior assessment during execution block stale submission; create a new review request with current inputs.

Use `status AGENT_DB REVIEW_ID` to inspect the persisted lifecycle. Failed, interrupted or authentication-blocked requests can be run again with their frozen inputs. Each attempt receives a new fencing token. `recover AGENT_DB` marks abandoned running reviews interrupted. Invoke recovery only on application startup after previous executors have stopped; merely opening a second database connection never interrupts active work. A process kill or dropped execution future is recovered this way. For an explicit cancellation check, append a delay in milliseconds to `run`, such as `500`.

The coordinator accepts a `Clock`, event channel, cancellation channel and runtime limits. It gives an interrupted runtime up to two seconds to acknowledge cancellation, then bounds `close()` to three seconds. Replacement runtimes must implement bounded cleanup and clean up owned resources on drop. Short synchronous SQLite transactions are never held across an await; busy-lock waits can take up to five seconds, so host coordination off the UI thread. The CLI records a runtime version and indicates that model/provider use configured defaults; exact resolved model identity is not currently exposed by `AgentRuntime`.

### Verification

Deterministic tests use real temporary financial and agent databases and a fake external runtime driving the real scoped tools. They cover reopen and changed-evidence history, incomplete ingestion, explicit selection, atomic rollback, concurrent and duplicate submission, stale revisions/attempts, cancellation before/after commit, dropped-future recovery, error/result bounds and native-search configuration. Run:

```sh
cargo test --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all -- --check
```


On 2026-09-10, all 116 workspace/all-target tests passed, as did strict workspace Clippy and formatting. Independent review covered transaction races, attempt fencing, tool scope and cancellation; its error-response-size finding was reproduced and fixed.

Live validation used only synthetic fixture data in temporary financial/agent databases and an empty runtime workspace with existing Codex authentication. Review A read a stored 100 USD asset fact and persisted a supported assessment. A fresh process/session for review B retrieved that saved assessment and a newly selected 80 USD fact, then persisted a contradicted assessment explaining the change. Offline `show` confirmed both original evidence versions and both assessments. A third run cancelled after 500 ms persisted `interrupted` and left the assessment count at two. CLI create, selection, edit, show, status and recovery were also exercised. These checks establish the local workflow and supported Codex integration, not investment reasoning quality or compatibility with every model/provider.
