# Lugus roadmap and guidance for future sessions

Last updated: 2026-09-10. This document records agreed direction, completed work, remaining milestones, and the recommended next task. It is a roadmap, not an executable implementation plan.

## Start here

The current milestone is **durable conversations and research workspaces**, in progress on local branch `codex/durable-conversations` in `/private/tmp/lugus-backend-development`. The user approved its design; follow `docs/superpowers/plans/2026-09-10-durable-conversations.md` and its execution ledger rather than restarting implementation. Company-to-market-instrument binding is implemented on local branch `codex/instrument-binding` in worktree `/private/tmp/lugus-backend-development`, through `20d5fb8`. This branch includes the completed production application runtime and company-resolution/observation foundations. It remains local and unmerged. Read the application README and completed binding plan before starting the next milestone.

First check repository/branch state. Company resolution and observation selection were implemented in `eea83bf`; application runtime and lifecycle fixes extend through `403aa1d`, with its completion documentation at `781a539`. Binding work builds on that commit. These features have not been merged into `main`; do not infer missing implementation from a checkout of `main`, repeat the work, or overwrite pending documents. Temporary worktree paths may change; branch and commit identify the implementation.

Integration into `main` is a separate action; this document does not authorize publishing, pushing, or merging.

## Established product and architectural decisions

- Lugus is a desktop research workspace combining conversation with persistent financial views. A workspace belongs to a conversation and may cover several companies.
- The layout has a left sidebar for recent conversations and pinned work, a conversation pane with a bottom composer, and an adjacent resizable, tabbed research area.
- Users and agents can open the same research views. Data access does not require sending a message first.
- Agents may fetch fresh data through activated capabilities without routine confirmation. Open views refresh automatically only for the active workspace.
- Background data refresh does not start model turns, rerun assessments, or alter saved evidence. Updates preserve the user's reading position and view selections.
- Initial views: company overview with a basic historical chart, reported fundamentals, filing list/reader, and a minimal thesis/review history view.
- Defer plugin management UI, advanced charting, configurable dashboards, and dedicated comparison views.
- Tauri with a TypeScript frontend is the preferred direction, pending desktop-host validation. Core financial and application logic stays independent of that choice.
- Resolution is a plugin capability. Initial discovery uses SEC data; future plugins add international discovery. Preserve company, listing/security, and provider-instrument distinctions.
- Favor pure, versioned domain functions and explicit interfaces around storage, network, clocks, and process effects. Avoid provider or model-vendor lock-in.

## Completed foundations

### Financial ingestion and agent harness

Existing financial capabilities provide filings, reported fundamentals, primary-document retrieval/storage, and daily market history through external provider processes. Evidence retains source identity, exact values, revisions, and retrieval associations. Offline reads do not perform network requests.

The agent harness supplies disposable turns, tool execution boundaries, lifecycle events, cancellation, and durable explicitly requested investment reviews. Reviews preserve selected fundamentals, filing metadata, and ingestion scope. They do not yet capture market prices or document passages as durable review evidence.

### Capability routing spike

The disposable experiment verified known capability → agent tool → financial ingestion/storage → structured research-view request, including a live agent run. Its extension verified routing between two provider processes and selected plain-text document context that retained the original version after a newer same-URL document arrived.

Eight deterministic spike tests and live probes passed. These are feasibility results, not a production application host or a visible desktop renderer. Arbitrary plugin-provided UI, real HTML/PDF passage extraction, and durable conversation state were not established.

### Company resolution and observation selection

Implemented in `eea83bf`:

- Optional `company_resolution:1` capability, with the SEC plugin at version `0.2.0`.
- Cashtags such as `$IBM` and `$PLTR`, bare-ticker-first search, and explicit CIK lookup. Name fallback occurs only after a successfully exhausted empty ticker search.
- Durable company catalog, source/provenance history, ambiguity handling, and explicit offline candidate choices.
- Run-scoped financial reads and deterministic daily-price/fundamental selection, including exact decimal equivalence and explicit same-date conflicts.
- Persistent repository identity, monotonic ingestion chronology, frozen selection coverage, and additive financial schema migrations through v4.
- CLI examples for resolution, offline catalog use, and observation inspection.

Verification: 145 workspace/all-target Rust tests, 31 SEC Python tests, 12 yfinance Python tests, strict Clippy, and formatting passed. Independent review findings were fixed and re-reviewed. Resolution CLI flows passed with a real synthetic provider process. Live SEC retrieval was not exercised during this implementation verification.

Company resolution does **not** yet establish a market-provider instrument mapping. Observation selection does **not** assemble complete financial statements, derive quarters/TTM, reconcile providers, or claim public point-in-time accuracy.

## Milestones, in recommended order

### 1. Production application runtime and capability routing — complete

Implemented in `lugus-app` through `403aa1d`:

- Shared scoped manual commands and a strict, host-bound `ToolExecutor`, with immutable turn offerings and live provider-generation checks.
- Independently managed provider workers with bounded queues, job retention, deadlines, cancellation, explicit process reaping, and lifecycle cleanup that survives cancelled callers.
- Durable fetch receipts, frozen exact-run datasets, bounded offline pages/documents, scoped candidate selection, and transactional view acceptance separate from renderer presentation.
- External JSON configuration and a CLI that invokes `AgentRuntime` against a real persisted thesis. Native market instruments remain explicit; no company-to-market mapping is inferred.

Verification: **243 workspace/all-target Rust tests**, strict workspace/all-target/all-feature Clippy, formatting, **31 SEC Python tests** and **12 yfinance Python tests** passed. Independent foundation and integration reviews were completed; confirmed findings were fixed and re-reviewed. A fresh standalone CLI run verified two real synthetic provider instances, explicit unavailable-provider behavior, exact decimal prices, accepted views with no presentation claim, and offline reopening after removing the provider manifest. No live model or provider-network call was required.

See [`lugus-app/README.md`](../../lugus-app/README.md) for the public API, limits and reproducible CLI commands. Job status is bounded and in-memory; durable fetch/dataset/view references survive restart. Conversation recovery, automatic refresh and desktop rendering remain later milestones.

**Exit met:** manual and actual `AgentRuntime` calls use the same production application operations, with explicit scope/provenance and correct unavailable-provider behavior.

### 2. Company-to-market-instrument binding — complete

Implemented through `20d5fb8`:

- Optional provider-neutral `instrument_lookup:1`, immutable exact lookup evidence, additive financial schema v5, and yfinance adapter 0.2.0 with the dependency still pinned to 1.7.0.
- Agent-created bindings from an exact saved company observation and selected listing plus market-provider metadata. Pure `instrument-binding-v1` rules require concordant issuer identity, ticker, known exchange and equity type; accepted bindings are classified `source_supported`. Clear matches proceed without routine confirmation. Missing/conflicting evidence cannot create a binding, and no default listing or provider is chosen.
- Workspace-scoped immutable binding records, transactional request deduplication, supersession/revocation history, bounded offline reads and additive application schema v2. Multiple bindings remain explicit.
- Shared manual/agent lookup, create/read/list/history/revoke and bound-price operations. Bound fetches derive the exact provider instance/version and native instrument from successful scoped preparation, retain live-generation checks, and preserve binding provenance through success, failure and cancellation. Old datasets retain their original association after later transitions.
- A twelve-tool actual `AgentRuntime` acceptance workflow against a real saved thesis: resolve Apple → discover AAPL from SEC listing evidence → source lookup → automatic binding → prices → frozen dataset → accepted chart.

Verification: **278 workspace/all-target Rust tests**, strict workspace/all-target/all-feature Clippy, formatting, **31 SEC Python tests** and **23 yfinance Python tests** passed. Task reviews completed; the binding response-capacity finding was fixed and re-reviewed. Whole-branch review found no Critical or Important issues. One nonblocking CLI assertion-strengthening follow-up is recorded in the completed plan.

A fresh standalone CLI run verified automatic Apple/AAPL binding, exact price/provenance, four refused adversarial cases, supersession from returned references, refusal of a new fetch using the old binding, and offline binding/history/dataset/view reads plus revocation after removing the generated manifest. No live SEC/Yahoo/model request was used. The pinned SDK's source-identity behavior was tested offline against real library transformations.

Source detail: yfinance `get_info()` overwrites the source symbol with the requested symbol. Lookup uses bounded public history and selected source chart metadata instead; an echoed request or successful price fetch is not identity evidence. Version 1 supports explicit SEC NASDAQ/NYSE and Yahoo venue mappings, preserves ticker punctuation and legal-name words, and makes no universal or historical security-identity claim. See the lookup protocol and application README for limits, including bounded tool responses after durable mutations.

**Exit met:** agents and manual clients can select supported company/listing evidence, create a binding, fetch prices through its pinned provider, and retain that association offline without silently resolving ambiguity.

### 3. Durable conversations and research workspaces

Persist conversations, messages, agent runs, selected context, and research-view references. Reconstruct bounded context across disposable turns and restore workspaces after restart without replaying completed model turns.

Separate conversation identity from the generic harness's currently required thesis identity. Integrate startup recovery and shutdown with the application host. Saved messages, view references, and assessments must retain their original evidence meaning after refresh.

**Exit:** ask a question, obtain a research-view request, restart the host, reopen the conversation and its views, and continue with the intended context.

### 4. Filing text and passage references

Extract readable content from HTML/PDF filings while retaining original document checksum, extraction version, text-representation identity, and mappings to source locations.

Bind a user's selection to the exact document/text version and pass a bounded reference into an agent turn. The plain-text spike does not validate HTML/PDF offsets or extraction fidelity. Conversation passage context and durable review evidence capture remain distinct concerns.

**Exit:** ask about a selected passage in a real-format fixture, follow its source location, and verify that a newer document revision does not reinterpret the old selection.

### 5. Active-workspace background refresh

Implement freshness policies, request deduplication, provider rate-limit handling, bounded retries/cancellation, and workspace-switch behavior. Schedule automatic refresh only for the active workspace; associate late results with their origin.

Keep data refresh independent of agent execution and immutable review history. Return enough state for the UI to preserve chart ranges, table selections, and document position.

**Exit:** fake-clock integration tests verify active/inactive workspace scheduling, overlapping requests, late results, failures, offline/stale data, and unchanged saved evidence.

### 6. Desktop integration

Validate Tauri/TypeScript against the established backend contracts, then implement the approved layout and initial research views. Include the minimal thesis/review workflow supported by the existing durable-review backend.

This milestone owns desktop transport, rendering, keyboard/focus behavior, view-state restoration, and packaging. Financial selection rules and provider orchestration remain in the backend.

**Exit:** the approved research workflow works in the desktop application and has been checked in the running UI, including loading, failure, offline, cancellation, and restoration states.

## Backend acceptance workflow before desktop implementation

Resolve `$IBM` → select its market instrument → fetch available data through configured capabilities → ask questions → request structured research views → save and reopen the conversation → refresh only the active workspace → inspect an earlier review with its original evidence.

This full workflow is not yet implemented. The application runtime and company-to-market binding are complete; durable conversations, passage extraction, active-workspace refresh and desktop integration remain. The deterministic agent acceptance uses a real thesis because generic conversation identity is still milestone 3.

## Reading list

Paths are relative to the repository root; inspect the implementation branch when a file is absent from the current checkout.

- `docs/superpowers/specs/2026-09-10-durable-conversations-design.md` — approved conversation persistence/recovery boundaries.
- `docs/superpowers/plans/2026-09-10-durable-conversations.md` — current milestone execution plan.
- `docs/superpowers/specs/2026-09-10-instrument-binding-design.md` — approved automatic source-supported binding design.
- `docs/superpowers/plans/2026-09-10-instrument-binding.md` — completed binding implementation and verification plan.
- `docs/superpowers/specs/2026-09-10-application-runtime-design.md` — approved runtime boundaries.
- `docs/superpowers/plans/2026-09-10-application-runtime.md` — completed implementation and verification plan.
- `docs/superpowers/specs/2026-09-10-desktop-research-design.md` — approved product scope; this roadmap supersedes its older interleaved delivery sequence.
- `docs/superpowers/specs/2026-09-10-research-application-contracts.md` — proposed application boundaries; review against current code before writing the runtime implementation plan.
- `docs/superpowers/specs/2026-09-10-company-resolution-design.md` — approved resolution design.
- `docs/superpowers/specs/2026-09-10-observation-selection-design.md` — approved selection rules and explicit limits.
- `docs/superpowers/plans/2026-09-10-backend-resolution-selection.md` — completed implementation plan on the feature branch.
- `docs/superpowers/spikes/2026-09-10-capability-routing/README.md` — disposable prototype findings and limits.
- `lugus-app/README.md` — shared host, scoped tools, lifecycle guarantees, bounds and deterministic CLI acceptance.
- `lugus-financial/README.md` — financial APIs, resolution/selection examples, migration and verification notes on the feature branch.
- `lugus-agent/README.md` — runtime and durable-review guarantees and limitations.
