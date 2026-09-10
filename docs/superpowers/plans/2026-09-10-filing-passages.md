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

- [x] Write RED realistic HTML test using literal expected text, e.g. `<h1>Risk factors</h1><p>Revenue &amp; cash <b>grew</b>.</p>` yields `Risk factors\nRevenue & cash grew.`; include Inline XBRL hidden fact exclusion and visible number preservation. A quote spanning inline nodes must map to each source node with correct decoded offsets.
```rust
assert_eq!(extracted.text, "Risk factors\nRevenue & cash grew.");
assert_eq!(page.text, "Revenue & cash grew.");
assert!(validate_selection(&text, 1, 2, "wrong").is_err());
```
- [x] Run `cargo test -p lugus-app --test passages_html --offline` and record expected missing-feature RED; then implement typed contracts and adapter.
- [x] Define strict serde DTOs and finite validated TextLimits. Defaults: input 32 MiB (also capped by existing document limit), text 16 MiB, nodes 200,000, depth 256, mappings 500,000, total source/mapping serialized bytes 64 MiB, page 64 KiB, passage 8 KiB; all per-call page/result envelopes are additionally capped by host/conversation bounds. Validate checked arithmetic, strict unknown fields, nonempty quote, Unicode boundaries and exact caller expected text.
- [x] Use maintained html5ever parsing, UTF-8 (BOM/declared/default) and explicitly declared Windows-1252 through encoding_rs with strict decoding; unsupported declarations fail. Decoder selection is recorded. Media type must be HTML; raw PDF/XML/plain text unsupported for extraction. Version all semantics as `html-text-v1` plus exact adapter versions.
- [x] Implement bounded parsing/tree construction (not an unbounded DOM followed by a size check), cancellation checks during feed/tree traversal/emission and depth/node accounting. Honor script/style/head/template/ix:hidden exclusion, hidden attribute, inline display:none and visibility:hidden; preserve visible ix text. Ignore external/computed CSS with limitation metadata. HTML parser errors may be recovered deterministically; no raw diagnostics in errors.
- [x] Normalize whitespace deterministically, use newline paragraph/heading/row and tab cell separators, retain source ranges for collapsed whitespace, mark inserted separators synthetic. Node paths refer to deterministic parsed tree; persist source text needed for mapping; coalesce adjacent compatible mappings without losing source bounds. Do not mutate reported numeric strings.
- [x] Pure range/source resolver returns clipped canonical mappings with source intervals; exact mappings may clip; normalized mappings retain contributing intervals. Reject synthetic-only selection. Test entities, non-ASCII, cross-node spans, duplicate quotes, malformed nesting, BOM/encoding disagreement policy, hidden metadata, tiny limits, cancellation and oversized/deep fixtures.
- [x] Run focused tests, strict Clippy/fmt. Commit `feat(app): extract versioned HTML filing text`; write report with RED/GREEN evidence, exported interfaces and material limitations.

## Task 2: Immutable scoped representation and passage storage

**Files:** Create `lugus-app/src/store/passages.rs`, focused `store/passages/` schema/write/read helpers; extend `store.rs`, `store/sqlite.rs`; `tests/passages_store.rs`.
**Consumes:** Task 1 final contracts and pure functions, EvidenceRepository::document, current dataset scoped reads and existing app-record/deduplication patterns.
**Produces:** ApplicationStore passage storage extension port, load preparation input / lookup existing / save completed representation methods, scoped text/header/passage/source reads and deduplicated create passage. Report exact signatures for Task 3.

- [x] Write real SQLite RED tests creating stored financial document observations and scoped document datasets. Repeated prepare/save must return the same representation; wrong workspace/repository denied; new extractor identity or document dataset creates a distinct representation.
```rust
assert_eq!(first.id, retry.id);
assert_ne!(first.id, revised.id);
assert_eq!(store.read_passage(&scope, &old.id)?.quote, "Revenue & cash grew.");
```
- [x] Run `cargo test -p lugus-app --test passages_store --offline`; implement additive schema v4 in one migration transaction, preserving prior app_records, bindings and conversation metadata. Do not change financial/review schema. Add optional unsupported default port methods to avoid breaking unrelated mock stores.
- [x] Load original input only through authorized document dataset and bounded EvidenceRepository read/checksum verification. Preparation may return cached header. Persist completed validated extraction in short transaction with unique scoped dataset+extractor key; no parser runs in store. Revalidate trusted dataset association on commit. Caller-supplied text/provenance cannot bypass trust through agent APIs.
- [x] Persist text/source/mappings in bounded chunks/rows with indexed offsets, preflight SQL lengths/counts before decoding and full envelope checks. Small page/source reads load only overlapping bounded rows. All IDs bound to workspace and repository. Header retains immutable observation, extractor/decoding, text checksum/length and limitations.
- [x] Passage creation validates expected quote and range against saved representation, reads intersecting source mappings, derives immutable provenance and checks stored/result bounds before transaction. Same workspace/request+input returns original; conflicting input fails. A quote cannot consist only of synthetic separators.
- [x] Resolve stored passage to bounded source-node excerpts from its own representation. Preserve originals after newer bytes at same URL/new extractor/tab closure/offline reopen. Detect corrupt lengths/mappings/checksums in accessed records safely. No full-filing allocation for small reads.
- [x] Cover concurrent connections racing deduplication, invalid range/quote and scope with no inserted rows, limits before allocation, changed-source restart, old schema migration and existing selected-reference restoration. Run focused storage and existing reference/conversation migration tests plus strict Clippy/fmt; commit `feat(app): persist immutable filing passages` and report interfaces.

## Task 3: Bounded host operations, agent tools and frozen passage context

**Files:** Create `lugus-app/src/application/passages.rs`, `agent/passages.rs`; extend `application.rs`, `agent.rs`, `conversations/domain.rs`, `conversations/frozen.rs`, `store/conversations/selection.rs`; tests `passages_application.rs`, `passages_conversations.rs`.
**Consumes:** Task 1 TextExtractor/contracts and Task 2 storage extension. Existing Application::store, ResearchExecutor, ConversationHost and frozen context builders.
**Produces:** Shared `prepare_text`, `read_text`, `text_header`, `create_passage`, `read_passage`, `resolve_passage` async Application APIs; six corresponding strict `lugus_*` tools; SelectedReference::Passage and restored FrozenReference validation. Report CLI-ready signatures.

- [x] Write RED host test with real store: prepare/read/select/source operations through manual and actual ResearchExecutor; strict unknown fields and forged scope rejected. Assert selected passage content appears as untrusted structured data in actual AgentRuntime RunRequest.
```rust
let selected = SelectedReference::Passage { id: passage.id.clone() };
assert!(captured.context.is_empty());
assert!(captured_input.contains("Revenue & cash grew."));
```
- [x] Implement preparation as bounded admission, load bounded owned input under store lock, extract in spawn_blocking outside that lock, then atomically save through storage. Hold extraction permits through cancellation cleanup. Cancellation checks plus bounded input prevent runaway work. Shutdown fences preparation and drains admitted workers; dropped callers cannot leave unbounded jobs or block store reads. Integrate lifecycle with existing shutdown ownership without changing provider semantics. Make extractor replaceable via compatible construction/configuration port; default local HTML adapter.
- [x] Shared methods validate IDs/input/output and use host limits. Cached preparation requires no provider; reads never extract. Add strict schemas from existing agent helpers and retain immutable turn offering, recorded intent/results, full ToolResult envelope limits and safe errors.
- [x] Add Passage variant to selected reference id/validation and typed frozen construction/restoration. Admission loads owned immutable passage using selected budget; quote/provenance/source mappings retained in serialized untrusted reference payload. Validate checksum, typed shape and identity on restoration. Existing dataset/view/binding snapshots remain unchanged. Required references over budget reject before admission/runtime creation.
- [x] Test malformed nested tool input, source text injection staying out of trusted instructions, same request returning old frozen context after revision, old snapshots readable, Unicode/range/output bounds, extraction cancellation/drop/shutdown, concurrent small reads while extraction is held, preparation failure without partial record, scope mismatch and no network offline.
- [x] Run focused host/context/agent/lifecycle regressions, strict Clippy/fmt; commit `feat(app): expose filing passages to research conversations`; report exact signatures and test evidence.

## Task 4: CLI acceptance and documentation

**Files:** Extend `lugus-app/examples/research.rs` through focused `examples/support/passages.rs`; `examples/support/conversations.rs` and fixture modules as required; `examples/synthetic/setup.py`, synthetic provider; `tests/passages_cli.rs`; README, approved spec status and roadmap milestone 4.
**Consumes:** Task 3 final shared API/tools/context.
**Produces:** Reproducible public CLI prepare-text/text-header/read-text/create-passage/read-passage/resolve-passage and a deterministic actual conversation runtime fixture; process acceptance exercising all milestone exit conditions.

- [x] Write RED CLI process acceptance using a real HTML/Inline XBRL fixture through the synthetic provider protocol and saved document dataset, never manually seeded passage records. Invoke shared commands and create a real conversation with selected passage.
```rust
assert_eq!(resolved.observation.checksum, original_checksum);
assert_eq!(reopened_passage.quote, "Revenue & cash grew.");
assert_ne!(new_header.id, old_header.id);
```
- [x] CLI inputs use strict external JSON and bounded argument/range parsing; all local reads/prepare/select operate offline. Runtime fixture is explicit deterministic AgentRuntime, with observed selected passage input and source resolution through actual tools. It must not claim a live model answer or rendered UI.
- [x] Full process test: prepare HTML, select cross-node passage, ask question and resolve source, ingest revised bytes at same URL, create new representation, shutdown/remove provider manifest/reopen, prove old passage and frozen input unchanged and new passage different. Include closed-tab preservation where existing CLI permits. Malformed/oversized/cross-conversation input must not write passage or admit turn.
- [x] Document exact CLI commands, output shapes and returned IDs, encoding/structural extraction limits, HTML-only scope, offline/source semantics and distinction from review evidence. Update roadmap milestone 4 complete only after root final checks; change HTML/PDF wording to explicit HTML scope and deferred PDF/OCR. Preserve main pending docs: root synchronizes only the roadmap after inspecting its diff.
- [x] Run focused CLI and relevant legacy CLI tests, strict Clippy/fmt; commit `feat(app): verify filing passage research across restarts`; report a standalone fresh-directory acceptance recipe and result assertions.

## Final verification (controller)

- [x] Run workspace/all-target tests offline locked, strict workspace/all-target/all-feature Clippy, formatting and git diff checks; SEC/yfinance adapter suites as appropriate.
- [x] Execute independent fresh standalone CLI flow from Task 4 with returned IDs and changed same-URL document; record original/new identity and offline frozen-context evidence.
- [x] Independent task reviews and whole-milestone review against `42ff3ae`, resolve confirmed findings with regression evidence and scoped re-review.
- [x] Record final counts, branch commits, remaining format limits and decisions in plan; mark roadmap milestone complete accurately; keep work local and unmerged.

## Completion record — 2026-09-10

Completed locally on `codex/filing-passages`, based on `42ff3ae`, with implementation/tests through `d9df02c`. The existing worktree at `/private/tmp/lugus-backend-development` is preserved; nothing was merged, pushed or published. Main's pending documents were preserved, with only the roadmap progress and this feature's approved design status synchronized.

### Implementation and independent review

- `9ad060b`: approved design and executable plan.
- `2d580b2`: pure HTML extraction/domain contracts. Independent review approved; its empty-source-array envelope edge case was fixed and regression-tested in the storage task.
- `c1b9fc7`, `13119b8`: immutable storage and complete mapping inventory validation. Review findings about deleted normalized contributors and storage error classification were fixed and re-reviewed clean.
- `2fdc300`, `6bae80c`: shared host/tools and frozen passage context. Review found that opaque source-node IDs were incorrectly treated as count-bounded; sparse high-ID regression coverage and the fix were re-reviewed clean.
- `85daca8`, `d9df02c`: CLI acceptance and documentation. Review requested stronger literal source assertions and full offline record equality; seven hand-derived mappings/excerpts, normalized whitespace and complete runtime/offline record comparisons now cover them. Scoped re-review is clean.
- Final independent whole-milestone review inspected `42ff3ae..d9df02c`, including unchanged conversation admission and tool/context boundaries: no Critical, Important or Minor findings. No findings remain parked or unresolved.

### Final verification

On `d9df02c`, `cargo test --workspace --all-targets --offline --locked` passed **400 tests in 56 suites**, with zero failures or ignored tests. `cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings`, `cargo fmt --all --check` and `git diff --check` passed. Logs: `/private/tmp/lugus-passages-verified-tests.log` and `/private/tmp/lugus-passages-verified-clippy.log`.

Adapter verification passed **31 SEC tests** and **23 yfinance tests** using the existing pinned SDK environment, without skips; adapter source hashes remained unchanged throughout this milestone. Logs: `/private/tmp/lugus-passages-sec.log` and `/private/tmp/lugus-passages-yfinance-pinned.log`.

The controller independently executed `/private/tmp/lugus-passages-standalone.py` in a fresh directory using the real CLI, synthetic provider process and actual AgentRuntime interface. It verified original raw fixture-byte checksum agreement, seven literal mapping/source entries (including two normalized whitespace ranges), full recorded tool-result equality, distinct revised representations/passages, stale-quote refusal, preservation after tab closure, complete passage/source/frozen-context equality after manifest removal and restart, idempotent retry without replay, and a successful fresh offline follow-up.

- Original document SHA-256: `440a9a9cb6170871ba0d2075271d7b638297b927546113b8e24028cf42757a57`.
- Revised same-URL SHA-256: `7cfb8d0f165b85ffff43127d928f10e262c548120c2215c3befccfaeae3673aa`.
- Original representation: `101240a9e3f7e206863610d772a339991004a7d5bc373e59f3a405955cbac4bd:0`.
- Revised representation: `b9f8dd63a0a273aeb2f2d07561267327309cd20a2bc3cfb606d41d373ff3a912:0`.
- Original passage: `9c3b69c5dadda9f2aa320698165b5405b01e82da9835876a2059b3648a6aac20:0`.
- Revised passage: `6abbe81e5a89b6e1052689ce4dfce036e253a9b8d5d9d7270f0656ceaa29c608:0`.

Standalone log: `/private/tmp/lugus-passages-standalone-final.log`. Full returned records and summary remain in `/private/var/folders/x4/t0lq0qzj5vn7x079vw_jr9_h0000gn/T/lugus-passages-accept-_7_z9nhs`. These local logs are supplementary; committed CLI tests and the README recipe reproduce the acceptance contract without them.

### Final decisions and limits

HTML/Inline XBRL only; PDF/OCR, XML/SGML, desktop rendering/highlighting and durable review passage capture remain deferred. Strict UTF-8 and explicitly declared Windows-1252 are supported. Coordinates address decoded parsed text nodes; structural visibility does not evaluate computed CSS. Pure domain code and replaceable local adapters avoid service/vendor dependence.

The bounded html5ever TreeSink uses a private typed unwind sentinel because parser callbacks are infallible; only that sentinel is caught and foreign panics propagate. Recoverable parser cancellation/limits require normal unwind builds, as used by this workspace. Injected trusted extractors must remain bounded and cooperative. The mapping inventory check scans at most 500,001 scalar covering-index entries rather than claiming constant-time metadata work; source payload reads stay limited to overlaps. Cancellation before publication admission prevents saving; an already admitted atomic save may finish, with shutdown draining workers asynchronously. No live-provider/model or browser-rendering acceptance is claimed.

All milestone exit conditions are met. The next milestone is active-workspace background refresh; integration remains a separate authorized action.
