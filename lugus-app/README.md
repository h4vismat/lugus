# lugus-app

A framework-independent Rust application host for explicit provider research, durable scoped evidence references, and accepted view requests. Manual clients and the `lugus-agent::ToolExecutor` adapter share the same application operations. The host depends on provider, repository, store, clock and ID ports; JSON configuration, child processes and SQLite are composition adapters.

## Host contract

`Application::start(configured_providers, repositories, store, limits, host_bounds, ids).await` validates configuration, initializes the repository outside the async executor, and starts active instances concurrently. Failed startup leaves that instance configured/unavailable while other instances and local reads work. `ApplicationConfig::load(path).await?.open(offline).await?` supplies the JSON/process/SQLite adapter. Relative paths resolve against the configuration file. Offline opening skips all manifest reads and child startup, even when providers remain listed in the configuration.

The cloneable host exposes:

- `scope(workspace, request, optional_run)`, `offering()`, `providers()` and `subscribe()`.
- `submit_manual(scope, command)` or `submit(scope, captured_offering, command)` returning `JobReceipt`; `status`, async `wait`, and `cancel` take an owned workspace scope and job ID.
- Async `activate(instance)`, `deactivate(instance)` and `shutdown()`. Activation explicitly restarts an existing instance, increments generation, and invalidates old offerings. It never implicitly retries through another provider.
- Async `read_fetch`, `create_dataset`, `dataset_header`, `read_dataset`, `read_document`, `select_candidate`, `open_view`, `read_view`, and `report_presentation` with a scope on every operation.

Transport adapters must supply an authenticated workspace identity to `scope`; arbitrary model arguments never supply scope. `ResearchExecutor::new(application, scope)` captures the host's current offering for one agent turn. `tool_specs()` returns strict stable schemas, and its `ToolExecutor` implementation rejects forged run IDs, unknown fields, and unoffered tools/instances. Cached tools remain available without providers. Fetch tool results contain terminal `JobStatus` and a durable `fetch_id`; call `lugus_read_fetch` to obtain exact runs before creating a dataset. Neither raw run registration nor renderer reports are agent tools.

A host admission lock orders submit, restart, deactivation and shutdown. An admitted lifecycle task owns its per-instance permit through cleanup even if its caller is dropped; callers cancelled while waiting for that permit start no work. Shutdown has one shared supervised operation and cached completion result, so dropping or repeating a shutdown wait cannot discard cleanup or hide a prior cleanup failure. Initial construction also retains startup ownership until the caller acknowledges receipt of the host. The same catalog boundary orders worker dispatch and cancellation checks. Shutdown fences submissions before awaiting cleanup; concurrent startup cannot publish after that fence. Provider exchanges are cancelled inside ingestion so committed pages and finalization survive. Host supervision persists a fetch receipt before publishing one authoritative terminal status. Persistence failure produces `Failed`; late cancellation cannot replace committed success. Dropping an agent tool wait signals its job while the host continues cleanup. Call `shutdown().await` before dropping the host or stopping its Tokio runtime to wait for child reaping and terminal persistence. Custom provider factories must bound startup/close, and custom repository/store ports must bound their synchronous work.

`HostBounds` separately limits pending jobs, retained terminal jobs and event capacity (defaults 256 each; all positive, at most 100,000). Full pending capacity rejects admission before provider effects. Terminal retention evicts the oldest completed job only; a waiter already subscribed retains its completion. Evicted status returns `MissingData`. Durable fetch/dataset/view IDs survive eviction and process restart. Events use nonblocking, bounded broadcast: lag or stream closure never means completion. Consult `status` or `wait`.

Every application-store call runs in `spawn_blocking` with a dedicated short store mutex. No SQLite transaction spans provider awaits. Deadlines start at worker admission and include queue time. `Limits` bounds queues, active execution, duration, pages, total items/bytes, documents, input, read pages and output. Manual local inputs and outputs are preflight bounded; agent output limits count the complete serialized `ToolResult`, including escaped JSON content. Oversized output returns a complete safe JSON error, never truncated JSON. A fetch may commit successfully even if its tool response exceeds the output budget; its authoritative job remains observable through host events/status. Raising output limits may be needed to inspect an unusually large durable receipt. Decimal evidence values remain strings.

`RandomIds::new()` obtains 256 bits of OS entropy, then generates IDs from that namespace and a non-wrapping atomic counter. Entropy failure prevents construction. Use separate sources safely across store/host reopening; custom `IdSource` adapters must preserve global uniqueness across restarts.

View acceptance is persisted and idempotent for identical workspace/request input. Changed input conflicts. Acceptance has no presentation result until a trusted renderer separately reports a matching view ID and descriptor revision. Original document reads verify source, provider and checksum; they expose original bytes, not extracted passages.

## Deterministic CLI acceptance

Requires Rust and Python 3; no model credentials, network or installed financial plugin is needed. From the repository root, choose a new empty directory:

```sh
python3 lugus-app/examples/synthetic/setup.py /tmp/lugus-research-demo
cargo build -p lugus-app --bin lugus-research --offline
```

The setup writes external configuration, finite limits, a manifest for the real synthetic protocol peer, two healthy instances, an unavailable instance, explicit request files, and thesis text. The company CIK and market instrument are independent explicit inputs; no company-to-market association is inferred.

```sh
target/debug/lugus-research thesis-create /tmp/lugus-research-demo/thesis.sqlite fixture-thesis /tmp/lugus-research-demo/thesis.txt
target/debug/lugus-research fetch /tmp/lugus-research-demo/config.json workspace manual-filings /tmp/lugus-research-demo/filings.json
target/debug/lugus-research fetch /tmp/lugus-research-demo/config.json workspace unavailable-check /tmp/lugus-research-demo/unavailable.json
target/debug/lugus-research agent-fixture /tmp/lugus-research-demo/config.json workspace /tmp/lugus-research-demo/thesis.sqlite fixture-thesis /tmp/lugus-research-demo/prices.json
```

The agent command loads that actual persisted thesis revision and invokes a deterministic implementation of `AgentRuntime`. It executes five real host-bound tool calls: fetch → read durable fetch → create exact dataset → read page/original → accept view. JSON output includes thesis, run/scope, full provider provenance, durable IDs, decimal-string rows, and the accepted view with `presentation: null`. The fixture supports filings, prices, document and company-resolution commands. It does not simulate a model decision, persist a conversation, create a financial assessment, or claim rendered UI.

Copy `fetch_id` from the manual output and `receipts.dataset.id`/`receipts.view.id` from the agent output to reopen them offline:

```sh
target/debug/lugus-research read-fetch /tmp/lugus-research-demo/offline.json workspace offline FETCH_ID
target/debug/lugus-research read /tmp/lugus-research-demo/offline.json workspace offline DATASET_ID 0 1
target/debug/lugus-research read-view /tmp/lugus-research-demo/offline.json workspace offline VIEW_ID
```

All local CLI commands open offline automatically. They also work with the original config if its manifest/plugin files are removed. Additional commands follow `COMMAND CONFIG WORKSPACE REQUEST ...`: `dataset FETCH_ID PROJECTION_JSON`, `header DATASET_ID`, `document DATASET_ID OFFSET LENGTH`, `view DATASET_ID price_chart|data_table|document`, and `select DATASET_ID OBSERVATION_ID`. `cargo run -p lugus-app --example research --offline -- ...` provides the same CLI. Operational fetch failures are structured JSON with stable error kinds; malformed CLI calls exit nonzero with a safe diagnostic. Each invocation explicitly shuts down its host before returning.

## Verification and remaining scope

```sh
cargo test -p lugus-app --all-targets --offline
cargo clippy -p lugus-app --all-targets --all-features --offline -- -D warnings
cargo fmt -p lugus-app --check
```

Integration tests exercise real Python children and SQLite: dispatch cancellation, partial-run finalization, scope/run injection, strict nested input, activation during turns, independent instances, startup/shutdown races, bounded queues/registry/events/results, storage failure, blocking-store isolation, reference reopening and the actual CLI sequence. Financial selection and provider protocol semantics remain in `lugus-financial`.

Company-to-market binding, general conversation identity and persistence, passage extraction, background refresh, live model acceptance and desktop rendering remain outside this milestone. The existing `AgentRuntime` requires a real thesis ID; the CLI intentionally enforces that boundary.
