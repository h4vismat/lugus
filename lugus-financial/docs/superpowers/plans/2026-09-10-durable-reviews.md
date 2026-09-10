# Durable Investment Reviews Implementation Plan

> **For agentic workers:** Use superpowers:executing-plans to implement this plan task-by-task in the feature worktree. Steps use checkbox syntax for tracking.

**Goal:** Persist explicit thesis reviews and recover them across fresh runtime sessions.

**Architecture:** Pure review records and validation, transactional ReviewStore port, SQLite implementation, public financial evidence adapter, and a coordinator using AgentRuntime/ToolExecutor. Durable assessment completion is independent of runtime completion.

**Tech Stack:** Rust 2024, existing workspace serde/serde_json, Tokio, rusqlite, chrono, sha2 and thiserror; tempfile integration tests.

**Spec:** ../specs/2026-09-10-durable-reviews-design.md (refines 2026-09-09-agent-harness-design.md).

## Global constraints
- Explicit reviews of already-ingested financial evidence only.
- Pure domain types contain no runtime protocol or provider SDK types.
- Separate agent database, optimistic revision checks, immutable evidence and history.
- Thesis 16 KiB; assessment JSON 32 KiB; 64 evidence items each at most 64 KiB; context 64 KiB.
- One active review per thesis; commit assessment and review completion atomically.
- Existing untracked user files remain untouched.

## Task 1: Durable records and storage
**Files:** `lugus-agent/src/reviews/{mod.rs,domain.rs,store.rs,sqlite.rs,schema.sql}`, `lugus-agent/tests/review_store.rs`; crate lib and dependencies.
**Interfaces:** domain `ThesisRevision`, `Evidence`, `Review`, `AssessmentDraft`, `Assessment`, `ReviewStatus`, `Attempt`; `ReviewStore: Send + Sync` with save_thesis, thesis, enqueue, review, claim, submit, finish, recover, assessments; `SqliteReviewStore::open(path)`.
- [x] Write real SQLite tests: persist thesis/evidence and submitted assessment, reopen and read exact history; conflicting review ID and thesis update; unknown evidence; repeated submission; stale thesis; one running review; stale attempt after recovery; completion survives finish/recover.
- [x] Run `cargo test -p lugus-agent --test review_store --offline`; confirm missing review API failure.
- [x] Implement records/validation and SQL transactions. Use `BEGIN IMMEDIATE`, conditional revision/status checks, unique running-thesis index, foreign keys, schema version 1. Store canonical JSON alongside indexed IDs/status; compare idempotent payloads before changing state.
- [x] Re-run storage tests, then format and inspect transaction boundaries.

## Task 2: Frozen financial evidence
**Files:** `lugus-agent/src/reviews/financial.rs`, `lugus-agent/tests/review_financial.rs`.
**Interface:** `capture_financial_evidence(repository: &dyn lugus_financial::storage::Repository, provider: &ProviderIdentity, query: &Query) -> ReviewResult<Vec<Evidence>>`.
- [x] Write a test ingesting financial facts A then B into a temporary real financial store; capture each and preserve the first queued selection; assert old evidence values remain available, source identity and ingestion completeness metadata retained. Query failures and oversized selections fail explicitly.
- [x] Run `cargo test -p lugus-agent --test review_financial --offline` before implementation.
- [x] Build evidence using public snapshot types and fingerprinted provenance/content; retain provider, source URLs, observation IDs and snapshot run status. No SQL access to financial internals.
- [x] Re-run test.

## Task 3: Coordinator and scoped tools
**Files:** `lugus-agent/src/reviews/{coordinator.rs,tools.rs}`, `lugus-agent/tests/review_workflow.rs`; `src/runtime.rs`, `src/codex/session.rs` and existing request fixtures/tests.
**Interfaces:** `ReviewCoordinator::new(store, clock).execute(runtime, ReviewExecution { review_id, runtime_identity, limits }, events, cancel) -> ReviewResult<Review>`; `Clock::now() -> DateTime<Utc>` and SystemClock. RunRequest gains `allow_web_search: bool`; existing generic examples retain true; durable reviews use false.
- [x] Write deterministic AgentRuntime scenarios driving real tools: successful submission, completed without submission, cancellation before/after submission, runtime failure after submission, stale edit, unknown/out-of-scope tool requests, failed retry in a new runtime, and fresh-session comparison using previous assessment.
- [x] Run workflow tests before implementing.
- [x] Claim and freeze attempt, assemble bounded data input, expose read_evidence and submit_assessment with strict schemas, drive runtime and classify persisted outcome. Recheck attempt on every tool; SQL handles final race checks. Trusted instructions and untrusted data use separate channels. Close runtime after each execution; preserve a committed assessment regardless of later runtime outcome.
- [x] Verify Codex request disables native search when false using protocol fixture; existing requests keep their prior behavior.
- [x] Run workflow/protocol tests and inspect error/cancellation ordering.

## Task 4: Runnable workflow and final verification
**Files:** `lugus-agent/examples/durable_review.rs`, `lugus-agent/README.md`.
- [x] Provide CLI commands create/edit/show/queue/run/recover. Queue accepts financial provider/query JSON and captures existing evidence; run uses an explicitly selected empty workspace and existing Codex auth. Show renders saved history without a model.
- [x] Exercise CLI create/edit/show/recover against temporary files; live execution remains explicit.
- [x] Run `cargo fmt --all -- --check`, `cargo test --workspace --all-targets --offline`, `cargo clippy --workspace --all-targets --offline -- -D warnings`; inspect final diff for trust, transaction and retry issues.
- [x] Record deterministic results and live limitations in README and plan; commit feature branch with reviewed files only.


## Final implementation and verification

The application remains a module inside lugus-agent, with a public ReviewStore port and short SQLite transactions. Financial capture enumerates snapshots before caller selection; enqueue enforces the 64-item bound. CLI selection retains ingestion-scope metadata. A direct assessment lookup avoids loading full history into every runtime request. RunRequest gains explicit allow_web_search with a false deserialization default; generic examples opt in and durable reviews disable it.

Validation: 116 workspace/all-target tests passed; strict workspace Clippy, formatting and whitespace checks passed. The new tests include actual host-channel cancellation, runtime cancellation acknowledgment, dropped-future recovery, concurrent submissions, transaction rollback, changed-evidence review across reopening, and a reproduced/fixed oversized tool-error response. Independent review found no remaining important issues.

Live synthetic verification: fresh Codex sessions persisted an initial assessment of 100 USD assets, then a changed assessment against 80 USD with the previous assessment available. Both evidence versions and assessments survived separate CLI processes. A third review cancelled after 500 ms remained interrupted with no additional assessment. CLI create/queue/selection/edit/show/status/recover worked against temporary files. No real financial/user data or credentials were copied. Exact resolved model/provider identity is not exposed by AgentRuntime; the CLI records its runtime version and explicitly labels configured defaults. Shared knowledge, automatic triggers, document/web capture, market-price adaptation and UI remain later milestones.
