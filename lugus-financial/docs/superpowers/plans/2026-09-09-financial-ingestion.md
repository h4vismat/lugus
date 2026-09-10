# Financial Ingestion Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development for the independent Python provider and review; implement the coupled Rust components together. Track steps here.

**Goal:** Ingest SEC filings and structured facts through a Python process and persist queryable history in SQLite.

**Architecture:** Rust owns capability interfaces, validation, normalization, process lifecycle, and storage. Python supplies SEC-specific decoding through versioned JSON-RPC. Pure transformations separate external effects.

**Tech Stack:** Rust 2024, Tokio, Serde, rusqlite, SHA-256, Python standard library.

**Spec:** ../specs/2026-09-09-financial-ingestion-design.md (approved).

## Global constraints

- Trusted separately installed plugins; no shell invocation or implicit installation.
- Exact decimal strings, namespaced identifiers, explicit periods, preserved disclosures.
- Explicit fetch versus offline query; no cross-provider fallback.
- Tests use fixtures; live SEC access is opt-in with user contact identity.
- Preserve the preexisting staged work. No automatic commits in this unborn repository.

## Execution decisions

- Work on `feat/financial-ingestion` in place: no HEAD exists from which to create a normal worktree.
- Cargo needs to update the parent workspace lockfile when adding approved dependencies; keep parent source and manifest unchanged.
- Restart failed pagination from page one; cursors are session-local. Idempotent storage makes this safe.

## Task 1: Domain and wire contract

Files: `src/domain/mod.rs`, `src/error.rs`, `src/capabilities.rs`, `src/lib.rs`, `docs/protocol/v1.md`, `tests/domain.rs`, `Cargo.toml`.

Interfaces: `CompanyId`, `Period`, `Fact`, `Filing`, `Page<T>`, `Query`, `Document`, `ProviderIdentity`; `Validate::validate() -> Result<()>`; pure `map_metric(&Fact) -> Option<Metric>` and `fingerprint<T: Serialize>(&T) -> Result<String>`.

- [x] Write tests rejecting imprecise numeric JSON, malformed decimals and inverted periods; assert namespaced mapping and stable fingerprints.
- [x] Run `cargo test -p lugus-financial --test domain` to observe missing APIs.
- [x] Implement immutable serde types and pure validation. Decimal strings must match `-?[0-9]+(\.[0-9]+)?`; all dates parse as dates.
- [x] Re-run the domain tests.

## Task 2: External process adapter

Files: `src/plugin/mod.rs`, `tests/plugin.rs`, `tests/fixtures/plugin.py`.

Interfaces: `Plugin::start(manifest, identity, configuration, limits).await`, capability methods accepting `Query`, `Plugin::close().await`. Independent traits expose `list_filings`, `fetch_facts`, and `fetch_document`.

- [x] Write process tests negotiating capabilities and exercising malformed responses, unsupported operations, timeouts and pagination.
- [x] Run the plugin tests and observe missing host behavior.
- [x] Implement bounded newline framing, request IDs, typed errors, initialization validation, serialized requests, drain stderr, timeout termination and reaping.
- [x] Re-run process tests using real Python child processes.

## Task 3: SQLite and application operations

Files: `src/storage/mod.rs`, `src/storage/schema.sql`, `src/application.rs`, `tests/ingestion.rs`.

Interfaces: `Repository` with run creation, atomic page persistence, run finalization, offline snapshots and document storage; `SqliteRepository::open`; `ingest` coordinates both capabilities.

- [x] Write SQLite integration tests for repeated refresh, changed disclosures, partial failures, transaction rollback and reopening.
- [x] Observe missing storage/application behavior.
- [x] Implement migration version checks, immutable observations keyed by stable SHA-256 excluding retrieval timestamp, run associations, status and cursor atomic writes, exact values in JSON/text, and content-addressed document BLOBs.
- [x] Re-run integration tests including the real fixture Python process.

## Task 4: Python SEC provider (delegated)

Files: `plugins/sec-edgar/` only. Consumes `docs/protocol/v1.md`; produces the four JSON-RPC methods documented there.

- [x] Write standard-library unittest fixtures for decoding periods, exact decimals, repeated filings, older submission files, unknown concepts and error mapping.
- [x] Run `python3 -m unittest discover -s plugins/sec-edgar/tests -v` and observe missing behavior.
- [x] Implement HTTP effects separately from pure decoders; SEC identity, throttling, retry limits, session-bound pagination and document size limits.
- [x] Re-run all provider tests, with network replaced only at the transport boundary.

## Task 5: Usable example, documentation and review

Files: `examples/sec_ingest.rs`, `README.md`, additional integration tests.

- [x] Provide explicit ingest/query/document modes and installation instructions, with configurable executable and SEC contact identity.
- [x] Run Rust formatting, full crate tests, Clippy and Python tests. Use an external temporary build target to respect the workspace sandbox.
- [x] Request independent review of protocol/domain/storage correctness. Fix material findings and rerun relevant tests.
- [x] Record final verification and limitations. Do not claim live SEC verification without an actual configured live run.

## Completion evidence

Implemented on `feat/financial-ingestion`, preserving existing staged changes. No commits or publication.

- 20 Rust integration tests pass: domain 4, process 5, storage 9, end-to-end 2.
- 19 Python fixture tests pass.
- Cargo formatting and all-target Clippy with warnings denied pass.
- Offline example query passes without SEC configuration.
- Independent review found two defects (reversed historical ranges and cancelled request reuse); both reproduced with failing tests, fixed and re-reviewed. No material findings remain.
- Live SEC retrieval not run; users must supply their own identifying contact User-Agent.
- Retrieval associations are exposed by `run_observations`; a cancelled ingestion stays visibly running, and restart starts a new idempotent run.

## Live verification follow-up

With the user-supplied SEC contact identity, ingested CIK 0000320193 for filing dates 2024-01-01 through 2024-12-31 into ignored local lugus.sqlite. The first run exposed legitimate primary-document subdirectories rejected by the parser. Added a regression, allowed safe relative paths, and retained traversal rejection. All 20 Python tests and both Rust end-to-end tests pass. Retry run 2 completed; offline query returned 100 filings and 1126 facts. Failed run 1 remains visible as designed. Contact identity was supplied only to the process environment, not committed into project defaults.
