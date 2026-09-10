# Application Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Deliver scoped manual/agent research operations with independently managed providers, frozen references, and accepted views.

**Architecture:** A new `lugus-app` Rust crate separates pure command/catalog decisions from process and storage effects. Each provider has a dedicated worker and repository handle; financial ingestion/selection remain in `lugus-financial`.

**Tech Stack:** Rust 2024; existing Tokio, serde, async-trait, rusqlite, chrono, sha2 dependencies; Python fixture processes. No vendor SDK or desktop dependency.

**Spec:** `docs/superpowers/specs/2026-09-10-application-runtime-design.md`

## Global Constraints

- Implement on top of `eea83bf`; no merge, push, or publication.
- Explicit provider-native market identifiers; no SEC-to-Yahoo inference.
- Different instances progress independently; no SQLite transaction across provider awaits.
- Explicit provider selection; no default, first-instance choice, or failover.
- Host scope cannot be replaced by model arguments.
- Old dataset IDs retain frozen evidence; decimal values remain strings.
- View acceptance is distinct from rendering.
- Cancel provider calls, not entire ingestion futures; finalize runs and reap children.
- No background scheduler, conversation persistence, extraction, or desktop integration.
- Agent acceptance uses a real stored thesis; general conversation identity stays in milestone 3.
- Validate finite queue/job/deadline/page/item/byte/input/output/document/read bounds.

## Shared vocabulary and file map

Task 1 owns crate setup, `src/domain.rs`, `src/error.rs`, `src/catalog.rs`.
Task 2 owns `src/provider.rs`, `src/worker.rs`, `tests/worker.rs` and process fixtures.
Task 3 owns `src/store.rs`, `src/references.rs`, `tests/references.rs`, and narrow bounded financial evidence queries.
Task 4 owns `src/application.rs`, `src/agent.rs`, CLI, acceptance tests, README.
Each task declares its modules in lib.rs sequentially and supplies a report documenting concrete exported signatures for its successor.

```rust
pub type Result<T> = std::result::Result<T, AppError>;
pub struct Scope {
    pub workspace_id: String,
    pub request_id: String,
    pub run_id: Option<String>,
}
// AppError: kind, bounded message, retryable, optional retry_after_seconds.
// ErrorKind: InvalidInput, Unsupported, AmbiguousProvider, Unavailable,
// Deactivated, MissingData, ScopeMismatch, StaleReference, Conflict,
// ResourceLimit, RateLimited, Cancelled, Storage, Timeout.
// Operation: Resolve, Lookup, Filings, Facts, Document, Prices.
// FetchCommand: strict tagged enum, explicit instance_id on every variant;
// Resolve(input), Lookup(LookupRequest), Filings/Facts(Query),
// Document(source_url), Prices(PriceQuery).
// DatasetKind: Prices, Facts, Filings, Resolution, Document.
// ViewKind: PriceChart, DataTable, Document.
// JobState: Queued, Running, Succeeded, Failed, Cancelled.
```

### Task 1: Pure contracts and capability catalog

**Files:** Create `lugus-app/Cargo.toml`, `src/{lib,domain,error,catalog}.rs`, `tests/catalog.rs`; modify root Cargo.toml/Cargo.lock.
**Consumes:** Existing financial Query, PriceQuery, LookupRequest, ProviderIdentity and validators.
**Produces:** Shared vocabulary, Limits validation, Catalog/Offering with generation-bound authorization. Record exact API in task report.

- [x] Write failing behavioral tests for duplicate configured IDs, unsupported versions, explicit provider requirement, stale generation, activation during a turn, deactivation after offering, empty/oversized scope, root cursor rejection and unknown JSON fields.

```rust
let offered = catalog.snapshot();
catalog.deactivate("market-a")?;
assert_eq!(catalog.authorize(&offered, &prices_a).unwrap_err().kind,
           ErrorKind::Deactivated);
```

- [x] Run `cargo test -p lugus-app --test catalog --offline` and observe missing behavior.
- [x] Implement map-based catalog and pure validation. Preserve full identity, activation, availability, versions, revision and generation. Snapshot only supported active/available operations; dispatch checks offering and live state. Map Resolve/Lookup to company_resolution v1, Filings/Document to filings v1, Facts to fundamentals v1, Prices to market_data v1; verify these strings against Plugin first.
- [x] Run test/Clippy and commit `feat(app): define scoped commands and capability catalog`.

### Task 2: Bounded provider workers and cancellation

**Files:** Create provider/worker modules, real Python fixture and `tests/worker.rs`.
**Consumes:** Task 1 contracts; Plugin and financial ingestion/resolution functions.
**Produces:** Explicit lifecycle port/adapter, bounded per-instance worker handle, cancellation and shutdown, exact fetch result provenance. Record public API in report.

- [x] Write failing tests with real fixture processes and a response barrier. Cancellation during a response must finalize the started run, retain committed pages, invalidate/reap the child, and return Cancelled. Test another instance completes while one is blocked, timeout, queue/page/item/byte bounds, queued cancellation, startup failure, source error and shutdown.
- [x] Run `cargo test -p lugus-app --test worker --offline` to observe failures.
- [x] Wrap every provider call with cancellation/deadline/total budgets; do not drop ingestion. Track one budget across resolution fallback and pages. Check size/count before handing a page to ingestion. Existing ingestion finalizes on wrapper errors.

```rust
let result = tokio::select! {
    biased;
    _ = cancelled(&mut cancel) => Err(cancel_error()),
    _ = tokio::time::sleep_until(deadline) => Err(timeout_error()),
    result = plugin_call => result,
};
// After ingestion returns, close/reap interrupted plugin before terminal result.
```

- [x] Run each mutable plugin/repository on a dedicated thread with its own current-thread Tokio runtime. Synchronous repository methods cannot block caller async threads. Initialize schemas before concurrent startup, use bounded channels and short lock sections, recheck admission/generation after queueing. Cancellation between pages finalizes too. Storage/finalization failure outranks clean cancellation. Resolution may return Ok with a failed persisted outcome: map that to an application failure. Preserve source errors that do not invalidate a healthy process.
- [x] Run worker tests/Clippy and commit `feat(app): manage bounded cancellable provider workers`.

### Task 3: Frozen references and durable view receipts

**Files:** Create store/references modules, `tests/references.rs`; add bounded exact-run read APIs to financial storage where needed.
**Consumes:** Task 1 vocabulary, Task 2 exact provenance, select_daily/select_facts, CatalogRepository.
**Produces:** ApplicationStore port and SQLite adapter; bounded offline dataset/document reads and creation, candidate selection, view acceptance/presentation results.

- [x] Write failing real-store tests: freeze a dataset, ingest a new revision, reopen and read old exact values; wrong workspace/repository rejection; preserved decimal/conflict/coverage semantics; atomic duplicate view acceptance; changed-input conflict; incompatible kinds and renderer failure.

```rust
let first = store.open_view(&scope, &request)?;
let repeat = store.open_view(&scope, &request)?;
assert_eq!(first.id, repeat.id);
assert_eq!(store.read_dataset("other-workspace", &dataset.id).unwrap_err().kind,
           ErrorKind::ScopeMismatch);
```

- [x] Run `cargo test -p lugus-app --test references --offline` to observe failures.
- [x] Persist application IDs, workspace/repository/full-provider identity, exact run/retrieval/observation references, query, frozen policy/coverage and typed bounded payloads. Use app schema version checks. Read stored pages with allocation bounds: never load all provider history and truncate afterward. Add preflight counts/bytes and exact-run financial queries as needed; pin selection to authorized run evidence. Keep missing/conflicting values explicit.
- [x] Filings/resolution retain run membership. Candidate selection validates membership in the workspace reference before delegation. Document references verify checksum plus exact provider/source/retrieval association; bytes/ranges are bounded, no HTML/PDF extraction. Persist view receipt and canonical input transactionally under workspace/request ID; presentation reports include view ID and descriptor revision. Repeating identical input returns the receipt; different input conflicts.
- [x] Run reference tests/Clippy and commit `feat(app): persist scoped research references and view receipts`.

### Task 4: Shared host, tool executor, and CLI acceptance

**Files:** Create application/agent modules, examples/research.rs, tests/application.rs, tests/agent.rs, README and fixtures; update completed progress documentation.
**Consumes:** Task 1–3 exports; AgentRuntime/ToolExecutor and real thesis store.
**Produces:** Cloneable application host, host-bound executor, structured manual/agent CLI workflow.

- [x] Write failing tests for shared manual/agent effects, forged run/scope, strict arguments, unoffered tools/providers, offline tools with no providers, executor-drop cancellation, deactivation while queued, terminal races, saturated event sink, shutdown, bounded job retention.
- [x] Run `cargo test -p lugus-app --offline` to observe missing behavior.
- [x] Implement host submission/status/wait/cancel and offline/reference/view commands. Register bounded jobs with owning workspace, one authoritative terminal result and nonblocking event delivery. Reject submissions during shutdown. Deactivation closes admission before cancelling queued/running jobs. Restart increments generation. Host creates scope; per-call request IDs encode run/call identity without collisions.

```rust
validate_call_run(&bound_run, &call.run_id)?;
let command = decode_strict(&call.name, call.arguments)?;
let scope = host_scope.for_call(&call.call_id)?;
let receipt = application.submit(&scope, &offering, command).await?;
let result = application.wait(&scope.workspace_id, &receipt.id).await?;
```

- [x] Derive stable schemas from offerings, always include supported cached-data tools, reject unknown fields and tools at runtime. A cancellation guard signals the submitted job when a tool wait is dropped; cleanup stays owned by host. Bound serialized ToolResults including safe failures without truncating JSON.
- [x] Add CLI external configuration/manual commands and deterministic agent-fixture mode using an actual persisted thesis and AgentRuntime adapter. Print scope, provider, reference and accepted-view receipts. Demonstrate two instances, offline reopening, unavailable provider. Never claim rendered UI or general conversations.
- [x] Run synthetic CLI; workspace/all-target Rust tests, strict workspace Clippy, formatting, SEC and yfinance Python suites. No live model/provider call required.
- [x] Document exact commands, limits, guarantees and remaining milestones; commit `feat(app): unify manual and agent research workflows`.

## Final review

- [x] Review full branch against eea83bf for spec compliance and quality. Fix confirmed findings with regressions and scoped re-review.
- [x] Leave local branch ready for review and report actual verification; no merge or push.

## Completion record

Completed on local branch `codex/application-runtime` through `403aa1d`, based on
`eea83bf`. Final verification passed: 243 workspace/all-target Rust tests with
`--offline --locked`, strict workspace/all-target/all-feature Clippy, workspace
formatting, 31 SEC Python tests and 12 yfinance Python tests. Independent foundation
and integration review findings were fixed and re-reviewed.

A fresh documented CLI run used two real synthetic provider processes and a real
stored thesis through `AgentRuntime`, verified explicit routing and unavailable
startup, retained decimal-string prices, accepted an unpresented price-chart view,
and reopened fetch/page/view references after removing the provider manifest.
See `lugus-app/README.md` for reproducible commands and operational limits.

The branch and worktree are preserved locally. Nothing was merged or pushed.
