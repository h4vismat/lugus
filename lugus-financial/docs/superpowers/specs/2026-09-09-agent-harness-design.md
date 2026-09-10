# Lugus agent harness and durable knowledge

Date: 2026-09-09
Status: Draft for user review
Implementation target: sibling workspace crate `lugus-agent`

## Purpose and agreed product behavior

Lugus is a local investment workspace. Users write investment theses freely, discuss them with an integrated agent, and track investments. Its experience resembles an IDE supplying context and tools to an embedded agent runtime.

Codex is the initial runtime. Compatible model providers can be configured underneath Codex; this project does not initially implement a direct model API agent loop. Other agent runtimes must remain possible through adapters.

Each thesis has a logical agent identity and its own assessment history. Runtime sessions are disposable. Lugus-owned storage preserves knowledge across sessions and makes it available across theses belonging to the local user. Gbrain is architectural inspiration, not a dependency.

Agents may research the web, read external documents, and autonomously revise their interpretations, shared knowledge, and graph relationships. They learn through measurable outcomes and user feedback. Learning means persisting and revising useful knowledge and lessons, not training model weights.

Unattended reviews run only while Lugus is open. Financial evidence changes, research refresh schedules, and outcome deadlines can trigger work. Reopening the application checks for changes and recovers pending work. No always-running operating-system service is required.

## Architectural boundaries

The proposed architecture has four boundaries:

1. **Domain and state transitions:** immutable records and pure functions validate commands, determine due work, calculate transitions, and check revision preconditions.
2. **Application coordination:** executes effects such as retrieving evidence, starting a runtime turn, and committing a validated assessment. It owns scheduling and cancellation while the application is open.
3. **Ports:** runtime execution, durable storage, financial evidence, research/document retrieval, and clocks are explicit interfaces. Domain types contain no Codex protocol types or provider SDK objects.
4. **Adapters:** Codex process/protocol integration, local database storage, and integration with `lugus-financial` implement those interfaces.

Initially these can be modules within `lugus-agent`, with a dependency on `lugus-financial` at the integration boundary. Memory is a separate interface, not necessarily a separate service or crate. Extract additional crates only when consumers or implementation size justify them.

The application owns process lifetime. The harness is usable independently of any desktop UI framework. Runtime events map to Lugus events that a future UI can consume.

## Durable records and graph

### Thesis

Store original free-form text and user-authored revisions separately from agent interpretations. An interpretation identifies claims, assumptions, related entities, expected outcomes, and time horizons where inferable. Mark whether a criterion came from the user or was inferred. Ambiguity may remain explicitly unresolved; agents ask when it materially affects assessment rather than inventing precision.

The thesis identifier is durable. Runtime thread identifiers are optional execution metadata and do not define thesis identity.

### Evidence

Financial evidence references the preserved observations in `lugus-financial`, including provider identity and observation/revision identity. Research evidence retains source URL, retrieval time, publication time when available, and the stored content or extract actually used. Content hashes identify captured versions. If full content cannot be obtained, record the capture's limited scope.

Distinguish when an event happened, when its source was published, and when Lugus obtained it. A past assessment must remain interpretable using the evidence available to it at that time. A retrieval timestamp alone does not establish source freshness or completeness.

### Assessment

An assessment references the thesis interpretation and evidence versions it evaluated. It records conclusions, supporting and opposing evidence, uncertainties, changes from the previous assessment, and the observed outcomes or feedback considered. Record model/runtime identity where available and the originating run identifier.

Assessment prose need not imply a binary valid/invalid verdict. Insufficient evidence is a legitimate result.

### Shared knowledge and lessons

Reusable findings and lessons retain provenance, applicability, revision history, and status such as active, contested, or superseded. Lessons reference retrospective assessments and their underlying feedback or measured outcomes. An unsuccessful investment does not by itself prove the reasoning was wrong.

Agents can revise these records autonomously. Revisions reference their predecessor and reason for change. Conflicting interpretations can coexist as contested knowledge; last-writer-wins replacement is not an acceptable conflict policy.

### Graph and retrieval

Use stable identifiers and typed edges such as supports, contradicts, depends-on, concerns, and supersedes. Edges retain provenance and revision history too. The graph is a domain model, not a requirement for a dedicated graph database.

Proposed first storage adapter: SQLite, consistent with the existing local financial component. Keep agent tables in a separately owned database and reference financial evidence by stable identifiers through the financial port; do not couple to financial table internals. Application transactions must not assume atomicity across both databases.

The first retrieval contract supports lookup, text search, and bounded graph traversal. Embeddings are an optional derived index that can be rebuilt from canonical records. Semantic retrieval must not become the sole route to exact evidence references or revision history.

## Review lifecycle

Persist review requests and their triggering inputs. States are queued, running, completed, interrupted, failed, and blocked. Blocked means a recoverable external condition, such as authentication, requires attention; interrupted means execution ended without a completed result.

1. Detect changes or due work. Persist a cursor only after corresponding work has been durably recorded. Repeated retrieval of identical financial observations does not by itself constitute new evidence.
2. Resolve affected theses using recorded subscriptions and relevant dependencies. Use bounded traversal and deduplicate matches. The graph is incomplete knowledge, so scheduled research refreshes remain necessary.
3. Coalesce pending triggers per thesis. Allow one active review per thesis initially. Capture the selected input revisions for the run; inputs arriving afterward remain pending.
4. Assemble a bounded starting context from the thesis, last assessment, triggers, and relevant memory references. Give the runtime tools to retrieve more context rather than loading the entire database.
5. Execute through the runtime adapter and enforce configured time/concurrency limits. Usage reporting is optional; use it when the runtime supplies it and do not claim an exact monetary cap without reliable pricing and accounting.
6. Save the final assessment through a structured tool command. Validate identifiers, evidence references, and revision preconditions. Commit the assessment and the completion of its selected review inputs atomically in agent storage.
7. Emit application events for updated assessments and material changes. The notification delivery mechanism belongs to the future UI.

Intermediate evidence captures and memory revisions can commit independently and survive interruption. Such writes carry stable operation identifiers and an originating run reference. Retry of the same operation is idempotent. A completed runtime turn without a valid persisted assessment does not complete a review.

Memory changes must not create unlimited immediate review cascades. Queue deduplicated affected work and apply bounded scheduling; a review must not continually retrigger itself from its own writes. Source changes arriving during a run are retained for follow-up.

On application shutdown, stop scheduling, request cancellation, and terminate/reap owned processes after a bounded grace period. On startup, mark abandoned running work interrupted and make it eligible for retry. Recovery starts from persisted state and may use a fresh runtime session; it does not depend on resuming a partially executed model turn.

## Codex adapter

Use the Codex App Server over local stdio as the proposed initial integration. It supplies application embedding primitives for authentication, threads, turns, approvals, and streamed events. Translate protocol messages inside the adapter. Probe the installed version and test against an explicitly supported schema; App Server is experimental.

Expose Lugus capabilities through tool contracts for thesis reads, memory search/traversal, evidence access/capture, revisions, and assessment submission. Transport binding—App Server tool callbacks or a local MCP endpoint—must be selected in the protocol feasibility work before implementation. This is an integration decision, not a change to the agreed ownership boundaries.

Use Codex-supported authentication flows and configuration. Do not copy credentials into Lugus knowledge records or logs. Whether a particular account or provider can execute a task is established during setup/runtime checks, not assumed from API compatibility.

Unattended execution uses explicitly permitted tools and a policy that cannot wait indefinitely for a human. Denied operations return a structured failure or block the run. Interactive conversations may surface approval requests in the future UI. Do not equate unattended work with unrestricted shell or filesystem access.

Research availability is a capability check: compatible model providers do not automatically supply native web search. Native research can be used where available, but evidence used in an assessment must be captured through Lugus's evidence contract. A future interchangeable research adapter can supply missing capabilities.

## Trust and consistency

Source documents and retrieved memory are data, not authority to change execution policy. Validate tool inputs and enforce write scope in the host. The thesis text remains distinguishable from agent edits. A session's access is scoped to the local Lugus workspace and its configured capabilities.

Use optimistic revision checks for shared records. On a stale write, return the current revision so the agent can reconsider; do not silently overwrite. Preserve independently sourced corroboration separately from several agents repeating one source.

Schema validation establishes structure and referential integrity, not factual correctness. Evidence coverage, uncertain conclusions, and feedback remain visible for later review.

## First implementation milestone

Deliver a headless harness in `lugus-agent` that can later be hosted by the desktop application:

- Durable thesis, evidence-reference, assessment, shared-memory, and graph records with a SQLite adapter.
- Runtime-independent execution and tool contracts plus a Codex adapter.
- Explicitly invoked thesis conversations/reviews and a coordinator callable while the application is open.
- Financial evidence integration, source-change deduplication, due work, and recovery.
- Evidence capture for external research and a verified research path with the initially supported Codex configuration.
- A runnable example demonstrating persistence across fresh sessions and revision after evidence or feedback changes.

The first milestone excludes desktop UI, an Obsidian-style graph renderer, portfolio accounting, trading execution, a background OS service, additional runtime adapters, mandatory vector infrastructure, and model fine-tuning. These product extensions must not be implied by completion of the harness.

## Verification and acceptance

Use pure-domain tests for state transitions and dependency resolution, temporary-database integration tests for persistence and conflicts, and a deterministic fake runtime for failure/recovery paths. Protocol fixtures must match the supported installed Codex schema. Live authenticated checks are separate from deterministic tests.

Acceptance scenarios:

1. Session A assesses a free-form thesis and saves knowledge; a fresh session B retrieves it for a related thesis without A's transcript.
2. A changed financial observation queues the affected thesis once; repeated identical retrieval does not. The next assessment references new evidence and retains the old assessment.
3. An update arriving during a review remains pending after completion.
4. Outcome evidence or user feedback produces a traceable retrospective lesson retrievable in a fresh session.
5. Two sessions revising one memory encounter a detectable revision conflict rather than silent data loss.
6. Process failure or application closure leaves recoverable work; replay does not duplicate committed operations or assessments.
7. Authentication failure, missing research capability, malformed tool output, and a turn ending without an assessment produce explicit non-success states.
8. One supported live Codex setup executes Lugus tools, captures external evidence, saves an assessment, and responds to cancellation.

## Implementation sequencing

First verify Codex tool binding, authentication handling, event schema, cancellation, unattended policy, and research capability with the installed version. Then build the domain/storage slice and runtime-independent tests, connect financial evidence and tools, implement the Codex adapter, and verify the end-to-end milestone. The detailed implementation plan follows user review of this document.

## References checked

- Codex App Server: https://learn.chatgpt.com/docs/app-server
- Codex provider configuration: https://learn.chatgpt.com/docs/config-file/config-advanced
- Architectural inspiration only: https://github.com/garrytan/gbrain
- Existing financial architecture: `2026-09-09-financial-ingestion-design.md` in this directory.

The local Codex CLI reported version `0.153.4` during design exploration. No authenticated agent execution has been tested in this brainstorming phase.
