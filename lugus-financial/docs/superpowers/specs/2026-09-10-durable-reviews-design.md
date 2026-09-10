# Durable investment reviews

Date: 2026-09-10
Status: Implemented and verified. Scope and architecture approved in conversation; implementation details below refine the approved 2026-09-09 design.

## Milestone
Save a free-form thesis, explicitly review already-ingested financial evidence, restart, and review changed evidence with the previous assessment available in a fresh runtime. Preserve original thesis revisions, agent interpretations, evidence versions, conclusions, counterevidence, uncertainty, open research questions, and assessment history. Automatic triggers, shared knowledge/graphs, external research capture, UI and trading are later milestones.

## Boundaries
`lugus-agent::reviews` owns pure domain validation, a synchronous transactional store port, SQLite adapter, financial evidence adapter, and async coordinator. Runtime protocol types stay in the existing adapter. The financial adapter consumes the public financial Repository snapshot API without accessing its tables or modifying ingestion. The agent database owns immutable copies of selected evidence payloads plus their provider/source references, allowing past assessments to remain readable even if the financial database is unavailable. Storage and runtime are replaceable.

## Records and transactions
Thesis revisions retain user text and creation time. Updates use an expected revision. A persisted review request captures the exact thesis revision, previous assessment ID, and immutable selected evidence before execution. Caller-supplied review IDs are idempotency keys; a repeated ID with different inputs is a conflict. Each attempt has an increasing fencing token and runtime identity supplied by the host. Only one running review per thesis is allowed. An assessment contains interpretation, conclusion, supporting/opposing claims with evidence IDs, uncertainty, open questions, and a changes explanation. Evidence IDs are content hashes of provenance and content; their payloads are stored with the queued request.

Submission validates nonempty bounded text, references within the selected evidence, current thesis revision, current prior assessment, run/attempt ownership and status. Unsupported questions remain open; structural validation does not certify factual accuracy. One transaction inserts the assessment and completes the review. Repeating an identical submission is a no-op; conflicting content is rejected. Completion survives subsequent runtime failure or cancellation. Old attempts cannot submit after recovery/retry. Pending reviews with stale thesis/assessment inputs require a new explicit request.

States: queued, running, completed, interrupted, failed, blocked. Only explicit startup recovery (after the host has stopped all previous coordinators) changes abandoned running records to interrupted. Opening a database alone does not recover active runs. Runtime completion without submission is failed; cancellation is interrupted; authentication/attention errors are blocked. Dropped coordinator futures leave a recoverable running record. Retry retains selected inputs, increments the attempt, and starts fresh. Startup recovery and submission are serialized transactions. No cross-database transaction is assumed.

## Context and tools
Trusted instructions explain the review process. Thesis, prior assessment, and evidence metadata are serialized as task data in the user input, never developer instructions. Evidence payloads are retrieved via a scoped read tool. A submission tool accepts a strict typed assessment. Tools cannot read other reviews or update thesis text. All references must belong to the frozen review selection. Native web search is disabled per review request in the Codex adapter; no live research capability is exposed by the coordinator.

Bounded records: thesis 16 KiB; assessment JSON 32 KiB; up to 64 evidence records of 64 KiB each; starting context 64 KiB. Oversized selection/context fails explicitly. Runtime timeout, tool-call/result limits and cancellation are supplied by the host. Cancellation gives the runtime a two-second acknowledgment window, then bounds close to three seconds; replacement runtimes own cleanup on drop. Financial capture may enumerate larger snapshots so callers can explicitly select a bounded subset before queueing. SQLite calls are short synchronous transactions and never held across awaits; callers with large workloads should host coordination off the UI thread.

## Acceptance
Real temporary SQLite databases verify reopen/history, evidence immutability, conflicts, idempotency, one active review per thesis, stale attempt rejection, interruption before/after commit and recovery. A deterministic runtime drives real tools/coordinator; financial adapter tests use the real financial store. A runnable CLI demonstrates create/edit/show/queue/run/recover with an explicit live Codex run. Live model execution is separate from deterministic verification.
