# Durable Conversations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task by task. Steps use checkboxes for tracking.

**Status:** Complete. Implementation, final verification and independent reviews passed; branch and worktree remain local.

**Goal:** Persist research conversations and workspaces, execute thesis-free disposable turns with frozen bounded context, and recover without replaying completed work.
**Architecture:** Application-owned conversation records and a supervised coordinator wrap existing ResearchExecutor/AgentRuntime ports. Pure context/state functions are separate from SQLite, runtime and execution ownership effects. Preserve existing financial and review storage.
**Tech Stack:** Rust2024, existing Tokio/serde/rusqlite; standard File::try_lock for local execution ownership (stable1.89, installed rustc1.98); no new network service or model dependency.
**Spec:** docs/superpowers/specs/2026-09-10-durable-conversations-design.md

## Global Constraints

- Create a research conversation without creating an investment thesis.
- A conversation owns one workspace in this milestone; that workspace may cover several companies.
- Runtime providers do not own conversation storage. Each turn starts a fresh runtime session, with an immutable tool offering and application-derived workspace/run/call scope.
- Conversation text, prior assistant text, evidence and tool results are data, not developer instructions.
- Reopening never replays a completed model turn or repeats its tools.
- Completed results cannot be overwritten by late cancellation or stale callbacks.
- No database transaction spans a runtime/provider await, and synchronous storage work remains outside the async executor.
- No automatic resume/retry, refresh, passage extraction, summaries, vector search, deletion, branching, desktop UI or implicit thesis/assessment creation.
- Keep legacy manual application APIs and thesis-review workflows functional. No rewrite/adoption of old standalone workspaces.
- Worktree /private/tmp/lugus-backend-development, branch codex/durable-conversations, base45a5299 (design) atop e65942b. No merge/push/live credentials. Root owns roadmap/spec/plan status.

## Concrete limits and ownership policy

New ConversationLimits independent of existing Limits, serde defaults: message_bytes16384, assistant_bytes32768, context_bytes262144, context_messages64, selected_refs32, selected_bytes131072, activity_events512, activity_bytes1048576, tool_calls128, tool_record_bytes1048576, tool_total_bytes16777216, open_views64, page_items100, page_bytes1048576, active_runs4, event_capacity256, runtime_close_timeout_ms5000. Runtime close timeout must be1..=60000ms. Default turn RunLimits: timeout60s, max_tool_calls128, max_tool_result_bytes1048576; validate timeout <=24h and intersect tool/result limits with conversation/application budgets. Positive values only; byte limits <=1GiB, item/event limits <=100000, active_runs<=1024. Metadata/title <=1024 bytes, IDs use existing256-byte bound; control-free IDs. Bound full serialized envelopes/escaping, not just text content. Terminal status/message metadata has a separately bounded reserve so exhausting activity cannot prevent finalization. Deadline uses existing finite RunLimits; coordinator enforces it even against injected runtimes.

Local execution owner: canonicalize existing application DB path, open stable adjacent `<canonical-db>.conversation.lock` read/write/create without truncation, File::try_lock; WouldBlock -> Conflict. Keep file and lease alive until all runtime supervisors/tools/storage finalization finish; never unlink lock file. Opening ordinary store/read handles never recovers runs. Lease carries matching canonical store key; activation increments persistent execution epoch and atomically interrupts abandoned active runs. Store transition/call writes require matching opaque epoch/attempt token. Same-process and cross-process ownership tests; symlink path aliases canonicalize identically. Local SQLite files on supported local filesystems only; do not support hard-link aliases or network filesystem lock semantics. Primary API reference: https://doc.rust-lang.org/std/fs/struct.File.html#method.try_lock .

## Task 1: Generic run subject and pure conversation/context contracts

**Files:** Modify lugus-agent/src/runtime.rs, reviews/coordinator.rs and all Rust RunRequest construction/tests/examples (rg thesis_id restricted to RunRequest usage, never rename thesis persistence fields). Create lugus-app/src/conversations.rs, conversations/context.rs, conversations/domain.rs; tests/conversation_context.rs; update lib exports.
**Produces:** RunSubject::{Conversation{id:String},Thesis{id:String}} and RunRequest.subject; ConversationLimits; typed conversation/message/run/workspace/context records and pure builder. Task2/3 consume exact exports in report.
- [x] Write RED runtime tests for Conversation without thesis, explicit Thesis, empty subject rejection, strict unknown/conflicting subject fields. Preserve legacy serialized request with thesis_id by strict compatibility decoder; reject both subject and thesis_id, even if equal. New serialization emits subject only.
```rust
let subject = RunSubject::Conversation { id: "conversation-1".into() };
assert_eq!(serde_json::to_value(subject).unwrap(), serde_json::json!({"kind":"conversation","id":"conversation-1"}));
```
- [x] Run affected runtime test to observe RED; implement subject and update review coordinator/examples/fixtures to explicit Thesis. Preserve review schema/thesis identifiers and Codex request policy. Unknown fields in new subject rejected; legacy RunRequest compatibility explicit, not fake IDs.
- [x] Define immutable Conversation{id,workspace_id,repository_id,title,created_at}, Message{id,conversation_id,run_id,role,text,created_at}, RunStatus{Admitted,Running,Completed,Failed,Interrupted}, RunRecord{id,conversation_id,workspace_id,request_id,user_message_id,epoch,status,input,created_at,finished_at,error}, WorkspaceState{conversation_id,workspace_id,revision,view_ids,selected_view_id}; timestamp/IDs injected. Final message references run. Run error is safe stable AppError, no raw runtime exception.
- [x] Define strict SelectedReference enum Dataset{id}, View{id}, Binding{id}; FrozenReference retains reference plus exact trusted bounded serialized payload/checksum. Dataset payload includes immutable header and bounded explicit data selection (first bounded page with coverage), not a latest query. Views pin immutable dataset ID/kind/revision, not mutable presentation status. Binding pins immutable record, not latest active status. Prior assessment references stay within existing review tools; no new review-store federation.
- [x] Define SendMessageRequest{conversation_id,request_id,text,selected:Vec<SelectedReference>}, ContextExchange{messages,status}, ContextSnapshot{policy,serialized,message_ids,references,omitted_messages}; pure build_context(new message, history newest-first complete exchanges, frozen refs, limits). Policy conversation-context-v1. Entire required new message/references must fit; walk newest exchanges as a suffix, stop at first that cannot fit (do not skip a large recent exchange to include old), output chronological; actual full JSON size including omission count checked. Incomplete exchange contains original user message plus explicit status; no partial assistant text. Cap candidate-history SQL reads in Task2, omission count derived from indexed counts (no all-history loads).
```rust
// Boundary assertions must use actual serialized bytes, including escaped source text.
assert!(snapshot.serialized.len() <= limits.context_bytes);
assert_eq!(snapshot.policy, "conversation-context-v1");
assert!(build_context(&oversize_required_input, &[], &[], &limits).is_err());
```
- [x] Test exact/one-byte-over bounds, escaping/multibyte text, required evidence cannot fit, complete-pair suffix selection, interrupted marker, deterministic IDs/order/omission metadata and all limits reject zero/huge. Implement pure code with no IO/model calls. Keep bounded allocations during serialization.
- [x] Run affected lugus-agent runtime/review tests, context tests, strict affected Clippy/fmt; scoped commit feat(agent): separate conversation run identity. Report exact exported structs/signatures and any essential deviations before Task2.

## Task 2: Durable scoped store, workspace layout and execution ownership

**Files:** Create lugus-app/src/conversations/store.rs, ownership.rs and focused storage modules under store/conversations/ for schema, reads, runs, journal, workspace; modify store.rs, store/sqlite.rs, store/views.rs; tests/conversations_store.rs, conversations_ownership.rs. Add application schema v3, no financial/review schema changes.
**Consumes:** Task1 typed contracts and context builder; existing SqliteApplicationStore/EvidenceRepository/scoped references/Clock/IdSource.
**Produces:** ConversationStore trait (ApplicationStore supertrait or required extension supported by application store); ownership port/LocalExecutionLease; exact record APIs in report for Task3.
- [x] Write real-SQLite RED tests: idempotent atomic create conversation+workspace, send user message+Admitted run, same request returns same receipt even after completion, conflicting input fails, second active turn returns Conflict without extra message, concurrent connection one winner, cross-workspace/repository denial, v2 migration preserves raw old rows, opening second store does not recover.
```rust
assert_eq!(first.id, retry.id);
assert_eq!(store.messages(&conversation.id, page)?.items.len(), 1);
// A stale execution epoch must not finish or append tool/activity records.
assert!(store.finish(&stale_attempt, completion).is_err());
```
- [x] Implement ConversationStore methods create/list/read/messages/workspace/run/runs/activity/tool_records, admit, start, append_activity, begin_tool, finish_tool, finish_run, recover. Exact names may follow local conventions; full typed signatures in report. Reads scoped via conversation-owned workspace/repository, page preflight in one SQLite snapshot before decode. Immutable messages/inputs/tool intents/results; mutable status indexes updated only via fenced transactional transitions. Terminal message+status atomic. Dedupe admission before rereading changed context so identical retry returns original frozen input.
- [x] Admission loads required selected refs through existing bounded scoped APIs; rejects invalid/unowned/oversize evidence. Prepare frozen context and check history revision in final immediate transaction; concurrent workspace/metadata changes cannot change frozen refs. Store immutable serialized context before execution. Count prior messages using SQL; read only bounded latest whole exchanges. No source/user data promoted to trusted instruction fields.
- [x] Implement persistent epoch activation/recovery requiring matching acquired lease key; active-run writes require epoch+run status. Recovery marks active attempts Interrupted, retains partial events and unknown tool outcomes, never executes tools. Completing twice returns same result only if identical; contradictory final text/status conflicts. Late failure/cancel cannot downgrade Completed.
- [x] Implement bounded journal: reserve result capacity before begin_tool dispatch, enforce aggregate count/bytes across records, same call ID+same args returns existing completed result without redispatch; pending duplicate returns explicit conflict/unknown, changed args conflict. Safe call failures stored; activity budget exhaustion cannot consume terminal reserve. Bounded paginated reads include unknown pending intents after recovery.
- [x] Add LocalExecutionLease using concrete File::try_lock policy above. Test competing handles/processes, symlink aliases, drop/reacquire, no recovery by ordinary reads, epoch fencing; no lock-file deletion. Errors sanitized. Document local-only lock semantics and canonical key binding.
- [x] Make view acceptance append atomically to managed conversation workspace only, increment revision; agent append preserves prior selection, first may select. Add expected-revision select/reorder/close pure transition + transactional mutation. Reorder requires exact current set/no duplicates, select must exist, close keeps historical receipt; closing selected chooses next at same index or previous last, None if empty. New creation generates fresh workspace IDs; legacy standalone accept_view unchanged. Idempotent accepted view retry must not reopen a closed tab. Bound full layout envelope before mutations.
- [x] Test two companies/views, first/next agent selection, stale layout revisions, rollback if layout cap exceeded, duplicate view retry after close, selected refs immutable after later status/presentation/data changes, all journal/page/context limits before allocation/effects. Run focused store/ownership/legacy refs+bindings tests, strict affected Clippy/fmt; commit feat(app): persist conversations and workspace state. Report APIs/epoch/lease semantics for Task3.

## Task 3: Supervised conversation host and recorded tool/runtime execution

**Files:** Create lugus-app/src/conversations/host.rs, execution.rs, journal.rs, runtime.rs; application/conversations.rs as needed; modify application.rs/config.rs exports only where required; tests/conversations_host.rs plus fixtures.
**Consumes:** Task2 exact store/ownership APIs, Task1 RunSubject/context, existing Application/ResearchExecutor and AgentRuntime.
**Produces:** ConversationHost lifecycle/composition API; async create/read/list/workspace mutations, send/cancel/wait/status, context/activity reads; injected RuntimeFactory creates fresh Box<dyn AgentRuntime> per turn. Existing Application can be wrapped/owned; no cyclic strong references. Exact API reported to Task4.
- [x] Write RED real-store/runtime tests for thesis-free run, two-turn frozen history/evidence, bounded independent conversations, duplicate send no second runtime, same-conversation concurrent conflict, full global admission no message/run side effect.
```rust
assert!(matches!(captured.subject, RunSubject::Conversation { .. }));
assert!(captured.context.is_empty()); // untrusted history belongs to structured user data
assert_eq!(runtime_invocations.load(Ordering::SeqCst), 1); // repeated send receipt
```
- [x] Implement exclusive ownership activation before recovery; bounded admission supervised before returning receipt. Check existing identical request first so retries can return durable receipt even when active capacity is full; only new admissions acquire capacity. RuntimeFactory errors finalize accepted run Failed, not leave Running. Capture immutable offering once, create scoped ResearchExecutor, wrap journal, send trusted instructions separately from frozen serialized data. Native web search defaults false. Context persistence and admission happen before any runtime start. Always check runtime report ID/outcome/size before final message commit.
- [x] Journal wrapper validates run/call identity and bounded inputs before intent persistence, then executes existing tools, records exact bounded result before returning it. Duplicate completed calls return saved results; unknown/pending duplicates never replay. On budget/storage failure return safe bounded error and stop turn; do not fabricate successful result. Keep ResearchExecutor cancellation and per-fetch ownership unchanged. Terminal statuses cannot imply every initiated tool completed if result missing.
- [x] Drain runtime events on bounded channel concurrently; append bounded partial text/activity under attempt fencing. Persisting activity failure triggers cancellation/finalization; event backpressure cannot prevent runtime close or terminal persistence. Enforce deadline/cancel against uncooperative injected runtime, close with finite timeout; retain explicit cleanup error rather than success. Own cleanup in spawned supervisor, not caller future.
- [x] Add supervised shutdown: close admission, cancel runs, wait registered supervisors including in-progress admission, close all runtimes, then shut down owned Application/provider jobs and release lease. Multiple/cancelled shutdown callers await shared outcome; never release ownership while writes/children can continue. Normal public reads can open without runtime/lease/recovery. No automatic turn replay.
- [x] Test cancelled send/wait/shutdown futures, runtime panic/factory failure/wrong ID, dropped event receiver/full events/storage error, cancellation around final-message commit, duplicate tool ID, unknown crash window, second host rejected, completed state after restart, stale epoch writes rejected and old runtime cannot affect new owner. Cover partial tool effects with real synthetic providers and exact durable refs.
- [x] Run focused host+legacy lifecycle/agent/review tests, strict affected Clippy/fmt; commit feat(app): supervise durable conversation turns. Report exact APIs and deterministic runtime setup for Task4; root owns final broad suite.

## Task 4: Shared CLI and restart/continue acceptance

**Files:** Extend lugus-app/examples/research.rs with focused conversation command module; examples/support/conversations.rs; examples/synthetic/setup.py; tests/conversations_cli.rs; README; config adapter if required to open conversation services. Keep existing agent-fixture/thesis/direct-native/binding CLI supported.
**Consumes:** Task3 host API; Task1 RunSubject; existing synthetic source and automatic binding tool chain.
**Produces:** Reproducible CLI create/send/show/messages/runs/workspace/context/layout/cancel/recover/continue commands using shared host; deterministic actual AgentRuntime provider selected explicitly by fixture command, no fake thesis or seeded binding.
- [x] Write RED CLI process test: new conversation without thesis, send actual12-tool Apple/AAPL research, persist final answer/view; shut down, remove manifest, reopen messages/layout/original evidence offline; fresh runtime follow-up verifies prior context and selected dataset; no previous calls replayed.
```rust
assert_eq!(restored.messages, before.messages);
assert_eq!(restored.workspace.view_ids, before.workspace.view_ids);
assert_eq!(continued.subject_kind, "conversation");
assert_eq!(completed_first_run_tool_count_after, completed_first_run_tool_count_before);
```
- [x] Implement CLI through shared host; fresh runtime per send. Use strict external JSON request/config, safe structured status/errors, finite bounds; distinguish offline reads from explicit execution/recovery ownership. No model/network credentials required. Add views-first empty conversation path and layout persistence/revision commands.
- [x] Real process crash fixture: hold run after a durable tool intent or partial output, kill only spawned fixture process, next exclusive host recovers Interrupted with pending tool unknown, completed earlier run unchanged, no automatic tools; explicit new message succeeds. Second simultaneously running host cannot recover. Preserve exact source refs after binding supersession/new dataset/closed tab.
- [x] Add focused malformed/oversized input and cross-conversation CLI tests; keep negative assertions on durable record counts not only exit status. Document commands/output keys/ownership/local filesystem limits/context omission semantics/partial text; no desktop presentation claims.
- [x] Run focused CLI+host tests, strict affected Clippy/fmt; scoped commit feat(app): add durable conversation CLI acceptance. Full report gives root exact standalone commands and expected IDs/state/call-count evidence.

## Final verification and completion (root)

- [x] Run workspace/all-target offline locked tests, workspace/all-target/all-feature strict Clippy, fmt, unchanged SEC/yfinance suites as appropriate. Execute fresh standalone CLI flow and crash/recover/continue using only returned IDs. Confirm legacy reviews and binding acceptance still work.
- [x] Whole-branch review against e65942b; fix confirmed findings with regression and scoped re-review. Record test counts and material limits.
- [x] Update roadmap milestone3 complete/nextmilestone4 filing text/passages, preserve pending main files, keep branch/worktree local. No merge/push. Remove only this completed plan's temporary SDD workspace after completion record committed.

## Implementation and verification record

Implementation is local on `codex/durable-conversations` in `/private/tmp/lugus-backend-development`, through `af1aa68`, based on the completed binding milestone `e65942b`. Task commits: contracts `4f4cf23`/`63460e8`; storage `ef755ad`; host `d25c698`/`5ce2495`; CLI `3c983e3`/`1d13517`. Independent task reviews and scoped fix reviews are complete. Review found and fixed restored-reference validation, post-admission supervision/publication races, and CLI stdout backpressure.

Verified on the corrected implementation:

- `cargo test --workspace --all-targets --offline --locked`: **363 passed**, 51 suites, zero failures or ignored tests; includes runtime/Codex adapter, legacy thesis-review and binding CLI regressions.
- `cargo clippy --workspace --all-targets --all-features --offline --locked -- -D warnings`, `cargo fmt --all --check`, and `git diff --check`: passed.
- SEC Python adapter: **31 passed**; pinned yfinance environment: **23 passed**. Adapter source hashes remained unchanged through final verification.
- Independent fresh CLI process workflow: twelve actual research tools, offline restoration after manifest removal, a fresh zero-tool follow-up with three frozen references, original closed-view evidence readable, an actual killed owner with pending tool outcome retained, rejected competing recovery, recovery without inserted messages/tools/activity, and a new explicit twelve-tool turn. Earlier completed records stayed unchanged. No thesis database or live model/provider credentials were used.

The CLI tests additionally cover views before messages, source-supported binding supersession, new datasets and layout revisions without reinterpreting old references, bounded invalid controls with no writes, idle stdin, unread stdout, broken pipes, provider cleanup and lease release.

Implementation decisions:

- Conversation limits are persisted and shared across handles. An explicit conflicting configuration is rejected; ordinary opens load stored limits. Changing those limits requires a future explicit configuration migration.
- CLI cancellation is a bounded stdin command to the owning send session. Separate processes can inspect state but cannot control a live owner; detached cancellation would require a future transport.

Limits: execution locking supports canonical local files and symlink aliases, not hard-link aliases or network filesystems. Unknown tool outcomes are never replayed automatically. Runtime factories own cancellation-safe startup until returning a runtime; callers explicitly shut down the host. Permanent storage failure is surfaced and may require later recovery. The CLI uses a deterministic actual AgentRuntime fixture; live model acceptance, document text extraction, refresh and desktop rendering remain outside this milestone. CLI final output has a finite delivery deadline after host cleanup; a slow or broken output pipe may leave incomplete NDJSON while durable status remains readable.

Final whole-milestone review and scoped re-review are complete with no open findings. Final fix `af1aa68` orders journal completion accounting before result publication, preserves safe durable `authentication_required` and `needs_attention` categories through offline reopening, formats the schema SQL without semantic changes, and adds deterministic coverage for an external metadata commit during context preparation. All four final-review findings were verified addressed.

The final corrected tree passed the full 363-test workspace run, strict Clippy and formatting, and another independent standalone workflow. Milestone 3 is complete; milestone 4, filing text and passage references, is next. The main-checkout roadmap is synchronized only after checking its saved content hash. No merge or push was performed. Temporary execution/review scratch is removed after this completion record is committed; the implementation, decisions, limitations and verification record remain here and in the application README.
