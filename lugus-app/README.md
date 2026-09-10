# lugus-app

A framework-independent Rust application host for explicit provider research, durable scoped evidence references, and accepted view requests. Manual clients and the `lugus-agent::ToolExecutor` adapter share the same application operations. The host depends on provider, repository, store, clock and ID ports; JSON configuration, child processes and SQLite are composition adapters.

## Host contract

`Application::start(configured_providers, repositories, store, limits, host_bounds, ids).await` validates configuration, initializes the repository outside the async executor, and starts active instances concurrently. Failed startup leaves that instance configured/unavailable while other instances and local reads work. `ApplicationConfig::load(path).await?.open(offline).await?` supplies the JSON/process/SQLite adapter. Relative paths resolve against the configuration file. Offline opening skips all manifest reads and child startup, even when providers remain listed in the configuration.

The cloneable host exposes:

- `scope(workspace, request, optional_run)`, `offering()`, `providers()` and `subscribe()`.
- `submit_manual(scope, command)` or `submit(scope, captured_offering, command)` returning `JobReceipt`; `status`, async `wait`, and `cancel` take an owned workspace scope and job ID.
- Async `create_binding`, `read_binding`, `list_bindings`, `revoke_binding`, and `binding_history` with scoped saved evidence. `fetch_bound_prices_manual(scope, &request)` or `fetch_bound_prices(scope, captured_offering, &request)` returns an ordinary supervised job.
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

The setup writes external configuration, finite limits, a manifest for the real synthetic protocol peer, two healthy instances, an unavailable instance, explicit request files, and thesis text. Direct request files retain explicit native instrument input. The additional `binding.json` workflow resolves Apple and automatically creates a source-supported association from saved SEC listing and market instrument evidence.

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

All local CLI read/mutation commands open offline automatically; `fetch` and `bound-prices` start configured providers. They also work with the original config if its manifest/plugin files are removed. Additional commands follow `COMMAND CONFIG WORKSPACE REQUEST ...`: `dataset FETCH_ID PROJECTION_JSON`, `header DATASET_ID`, `document DATASET_ID OFFSET LENGTH`, `view DATASET_ID price_chart|data_table|document`, and `select DATASET_ID OBSERVATION_ID`. `cargo run -p lugus-app --example research --offline -- ...` provides the same CLI. Operational fetch failures are structured JSON with stable error kinds; malformed CLI calls exit nonzero with a safe diagnostic. Each invocation explicitly shuts down its host before returning.

## Automatic agent instrument bindings

Run the generated binding workflow against the actual stored thesis:

```sh
target/debug/lugus-research agent-fixture /tmp/lugus-research-demo/config.json workspace /tmp/lugus-research-demo/thesis.sqlite fixture-thesis /tmp/lugus-research-demo/binding.json > /tmp/lugus-research-demo/binding-result.json
```

`binding.json` explicitly selects company instance `filings`, market instance `market`, input `Apple`, native namespace `yahoo:symbol`, dates and page size. The deterministic runtime performs twelve actual tool calls: resolve → read company fetch → freeze the terminal resolution run → read candidates/listings → look up the discovered native symbol → read lookup evidence → create binding → fetch bound prices → read price fetch → freeze prices → read prices → accept chart. It requires exactly one candidate and one listing, refusing ambiguity before lookup; manual clients and agents can explicitly select any retained listing through `create_binding`. It never selects the first listing from an ambiguous set. The native symbol is a lookup candidate derived without punctuation conversion; acceptance comes from saved source identity evidence.

Output includes `receipts.company_dataset`, `companies`, `instrument_fetch`, `binding`, `fetch`, `dataset`, `page`, and `view`. `binding.policy` is `instrument-binding-v1`; its reasons classify acceptance as `source_supported`. The exact binding ID appears in the price fetch and frozen dataset, and the view points to that dataset. Binding scope contains the host-owned run/request IDs. No per-binding confirmation is required.

The generated `wrong-issuer.json`, `wrong-exchange.json`, `missing-evidence.json`, and `multiple-listings.json` configurations are adversarial alternatives to `config.json`. Run the same `agent-fixture` command with each: all exit nonzero without creating a binding. Multiple listings require explicit selection; missing or contradictory identity evidence does not become verified by price retrieval.

To demonstrate supersession and offline provenance using only returned references:

```sh
python3 - <<'PYDATA'
import json
from pathlib import Path
root = Path('/tmp/lugus-research-demo')
b = json.loads((root / 'binding-result.json').read_text())['receipts']['binding']
(root / 'supersede.json').write_text(json.dumps({
    'company_dataset_id': b['company_dataset_id'],
    'company_observation_id': b['company']['observation_id'],
    'listing': b['listing'], 'instrument_fetch_id': b['instrument_fetch_id'],
    'supersedes': b['id'],
}))
(root / 'bound-prices.json').write_text(json.dumps({
    'binding_id': b['id'], 'start': '2024-01-01', 'end': '2024-01-03', 'page_size': 10,
}))
PYDATA
target/debug/lugus-research bind /tmp/lugus-research-demo/offline.json workspace supersede /tmp/lugus-research-demo/supersede.json
target/debug/lugus-research bound-prices /tmp/lugus-research-demo/config.json workspace old-binding /tmp/lugus-research-demo/bound-prices.json
```

The supersession returns a new binding ID; fetching the old binding returns a structured `conflict`. After all CLI hosts have shut down, remove the generated manifest to prove no provider is needed for these reads (replace uppercase IDs with the original result IDs):

```sh
rm /tmp/lugus-research-demo/manifest.json
target/debug/lugus-research read-binding /tmp/lugus-research-demo/offline.json workspace inspect BINDING_ID
target/debug/lugus-research binding-history /tmp/lugus-research-demo/offline.json workspace history BINDING_ID 0 10
target/debug/lugus-research read /tmp/lugus-research-demo/offline.json workspace old-data DATASET_ID 0 1
target/debug/lugus-research read-view /tmp/lugus-research-demo/offline.json workspace old-view VIEW_ID
```

The original binding history contains active and superseded events; the original dataset keeps its original binding ID. Additional local commands are `list-bindings OFFSET LIMIT` and `revoke-binding BINDING_ID`. `bind REQUEST_JSON` accepts company dataset/observation IDs, an exact selected listing, instrument fetch ID, and optional `supersedes`. Agent `lugus_create_binding` wraps these fields in `request`; other binding tools use `binding_id`, with `page` for list/history. `lugus_fetch_bound_prices` and manual bound-price JSON accept only binding ID, start/end dates and page size. Raw evidence, actor/scope replacement, provider/instrument replacement, and verification flags are rejected.

The host prepares an active scoped binding before admission, checks its exact provider instance/plugin/version against the captured offering, then uses existing live-generation authorization and supervised fetching. A same-version restart works in a fresh turn. Provider version changes require new lookup evidence and an explicit new binding. Already-prepared work may finish after revocation or supersession; its terminal success, failure or cancellation retains the original binding ID. Full or unavailable admission never fabricates a successful fetch. Binding creation can persist before a larger agent response envelope exceeds its output limit, as with other durable operations; retrying the same tool call is idempotent and bindings remain inspectable through bounded local reads/listing.

Version 1 supports equity, exact ticker agreement preserving punctuation, explicit SEC NASDAQ/NYSE and Yahoo NMS/NGM/NCM/NASDAQ/NYQ/NYSE venue mappings, and concordant issuer identifiers or normalized full names/retained aliases. Unknown venues or missing identity remain incomplete; contradictions conflict. The policy is source-supported evidence agreement, with no universal security-master or historical identity claim, automatic provider failover, or background revalidation.

## Verification and remaining scope

```sh
cargo test -p lugus-app --all-targets --offline
cargo clippy -p lugus-app --all-targets --all-features --offline -- -D warnings
cargo fmt -p lugus-app --check
```

Integration tests exercise real Python children and SQLite: dispatch cancellation, partial-run finalization, scope/run injection, strict nested input, activation during turns, independent instances, startup/shutdown races, bounded queues/registry/events/results, storage failure, blocking-store isolation, reference reopening and the actual CLI sequence. Financial selection and provider protocol semantics remain in `lugus-financial`.

`AgentRuntime` uses an explicit `RunSubject::Thesis` or `RunSubject::Conversation`.
The legacy `agent-fixture` intentionally requires a real persisted thesis. Durable
conversations use application-owned storage and need no thesis or assessment.
Passage extraction, background refresh, live model acceptance, and desktop rendering
remain outside this milestone. Document observations retain original evidence metadata;
they do not imply extracted filing text.

## Durable conversation CLI

The same binary and example expose `conversation COMMAND CONFIG ...`. These commands
compose the shared `ConversationHost` and `Application` APIs. A conversation owns one
workspace, which may cover several companies. Creating a conversation leaves its
messages, runs, and tabs empty. Existing manual fetch/dataset/view commands can use
its returned `workspace_id` to open views before any conversation turn.

Start with a fresh directory using `examples/synthetic/setup.py` as above. In addition
to the legacy files, setup writes `conversation-create.json`,
`conversation-research.json`, and `blocked.json`. No model credentials or network
are needed; the explicit fixture JSON selects a deterministic `AgentRuntime` adapter.
The research mode reuses the same twelve real source-supported Apple/AAPL tools as
`agent-fixture`, without manufacturing a thesis or seeding a binding.

```sh
target/debug/lugus-research conversation create /tmp/lugus-research-demo/config.json /tmp/lugus-research-demo/conversation-create.json > /tmp/lugus-research-demo/conversation.json
python3 - <<'PYINPUT'
import json
from pathlib import Path
root = Path('/tmp/lugus-research-demo')
c = json.loads((root / 'conversation.json').read_text())
(root / 'conversation-send.json').write_text(json.dumps({
    'conversation_id': c['id'], 'request_id': 'research-1',
    'text': 'Research Apple/AAPL with source-supported evidence.', 'selected': [],
}))
PYINPUT
target/debug/lugus-research conversation send /tmp/lugus-research-demo/config.json /tmp/lugus-research-demo/conversation-send.json /tmp/lugus-research-demo/conversation-research.json
```

`send` emits newline-delimited JSON: a flushed `{"event":"admitted","run":...}`
receipt and, after supervised cleanup, `{"event":"terminal","run":...}`. Save
`run.id` and `run.conversation_id`. A terminal record may be `completed`, `failed`,
or `interrupted`; successful CLI exit means the command was handled, so inspect
`run.status`. Admission errors exit nonzero with bounded JSON on stderr
(`error.kind`, `error.message`, `error.retryable`). No raw provider diagnostics appear.
Reusing an identical `request_id` returns its original receipt without replaying tools;
changing a request with the same ID conflicts. A fresh accepted turn creates a fresh
runtime with the host's immutable offering and derived workspace/run/call scope.

Replace `C`, `R`, `D`, and `V` below with returned conversation/run/dataset/view IDs:

| Command after `conversation` | Result |
| --- | --- |
| `create CONFIG FILE` | Conversation; strict file has `request_id`, `title` |
| `list CONFIG OFFSET LIMIT` | Conversation page |
| `show CONFIG C` | Conversation identity and workspace ID |
| `messages CONFIG C OFFSET LIMIT` | Message page, chronological `items` |
| `runs CONFIG C OFFSET LIMIT` | Durable run page |
| `status CONFIG C R` | Durable run record |
| `workspace CONFIG C` | Revision, ordered `view_ids`, selected view |
| `context CONFIG C R` | Immutable input snapshot, pinned references, omission count |
| `activity CONFIG C R OFFSET LIMIT` | Durable activity page |
| `tools CONFIG C R OFFSET LIMIT` | Exact tool intents and outcomes |
| `dataset CONFIG C D OFFSET LIMIT` | Original scoped dataset page and provenance |
| `view CONFIG C V` | Original scoped accepted view receipt |
| `layout CONFIG C FILE` | Revised workspace layout |
| `send CONFIG REQUEST_JSON FIXTURE_JSON` | Admitted and terminal run records |
| `continue CONFIG REQUEST_JSON FIXTURE_JSON` | Fresh offline continuation fixture |
| `recover CONFIG` | `{"recovered":true}` after exclusive recovery and cleanup |

All pages expose `items` and `next_offset`, except dataset pages which expose `rows`.
Use bounded pages (for example `0 10`) until `next_offset` is null. Reads, create,
layout, and explicit recovery open offline even with the original config after its
manifest has been removed. Ordinary reads never acquire execution ownership or
recover a run. Dataset/view reads enforce the conversation workspace's scope.

The completed research assistant message's `text` is JSON containing `subject_kind`,
`answer`, and concise exact IDs under `receipts.company_dataset`, `instrument_fetch`,
`binding`, `fetch`, `dataset`, and `view`. Full provider results remain in the durable
tool journal: `intent.arguments` and `outcome.result.content` are JSON strings. A view
receipt records acceptance, not desktop presentation.

For a fresh offline follow-up, write another send request with a new `request_id`
and `selected: [{"kind":"dataset","id":"D"}]`. Its fixture file is
`{"mode":"continue","expected_message_ids":["USER_MESSAGE_ID","ASSISTANT_MESSAGE_ID"],"expected_dataset_id":"D"}`.
Use the exact prior IDs from `messages`, in chronological order, and pass both files
to `continue`. This new runtime checks the prior messages and selected frozen dataset
in the host-provided context; it executes zero tools. It also works after deleting
the generated manifest. The original run's tool count stays twelve. Explicit
selection can also include `{"kind":"view","id":"V"}` or an active binding.
Pinned history retains original source references after later binding supersession,
a different dataset, or closing a tab. The context policy includes a bounded suffix
of whole exchanges and reports `omitted_messages`; it creates no automatic summary.
Prior messages and evidence are user data, separate from trusted instructions.

Layout JSON uses an expected revision, for example
`{"expected_revision":1,"mutation":{"operation":"close","view_id":"V"}}`.
Other mutations are `select` with `view_id` and `reorder` with the complete `view_ids`
array. Stale revisions conflict. Closing a tab preserves its original view and data.

Cancellation belongs to the live `send`/`continue` process. After its admission
receipt, write one JSON line to **that process's stdin**:
`{"command":"cancel","conversation_id":"C","run_id":"R"}`.
It validates both IDs against its receipt, signals the owning host, and waits for
cleanup before emitting the terminal result. Invalid controls emit
`{"event":"control_error","error":...}` without canceling. Lines are limited to
4096 bytes including their newline; an oversized line closes the control reader.
EOF merely closes controls and never auto-cancels. An idle open stdin never keeps a
finished command alive. Independent CLI processes can inspect a live run, but v1 has
no interprocess control service for canceling another owner.

Conversation stdout and stderr use one dedicated output worker, a queue of at most
two frames, and a 16 MiB bound per complete serialized frame including its newline.
The worker owns duplicated OS handles; a stalled pipe never blocks the async host,
control handling, deadlines, or cleanup. Under output backpressure, `control_error`
notifications may be dropped; invalid controls still have no effect on the run.
After awaited host cleanup releases execution ownership, final enqueue and flush
share a one-second delivery deadline. A stalled or broken stream causes nonzero
exit and may leave incomplete NDJSON or no final diagnostic; no worker is joined
indefinitely. A broken stream detected during execution triggers awaited host
shutdown, with a safe error on stderr when it remains writable. Durable run status
is authoritative even if final output is lost: inspect `status`, or locate the
original `request_id` in `runs` when the admission receipt was not delivered.
Output failure never authorizes replay of a completed turn.

For a real crash demonstration, submit a new request using `blocked.json` and the
research fixture. Poll `tools` until eight records exist: the first seven completed,
with the eighth bound-price call's `outcome` and `finished_at` still null. Kill only
the spawned CLI owner. The synthetic child detects pipe EOF and exits. Offline
`status` still says `running`; `recover` explicitly reconciles it to `interrupted`
and leaves that pending tool outcome unknown (null). Recovery does not create a
runtime, retry a tool, add messages, or modify completed earlier runs. Partial text
is retained in runtime activity (`data` encodes a `text_delta` event), never promoted
to a completed assistant message. Submit a new request using `config.json` to execute
again. A second owner or recovery process conflicts while the first owner is alive.

Execution uses an OS lease beside the canonical application database,
`<canonical-db>.conversation.lock`; never delete that stable lock file. Ownership is
held until all supervisors, tools, and storage finalization finish. Supported scope
is local SQLite on local filesystems, with symlink aliases canonicalized; hard-link
aliases and network filesystem locking semantics are unsupported.

External request/config files are limited to 1 MiB and reject unknown fields. Shared
conversation limits independently bound complete serialized messages, context,
selected references, journal/activity, pages, open views, active runs, and shutdown.
Defaults include a 60-second turn deadline, 128 tool calls, 16 KiB user messages,
32 KiB assistant messages, 256 KiB context, and a 5-second runtime-close timeout.
Application/tool/page budgets also apply, including JSON escaping and envelopes;
the generated sample's 32 KiB application output cap can constrain a large context
before the default context cap. `conversation_limits` in application config uses
serde defaults and must match stored limits once configured; changing durable
limits requires a future explicit migration.
