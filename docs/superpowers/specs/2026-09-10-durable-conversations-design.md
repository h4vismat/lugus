# Durable conversations and research workspaces

Status: approved by the user on 2026-09-10. Implementation complete; see the [completed plan](../plans/2026-09-10-durable-conversations.md) for verification and limits.
Base: `codex/instrument-binding` at `e65942b`.
Design branch: `codex/durable-conversations` in the existing isolated worktree.

## Intended behavior

Create a research conversation without creating an investment thesis. A user may
start with a message or open a research view first. Messages, selected evidence,
agent runs and workspace views survive application restart. A subsequent disposable
agent turn receives bounded, explicitly assembled prior context. Reopening never
replays a completed model turn or repeats its tools.

A conversation owns one workspace in this milestone; that workspace may cover
several companies. Investment theses and their assessments remain separately
identified, explicitly requested records. Conversation text does not become a
thesis or an assessment automatically.

## Approaches and recommendation

1. Recommended: application-owned relational conversation storage and a small run
   coordinator. Reuse the application database and financial/view references; keep
   state transitions and context selection pure. This fits existing scope and
   transaction boundaries and works with any AgentRuntime implementation.
2. Persist and resume model-provider thread IDs. This ties recovery and history to
   a provider, leaves local tool effects harder to reconcile, and conflicts with the
   disposable-turn architecture. Provider session IDs may be diagnostic metadata,
   but are not conversation identity or recovery authority.
3. Introduce a general event-sourcing platform. Replayable aggregates would add
   infrastructure and migration complexity beyond this milestone. Use immutable
   messages/run inputs and bounded run events with ordinary transactional indexes.

## Ownership and runtime identity

`lugus-app` owns conversation/workspace storage, context preparation and run
coordination. Keep these in focused modules behind persistence, clock, ID and
runtime-factory ports. SQLite and process adapters implement effects; pure functions
validate transitions, choose context and update workspace selection.

Replace the generic RunRequest's mandatory thesis identity with an explicit subject:
conversation or thesis, each with its own identifier. Adapt review coordination to
use the thesis subject and new conversation coordination to use the conversation
subject. Preserve stored thesis revisions, reviews, assessments and their fencing
semantics. If legacy serialized RunRequest values are accepted, map an explicit
legacy thesis_id to a thesis subject only; reject conflicting identities. Never
invent a thesis for a conversation.

Runtime providers do not own conversation storage. Each turn starts a fresh runtime
session, with an immutable tool offering and application-derived workspace/run/call
scope. Conversation text, prior assistant text, evidence and tool results are data,
not developer instructions. Keep trusted instructions separate when adapting input
to the existing runtime's prompt/context fields.

## Durable records and transactional operations

Persist conversation metadata, workspace identity, immutable user/assistant
messages, immutable per-run input snapshots, run lifecycle, bounded activity/tool
records, selected-context references, and ordered open-view references. All records
are scoped to the owning conversation/workspace and financial repository identity.
Lists and reads use explicit pagination and byte preflight before decoding.

Creating a conversation and its empty workspace is atomic and idempotent. Sending a
message atomically stores the user message and admits one run using a caller request
ID; identical retries return the existing receipt, conflicting inputs fail. Permit
one active turn per conversation, with bounded global admission. A second send while
that conversation is active returns a conflict before appending a message. Different
conversations can run independently within configured limits.

A run moves through admitted/running to completed, failed or interrupted. A final
assistant message and completed status commit together only for the current owned
attempt and matching runtime run ID. Streaming partial text is retained as bounded
run activity; it is never promoted to a completed assistant answer after failure.
Record stable safe failure categories, including authentication/attention needs.
Completed results cannot be overwritten by late cancellation or stale callbacks.

Persist tool-call intent before dispatch and bounded results afterward through a
conversation executor wrapper around the existing ResearchExecutor. Do not infer
success from runtime events or invent receipts after a crash. Retain exact call IDs
and origin run/request IDs so already-committed application evidence remains
inspectable. Exhausted persistence budgets prevent further tool dispatch. A crash
between a tool effect and its result record leaves that call's outcome unknown;
recovery preserves the effect and marks the run interrupted, without replaying it.

## Context for the next turn

Use a deterministic, versioned context builder. Include the new user message,
explicitly selected immutable evidence references and a suffix of prior conversation
history. Completed exchanges remain intact; interrupted/failed activity has explicit
status and is not presented as a completed assistant answer. Preserve chronological
order and record which earlier messages were omitted.

Freeze the exact selected message IDs, reference versions, serialized context and
selection-policy version before runtime execution. Later messages, view changes or
new financial observations cannot reinterpret an existing run's inputs. Resolve
selected references through existing scoped bounded reads; raw model-supplied
provenance cannot enter the snapshot. Required user input or selected evidence that
cannot fit fails explicitly; optional older history may be omitted with a recorded
coverage marker. Do not silently truncate required evidence or claim full history.

Use explicit finite limits for message bytes, context bytes/items, per-run activity
and tool-record bytes/counts, open views, pages, active runs and pending events. The
implementation plan must enumerate defaults and boundary fixtures before coding.
This version does not add model-generated summaries, embeddings or vector search.
Older durable history remains available through bounded local reads.

## Workspace restoration and evidence meaning

Persist open view IDs, their order and the selected view separately from immutable
dataset meaning. View acceptance and attachment to a managed workspace must be
transactionally consistent and idempotent. Agent-opened views append without
replacing existing views or changing an existing selection; the first view may
become selected when the workspace was empty. Explicit user select/reorder/close
operations use an expected workspace revision to reject stale updates.

Closing a tab removes it from current layout, not from historical messages or saved
evidence. A run's selected context is independent of later tab selection. Restoration
returns persisted descriptors and state; it does not claim the views were rendered
again. Dataset, document, binding and existing assessment references keep their
original versions. No refresh, filing passage extraction or automatic reassessment
is introduced.

New conversations receive new host-owned workspace IDs. Existing standalone
workspace references remain readable through existing APIs; do not silently adopt
or rewrite them into conversations during migration. An explicit adoption/import
workflow can be added separately if needed.

## Startup, shutdown and interrupted work

Acquire exclusive ownership of the conversation execution store before recovering
abandoned runs. A second host or an ordinary read-only connection must not mark a
live host's work interrupted. Isolate local ownership/locking behind a port; the
implementation plan must specify and test the concrete ownership mechanism.

On startup after ownership is established, mark abandoned admitted/running attempts
interrupted and fence their future writes. Completed runs stay completed. Restore
conversations and workspaces through local reads without starting a runtime or
fetching provider data. A user continues by sending a new message; there is no
automatic resume/retry of interrupted turns or pending tool calls in this version.

During shutdown, stop new turn admission, cancel active runtime/tool work, await
supervised finalization and close runtimes before releasing ownership. Preserve
existing financial worker cleanup, cancellation and durable receipt behavior. A
dropped caller must not abandon cleanup; a killed process is handled by startup
recovery. No database transaction spans a runtime/provider await, and synchronous
storage work remains outside the async executor.

## Migration, tests and executable acceptance

Add an application schema migration from v2 without rewriting existing evidence,
bindings, datasets, views or review databases. Keep legacy manual application APIs
and thesis-review workflows functional. Validate old reads after migration and
cross-workspace/repository denial for all new operations.

Use real SQLite databases, real synthetic provider processes and deterministic
AgentRuntime implementations. Cover message/run dedupe and concurrent admission,
context budgets and omission markers, frozen source references, stale attempt
writes, event backpressure, cancellation, dropped callers, crash recovery with
exclusive ownership, atomic final-message completion and workspace revision races.

Expose shared manual APIs and a reproducible CLI acceptance workflow:
1. Create a conversation without a thesis and send a question.
2. The actual runtime uses research tools to resolve Apple, bind AAPL, fetch prices
   and accept a view; save the assistant answer and run provenance.
3. Shut down and reopen offline, restoring messages, ordered views, selection and
   exact original dataset/binding references without model or provider calls.
4. Start a fresh runtime turn with a follow-up question; assert its frozen context
   contains the intended earlier message and selected evidence.
5. Interrupt a separate run, restart, confirm interrupted status and no repeated
   calls; successfully continue through a new explicitly requested message.

Run workspace tests, strict Clippy, formatting, affected adapter regressions and
standalone CLI checks. Review the full change before marking milestone 3 complete.
No live model/network run, merge or push is required by this design.

## Scope limits

This milestone establishes backend persistence, orchestration and restoration.
Desktop rendering, background refresh, passage extraction, automatic summaries,
conversation branching/editing, deletion/retention policies and a new thesis-review
UI are later work. Existing explicit review functionality remains intact.
