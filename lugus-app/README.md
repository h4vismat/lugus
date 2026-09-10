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

General conversation identity and persistence, passage extraction, background refresh, live model acceptance and desktop rendering remain outside this milestone. The existing `AgentRuntime` requires a real thesis ID; the CLI intentionally enforces that boundary.
