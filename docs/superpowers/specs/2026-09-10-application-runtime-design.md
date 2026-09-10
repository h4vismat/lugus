# Production application runtime and capability routing

Status: approved by the user on 2026-09-10, including the thesis-backed agent boundary. Implementation follows the associated plan.

## Scope and base

Implement backend roadmap milestone 1 on top of `eea83bf` (`codex/company-resolution-observations`). Keep the existing pending product, contract, roadmap, and spike documents intact. No merge, push, or publication is part of this work.

Deliver a framework-independent `lugus-app` Rust crate with configured provider lifecycle, known-capability routing, shared manual/agent operations, bounded results, cancellation, offline reads, and validated view requests. Verify through deterministic integration tests and a CLI fixture workflow.

Company-to-market binding, durable conversations, passage extraction, background refresh, and desktop rendering remain their respective later milestones. Market requests require explicit provider-native instrument identifiers; an SEC resolution must never manufacture a Yahoo mapping.

## Approaches

1. **Recommended: application crate with a worker per provider instance.** Reuse financial ports and selection functions. Each worker owns its mutable provider process and serializes calls to that instance. Different instances can progress independently. Pure functions decide eligibility, validate commands, and transition application state.
2. **One application actor owning all providers and storage.** Simple ownership, but awaiting ingestion blocks unrelated providers and local commands unless a separate execution layer is added. This repeats the spike's central limitation.
3. **Separate orchestration service.** Gives a process boundary to clients, but adds transport, deployment, and failure modes before any desktop transport is required. Explicit Rust ports leave this option open later.

## Ownership and concurrency

`lugus-financial` retains financial domain rules, provider protocol, ingestion, catalog, and evidence selection. `lugus-agent` retains the vendor-independent runtime/tool ports and existing review behavior. `lugus-app` owns application commands, provider catalog, jobs, references, and view acceptance. A CLI composes the adapters.

Separate pure catalog/command/reference validation from effects through provider lifecycle, repository factory, application store, clock, identifier source, and event sink ports. Provider workers use bounded queues and their own repository handles. SQLite transactions stay short and never span provider awaits. Synchronous database work must run outside async executor threads, using a dedicated worker context or blocking boundary appropriate to the existing synchronous repository traits.

Catalog locks protect brief state transitions only. Dispatch reserves an instance generation after validation; deactivation closes admission before cancelling queued and running work. A process restart increments generation, making old turn offerings invalid. Shutdown closes admission, cancels jobs, finalizes outcomes, and explicitly reaps children within configured deadlines.

## Catalog and offerings

Configuration identifies instance ID, manifest path, activation, process configuration, and limits. Reject duplicate IDs and invalid limits before starting processes. Preserve full `ProviderIdentity` and supported negotiated capability versions. Keep activation distinct from observed process availability; a failed instance does not prevent other configured instances or offline operations from working.

Application-owned adapters support the existing v1 company-resolution, filings/document, fundamentals, and market-data capabilities. Unknown capabilities or versions produce explicit unsupported decisions. Do not derive schemas or renderers from arbitrary provider output.

A turn snapshot records catalog revision and offered tuples of instance generation, provider identity, capability version, and operation. Stable tool names have explicit provider-instance arguments. Initially require explicit selection; no implicit default, first-provider choice, or failover.

Dispatch checks both the immutable offering and current catalog state, including at execution after queueing. Activation during a turn cannot enlarge that turn's permissions. Cached reads and view requests remain offered independently of provider availability.

## Shared command boundary

The host constructs a scope containing workspace ID, request ID, and optional agent run ID. The turn executor captures its offering and scope; model arguments cannot replace them. Reject unknown fields, invalid shapes, forged run IDs, and unoffered operations before effects.

Both manual callers and `ToolExecutor` invoke the same typed application operations:

- Resolve/search a company or look up an explicit identifier; retain resolution runs, candidates, and ambiguity. Explicit candidate selection delegates to the existing catalog validation.
- Fetch filings, facts, a primary document, or daily prices through an explicitly selected provider and validated native query.
- Create a frozen dataset reference from authorized stored run evidence, including offline creation when the provider is unavailable.
- Read a bounded page from a dataset or bounded bytes from an authorized original document.
- Open a supported research view, inspect a job, or cancel an owned job.

Fetch submission returns a job receipt. The agent adapter can await that job to return a bounded terminal result; manual clients can observe the same job through status/events. Cancelling a tool wait must propagate to its job while allowing host cleanup to finish.

## Evidence and view references

Persist application-owned opaque dataset IDs in an application store. Each record includes workspace ownership, financial repository identity, full provider identity, query, exact run/retrieval/observation references, frozen selection policy and coverage, and dataset kind. Use existing `select_daily` and `select_facts` semantics; do not recompute an old ID against the latest runs. Filing lists and resolution candidates retain their exact run scope without inventing selection policies.

Original-document references retain checksum and exact provider/source association. Verify both when reading. Do not expose arbitrary filesystem paths or interpret HTML/PDF offsets as passage selections.

Version 1 view descriptors support price charts, typed data tables, and original documents. Validate reference ownership and kind compatibility. Decimal values remain strings; conflicts, missing values, retrieval dates, and source coverage stay explicit.

Persist view acceptance atomically with the request's normalized input and receipt. Repeating a workspace/request ID with identical input returns the existing receipt; different input returns a conflict. A view ID points to a dataset ID; neither accepting another view nor fetching fresh data changes an earlier dataset's meaning.

Acceptance means the application recorded the request. A separate presentation-result contract carries view ID and descriptor revision so stale renderer reports cannot acknowledge a newer request. No renderer success is claimed by the CLI. Persisting these references and receipts is the narrow storage requirement here; conversation history and restoration orchestration remain milestone 3.

## Jobs, cancellation, and bounds

Job states are queued, running, succeeded, failed, and cancelled, with one authoritative terminal result. Events contain job, workspace, and request IDs. Event-stream closure never implies completion; status remains queryable. Event backpressure cannot prevent cleanup or terminal-state recording.

Do not cancel by dropping the entire existing ingestion future: current run finalization happens after its internal await. Introduce cancellation/deadline-aware provider-call boundaries that return a typed failure into ingestion, allowing its existing finalization to run. The plugin's `RequestGuard` invalidates an interrupted connection; explicitly close and reap it before reporting cleanup complete. Preserve committed pages and failed ingestion provenance; represent user cancellation distinctly at the application job layer. Cancellation between pages must also finalize the run. Handle document retrieval and resolution through their corresponding persistence boundaries.

If success commits before cancellation wins the terminal transition, report success. Cancellation cannot roll back committed evidence. Storage/finalization failure is an explicit failure, not a clean cancellation. A cancelled process is unavailable until explicitly restarted; never silently retry through another instance.

Set validated finite limits for queues, concurrent jobs, operation duration, pages, total ingested items/bytes, document size, input size, read pages, and serialized results. Existing per-response plugin limits alone do not bound multi-page ingestion. Reject or paginate before materializing unbounded application results. Limits are externally configurable and tested at their boundaries.

Errors have stable kinds, bounded safe messages, retryability, and optional retry-after metadata. Include invalid input, unsupported operation/version/view, ambiguity, unavailable/deactivated provider, scope mismatch, missing/stale reference, request conflict, resource limit, rate limit, cancellation, and storage failure. Do not expose raw configuration, process output, or provider errors as user-facing messages. No automatic retry scheduler is introduced.

## Agent identity decision

The generic `RunRequest` currently requires a nonempty `thesis_id`. The roadmap assigns separating conversation identity from thesis identity to milestone 3.

Recommended boundary: implement the production research `ToolExecutor` now and verify it through an `AgentRuntime` run associated with a real stored thesis. Tests may create that thesis explicitly as fixture setup. Do not fabricate a thesis ID or claim a general conversation host. Move the generic identity change forward only if the user wants milestone 1 to launch research turns without a thesis.

## Acceptance and verification

Use two real synthetic provider processes and temporary stores. Exercise manual commands and a deterministic `AgentRuntime` adapter through the same executor/application path; a live model call is optional and not the correctness oracle.

Verify explicit routing/provenance, duplicate IDs, unsupported versions, unavailable startup, deactivation after offering and while queued, restart generation changes, no fallback, forged scope, and cached reads with all processes stopped. Block one provider and prove another provider and offline reads can progress.

Verify cancellation before dispatch, during a provider response, and between pages; deadlines and shutdown; child reaping; preserved partial evidence and finalized ingestion runs; terminal races; queue/result/page limits; and event backpressure. Use barriers or controlled fixtures instead of timing-dependent sleeps.

Reopen the application store to verify stable references, repository mismatch rejection, immutable old selections after new ingestion, cross-workspace denial, duplicate view acceptance, incompatible view kinds, and distinct renderer failure. Reuse financial selection fixtures for revision/conflict semantics rather than duplicating financial algorithms in the application.

Run workspace/all-target Rust tests, strict Clippy, formatting, and existing provider Python tests after implementation. The CLI demonstrates explicit provider fetch, offline read, and accepted view requests through manual and agent paths, with visible scope/provenance and unavailable-provider outcomes.

## Review checkpoint

Review this architecture and the thesis-backed agent boundary before writing the executable implementation plan. The earlier research-contract proposal has obsolete milestone references: resolution/selection are already implemented, while passage extraction and background refresh remain later roadmap work.
