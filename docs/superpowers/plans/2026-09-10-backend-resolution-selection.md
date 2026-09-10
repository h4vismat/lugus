# Company resolution and observation selection implementation plan

> Execute in the isolated backend worktree using the execution and test-driven-development skills. Independent adapter and selector work may be delegated; root owns integration and review.

**Goal:** Implement the approved company-resolution and observation-selection specifications, including ticker/cashtag entry, SEC resolution, durable catalog evidence, and run-scoped deterministic financial read models.
**Architecture:** Pure Rust domain and selection functions; repository ports isolate SQLite; external-process capabilities isolate source access. No UI or automatic SEC-to-Yahoo symbol mapping.
**Specifications:** `docs/superpowers/specs/2026-09-10-company-resolution-design.md` and `docs/superpowers/specs/2026-09-10-observation-selection-design.md`.

## Constraints

- Existing APIs and evidence remain available; additive schema migration from v2.
- SEC-first discovery, plugin capability `company_resolution:1`, protocol envelope v1.
- Preserve ticker punctuation. `$IBM` is explicit identifier search; bare `IBM` tries exact ticker before name search only after an exhausted no-match result.
- Never guess identity links, merge by name, select conflicting facts by database ID, or mix price revisions from separate runs.
- Tests first for each behavioral slice; fixtures for network access; optional live validation is separate.

## Task 1: Rust resolution contract and catalog

Files: new `lugus-financial/src/resolution/{mod.rs,catalog.rs}`, `storage/catalog-v3.sql`; extend `capabilities.rs`, `plugin/mod.rs`, `storage/mod.rs`, `lib.rs`; new resolution tests and example.

Wire contract:
- search params `{query:{kind:"name",text:"..."}|{kind:"identifier",identifier:{namespace,value},exchange:null|{namespace,value}},page_size:1..100,cursor:null|string}`.
- lookup params `{identifier:{namespace,value}}`.
- candidate `{identifier:{namespace,value},name,aliases:[],listings:[{ticker:{namespace,value},exchange:null|{namespace,value}}],source_url,source_checksum,retrieved_at,match_reasons:[]}`.
- page `{items:[],next_cursor:null|string,snapshot:string,coverage:string}`.
- match reasons `exact_identifier`, `exact_name`, `name_substring`; lookup returns empty reasons.

- [x] Write red tests for cashtags, bare ticker fallback, invalid identifiers, ambiguous/partial search, and stable offline catalog IDs.
- [x] Implement typed contracts, bounded validation, provider trait and plugin methods.
- [x] Implement append-only catalog import/run status, exact external identity links, explicit conflicts, local search, and retrieval history.
- [x] Add a runnable search/lookup/offline example and protocol documentation.
- [x] Verify fixtures, process adapter, catalog reopen/migration, and error states.

## Task 2: SEC resolution implementation

Files: new `plugins/sec-edgar/resolution.py`, new Python resolution tests; extend provider dispatch, plugin identity/manifests and documentation.
Consumes the literal Task 1 wire shape. Implement search with stable process-local snapshot pagination and CIK lookup against submissions. Use injected transport and existing error conventions. Parse directory fields by name, preserve listing groups, reject malformed payloads, unsupported namespaces and inconsistent lookup identity.

- [x] Write failing source/dispatch tests using injected HTTP fixtures.
- [x] Implement source parsing, bounded search/lookup, and existing lifecycle integration.
- [x] Verify all Python tests, including existing filings/fundamentals behavior.

## Task 3: Run-scoped reads and pure observation selectors

Files: new `src/selection.rs`, `src/storage/selection.rs`, additive `storage/selection-v4.sql` if needed; new Rust selection tests and example.
Consumes existing evidence types. Produce an explicit repository read port for run-specific evidence, persistent repository identity and monotonic ingestion chronology. Root integrates migration/export edits.

- [x] Write failing tests for coherent run choice, source scope containment, empty/partial refresh, and immutable selections.
- [x] Implement run-scoped repository reads and pure market/fundamentals policies from the approved spec.
- [x] Verify exact decimal equivalence, same-date conflicts, distinct periods, latest-instant conflicts, permutation invariance, restart and migration.
- [x] Add explanatory CLI/example output and document public API.

## Task 4: Integration and review

- [x] Integrate catalog v3 then selection v4 migration and validate old v0/v1/v2 databases.
- [x] Run `cargo test --workspace --all-targets --offline`, strict Clippy, formatting, SEC and yfinance Python tests.
- [x] Review spec compliance and correctness, fix actionable findings and rerun affected checks.
- [x] Update specification statuses and README verification notes; commit the completed implementation on the feature branch.

## Execution ledger

- Worktree: `/private/tmp/lugus-backend-development`, branch `codex/company-resolution-observations`.
- Shared file ownership: root integrates SQLite migration dispatcher and public exports; adapter task owns Python files; selection task owns its new Rust modules and tests.

## Completion evidence

- Domain/process/catalog/application/selector tests implemented, including cashtags, offline choices, source errors, additive migration and exact evidence selection.
- Independent review found three gaps (explicit offline choices, typed lookup failures, frozen source coverage); all fixed with regression tests and scoped re-review passed.
- Full workspace tests, strict Clippy, formatting, SEC and yfinance Python suites passed. See README verification record for totals.
- CLI resolve/lookup/query/history/select/reopen exercised with a real synthetic provider process and temporary storage. No live SEC financial retrieval was performed.
- Source completion and source coverage remain separate: a completed run with partial source coverage is eligible and retains that explicit marker.
