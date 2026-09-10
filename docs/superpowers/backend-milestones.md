# Lugus roadmap and guidance for future sessions

Last updated: 2026-09-10. This document records agreed direction, completed work, remaining milestones, and the recommended next task. It is a roadmap, not an executable implementation plan.

## Start here

The next milestone is **company-to-market-instrument binding**. Determine and approve the validation source/contract before implementation; an SEC ticker match alone must not create a market-provider association.

Production application runtime and capability routing are implemented on local branch `codex/application-runtime` in worktree `/private/tmp/lugus-backend-development`, through implementation/fix commit `403aa1d`, based on `eea83bf`. The branch remains local and unmerged. Read the application README and completed plan before starting the next milestone.

First check repository/branch state. Company resolution and observation selection were implemented in commit `eea83bf` on branch `codex/company-resolution-observations`. The application-runtime branch includes that commit; neither feature has been merged into `main`. Do not infer missing implementation from a checkout of `main`, repeat the work, or overwrite pending documents. Temporary worktree paths may change; the branch and commit identify the implementation.

The application-runtime implementation includes that foundation. Integration into `main` is a separate action; this document does not authorize publishing, pushing, or merging.

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

### 2. Company-to-market-instrument binding

Connect a resolved company to a selected market provider's instrument using explicit mappings and validation. Preserve mapping provenance, multiple listings/share classes, ambiguity, and history.

Resolving `$IBM` to an SEC entity enables company identification. An SEC ticker match alone must not silently create a `yahoo:symbol` association. Determine the validation source/contract before implementation; explicit user-declared mappings may be represented honestly as such.

**Exit:** select the intended company and instrument, fetch its prices through the selected provider, and retain the evidence for that association. Ambiguity cannot silently choose a listing or provider.

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

This full workflow is not yet implemented. The application runtime is complete; company-to-market binding, durable conversations, passage extraction, active-workspace refresh and desktop integration remain. The deterministic agent acceptance uses a real thesis because generic conversation identity is still milestone 3.

## Reading list

Paths are relative to the repository root; inspect the implementation branch when a file is absent from the current checkout.

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
