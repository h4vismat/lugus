# HTML Filing Passages Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development to implement this plan task-by-task. Steps use checkbox syntax for tracking.

**Goal:** Ask about an exact selected HTML filing passage, resolve its original source, and retain it unchanged across revisions and offline restart.

**Architecture:** Pure versioned local HTML extraction and passage validation in `lugus-app::passages`; bounded application-owned immutable storage around the existing financial document repository. Shared manual/agent APIs feed typed frozen passage context into existing conversations.

**Tech Stack:** Rust, serde, SHA-256, SQLite, Tokio; locally available html5ever 0.38 and encoding_rs 0.8 through replaceable adapters. Record exact parser/decoder versions in extraction identity.

**Spec:** `docs/superpowers/specs/2026-09-10-filing-passages-design.md` (approved 2026-09-10).

## Global Constraints

- Work in `/private/tmp/lugus-backend-development`, branch `codex/filing-passages`, based on `42ff3ae`. Preserve main's pending documents. No merge, push or publishing.
- HTML including Inline XBRL only; PDF/OCR/XML/SGML extraction, desktop rendering and durable review passage capture are deferred.
- Original financial bytes/observations remain unchanged. Application schema advances additively from v3 to v4.
- Exact immutable identities and scoped access; checksums are integrity metadata, never authorization.
- Half-open UTF-8 ranges; distinguish parsed text-node locations from original byte offsets and browser UTF-16.
- Pure domain functions, explicit replaceable adapters; no network during extraction or offline reads.
- Finite input, parse, output, mapping, concurrency and response bounds. No extraction under store mutex or SQL transaction.
- Test-first with observed RED and GREEN. Focused tests during iteration, broad final verification once integrated. Every implementer self-reviews; controller dispatches reviews. Workers never spawn subagents.
- Small focused files may be split within the named module directories as needed. Report exact final public interfaces for downstream tasks.

## Task 1: Pure extraction, mapping and passage contracts

**Files:** Create `lugus-app/src/passages.rs`, `passages/domain.rs`, `passages/html.rs`, `passages/mapping.rs`, optional focused helper modules in `passages/`; export in `lib.rs`; dependencies in `lugus-app/Cargo.toml` and lockfile; `lugus-app/tests/passages_html.rs`, `tests/fixtures/filing.html`.

**Consumes:** Existing AppError/ErrorKind/Scope, Limits, DocumentObservation, DatasetHeader and SHA-256.
**Produces:** Public typed TextLimits, ExtractorIdentity, ExtractedText, TextRepresentation (header), TextPage, SourceNode, SourceMapping, CreatePassageRequest, Passage, PassageSource; exact exported fields and signatures in report. Pure `extract_html(bytes, media_type, limits, cancellation)` through a replaceable `TextExtractor` interface; pure range/mapping/passage validation.

- [ ] Write RED realistic HTML test using literal expected text, e.g. `<h1>Risk factors</h1><p>Revenue &amp; cash <b>grew</b>.</p>` yields `Risk factors\nRevenue & cash grew.`; include Inline XBRL hidden fact exclusion and visible number preservation. A quote spanning inline nodes must map to each source node with correct decoded offsets.
```rust
assert_eq!(extracted.text, "Risk factors\nRevenue & cash grew.");
assert_eq!(page.text, "Revenue & cash grew.");
assert!(validate_selection(&text, 1, 2, "wrong").is_err());
```
- [ ] Run `cargo test -p lugus-app --test passages_html --offline` and record expected missing-feature RED; then implement typed contracts and adapter.
- [ ] Define strict serde DTOs and finite validated TextLimits. Defaults: input 32 MiB (also capped by existing document limit), text 16 MiB, nodes 200,000, depth 256, mappings 500,000, total source/mapping serialized bytes 64 MiB, page 64 KiB, passage 8 KiB; all per-call page/result envelopes are additionally capped by host/conversation bounds. Validate checked arithmetic, strict unknown fields, nonempty quote, Unicode boundaries and exact caller expected text.
- [ ] Use maintained html5ever parsing, UTF-8 (BOM/declared/default) and explicitly declared Windows-1252 through encoding_rs with strict decoding; unsupported declarations fail. Decoder selection is recorded. Media type must be HTML; raw PDF/XML/plain text unsupported for extraction. Version all semantics as `html-text-v1` plus exact adapter versions.
- [ ] Implement bounded parsing/tree construction (not an unbounded DOM followed by a size check), cancellation checks during feed/tree traversal/emission and depth/node accounting. Honor script/style/head/template/ix:hidden exclusion, hidden attribute, inline display:none and visibility:hidden; preserve visible ix text. Ignore external/computed CSS with limitation metadata. HTML parser errors may be recovered deterministically; no raw diagnostics in errors.
- [ ] Normalize whitespace deterministically, use newline paragraph/heading/row and tab cell separators, retain source ranges for collapsed whitespace, mark inserted separators synthetic. Node paths refer to deterministic parsed tree; persist source text needed for mapping; coalesce adjacent compatible mappings without losing source bounds. Do not mutate reported numeric strings.
- [ ] Pure range/source resolver returns clipped canonical mappings with source intervals; exact mappings may clip; normalized mappings retain contributing intervals. Reject synthetic-only selection. Test entities, non-ASCII, cross-node spans, duplicate quotes, malformed nesting, BOM/encoding disagreement policy, hidden metadata, tiny limits, cancellation and oversized/deep fixtures.
- [ ] Run focused tests, strict Clippy/fmt. Commit `feat(app): extract versioned HTML filing text`; write report with RED/GREEN evidence, exported interfaces and material limitations.

## Task 2: Immutable scoped representation and passage storage

**Files:** Create `lugus-app/src/store/passages.rs`, focused `store/passages/` schema/write/read helpers; extend `store.rs`, `store/sqlite.rs`; `tests/passages_store.rs`.
**Consumes:** Task 1 final contracts and pure functions, EvidenceRepository::document, current dataset scoped reads and existing app-record/deduplication patterns.
**Produces:** ApplicationStore passage storage extension port, load preparation input / lookup existing / save completed representation methods, scoped text/header/passage/source reads and deduplicated create passage. Report exact signatures for Task 3.

- [ ] Write real SQLite RED tests creating stored financial document observations and scoped document datasets. Repeated prepare/save must return the same representation; wrong workspace/repository denied; new extractor identity or document dataset creates a distinct representation.
```rust
assert_eq!(first.id, retry.id);
assert_ne!(first.id, revised.id);
assert_eq!(store.read_passage(&scope, &old.id)?.quote, "Revenue & cash grew.");
```
- [ ] Run `cargo test -p lugus-app --test passages_store --offline`; implement additive schema v4 in one migration transaction, preserving prior app_records, bindings and conversation metadata. Do not change financial/review schema. Add optional unsupported default port methods to avoid breaking unrelated mock stores.
- [ ] Load original input only through authorized document dataset and bounded EvidenceRepository read/checksum verification. Preparation may return cached header. Persist completed validated extraction in short transaction with unique scoped dataset+extractor key; no parser runs in store. Revalidate trusted dataset association on commit. Caller-supplied text/provenance cannot bypass trust through agent APIs.
- [ ] Persist text/source/mappings in bounded chunks/rows with indexed offsets, preflight SQL lengths/counts before decoding and full envelope checks. Small page/source reads load only overlapping bounded rows. All IDs bound to workspace and repository. Header retains immutable observation, extractor/decoding, text checksum/length and limitations.
- [ ] Passage creation validates expected quote and range against saved representation, reads intersecting source mappings, derives immutable provenance and checks stored/result bounds before transaction. Same workspace/request+input returns original; conflicting input fails. A quote cannot consist only of synthetic separators.
- [ ] Resolve stored passage to bounded source-node excerpts from its own representation. Preserve originals after newer bytes at same URL/new extractor/tab closure/offline reopen. Detect corrupt lengths/mappings/checksums in accessed records safely. No full-filing allocation for small reads.
- [ ] Cover concurrent connections racing deduplication, invalid range/quote and scope with no inserted rows, limits before allocation, changed-source restart, old schema migration and existing selected-reference restoration. Run focused storage and existing reference/conversation migration tests plus strict Clippy/fmt; commit `feat(app): persist immutable filing passages` and report interfaces.

## Task 3: Bounded host operations, agent tools and frozen passage context

**Files:** Create `lugus-app/src/application/passages.rs`, `agent/passages.rs`; extend `application.rs`, `agent.rs`, `conversations/domain.rs`, `conversations/frozen.rs`, `store/conversations/selection.rs`; tests `passages_application.rs`, `passages_conversations.rs`.
**Consumes:** Task 1 TextExtractor/contracts and Task 2 storage extension. Existing Application::store, ResearchExecutor, ConversationHost and frozen context builders.
**Produces:** Shared `prepare_text`, `read_text`, `text_header`, `create_passage`, `read_passage`, `resolve_passage` async Application APIs; six corresponding strict `lugus_*` tools; SelectedReference::Passage and restored FrozenReference validation. Report CLI-ready signatures.

- [ ] Write RED host test with real store: prepare/read/select/source operations through manual and actual ResearchExecutor; strict unknown fields and forged scope rejected. Assert selected passage content appears as untrusted structured data in actual AgentRuntime RunRequest.
```rust
let selected = SelectedReference::Passage { id: passage.id.clone() };
assert!(captured.context.is_empty());
assert!(captured_input.contains("Revenue & cash grew."));
```
- [ ] Implement preparation as bounded admission, load bounded owned input under store lock, extract in spawn_blocking outside that lock, then atomically save through storage. Hold extraction permits through cancellation cleanup. Cancellation checks plus bounded input prevent runaway work. Shutdown fences preparation and drains admitted workers; dropped callers cannot leave unbounded jobs or block store reads. Integrate lifecycle with existing shutdown ownership without changing provider semantics. Make extractor replaceable via compatible construction/configuration port; default local HTML adapter.
- [ ] Shared methods validate IDs/input/output and use host limits. Cached preparation requires no provider; reads never extract. Add strict schemas from existing agent helpers and retain immutable turn offering, recorded intent/results, full ToolResult envelope limits and safe errors.
- [ ] Add Passage variant to selected reference id/validation and typed frozen construction/restoration. Admission loads owned immutable passage using selected budget; quote/provenance/source mappings retained in serialized untrusted reference payload. Validate checksum, typed shape and identity on restoration. Existing dataset/view/binding snapshots remain unchanged. Required references over budget reject before admission/runtime creation.
- [ ] Test malformed nested tool input, source text injection staying out of trusted instructions, same request returning old frozen context after revision, old snapshots readable, Unicode/range/output bounds, extraction cancellation/drop/shutdown, concurrent small reads while extraction is held, preparation failure without partial record, scope mismatch and no network offline.
- [ ] Run focused host/context/agent/lifecycle regressions, strict Clippy/fmt; commit `feat(app): expose filing passages to research conversations`; report exact signatures and test evidence.

## Task 4: CLI acceptance and documentation

**Files:** Extend `lugus-app/examples/research.rs` through focused `examples/support/passages.rs`; `examples/support/conversations.rs` and fixture modules as required; `examples/synthetic/setup.py`, synthetic provider; `tests/passages_cli.rs`; README, approved spec status and roadmap milestone 4.
**Consumes:** Task 3 final shared API/tools/context.
**Produces:** Reproducible public CLI prepare-text/text-header/read-text/create-passage/read-passage/resolve-passage and a deterministic actual conversation runtime fixture; process acceptance exercising all milestone exit conditions.

- [ ] Write RED CLI process acceptance using a real HTML/Inline XBRL fixture through the synthetic provider protocol and saved document dataset, never manually seeded passage records. Invoke shared commands and create a real conversation with selected passage.
```rust
assert_eq!(resolved.observation.checksum, original_checksum);
assert_eq!(reopened_passage.quote, "Revenue & cash grew.");
assert_ne!(new_header.id, old_header.id);
```
- [ ] CLI inputs use strict external JSON and bounded argument/range parsing; all local reads/prepare/select operate offline. Runtime fixture is explicit deterministic AgentRuntime, with observed selected passage input and source resolution through actual tools. It must not claim a live model answer or rendered UI.
- [ ] Full process test: prepare HTML, select cross-node passage, ask question and resolve source, ingest revised bytes at same URL, create new representation, shutdown/remove provider manifest/reopen, prove old passage and frozen input unchanged and new passage different. Include closed-tab preservation where existing CLI permits. Malformed/oversized/cross-conversation input must not write passage or admit turn.
- [ ] Document exact CLI commands, output shapes and returned IDs, encoding/structural extraction limits, HTML-only scope, offline/source semantics and distinction from review evidence. Update roadmap milestone 4 complete only after root final checks; change HTML/PDF wording to explicit HTML scope and deferred PDF/OCR. Preserve main pending docs: root synchronizes only the roadmap after inspecting its diff.
- [ ] Run focused CLI and relevant legacy CLI tests, strict Clippy/fmt; commit `feat(app): verify filing passage research across restarts`; report a standalone fresh-directory acceptance recipe and result assertions.

## Final verification (controller)

- [ ] Run workspace/all-target tests offline locked, strict workspace/all-target/all-feature Clippy, formatting and git diff checks; SEC/yfinance adapter suites as appropriate.
- [ ] Execute independent fresh standalone CLI flow from Task 4 with returned IDs and changed same-URL document; record original/new identity and offline frozen-context evidence.
- [ ] Independent task reviews and whole-milestone review against `42ff3ae`, resolve confirmed findings with regression evidence and scoped re-review.
- [ ] Record final counts, branch commits, remaining format limits and decisions in plan; mark roadmap milestone complete accurately; keep work local and unmerged.
