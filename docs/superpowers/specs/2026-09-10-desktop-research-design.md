# Lugus desktop research workspace

Status: product scope agreed in conversation on 2026-09-10; background refresh subsequently deferred on the same date. Backend milestones 1–4 are complete as recorded in the roadmap. This document records product scope and proposes boundaries; it is not yet an implementation specification for the desktop runtime.

## Product and first release

Lugus is a desktop research application combining conversations with persistent, interactive evidence views. A workspace belongs to a conversation and may cover multiple companies or subjects. Users can begin by asking a question or opening data directly.

The first release completes a focused company-research workflow: find a company, inspect prices and reported fundamentals, read filings, write a thesis, request a review, and revisit the assessment with its original evidence.

Tauri with a TypeScript frontend is the user's preferred direction. Desktop framework validation and frontend library selection belong to the technical design; no framework or component library has been installed or selected by this document.

Platform decision: validate feasibility on macOS first; other operating systems follow after that validation. The [isolated desktop-host probe](../spikes/2026-09-10-desktop-host/README.md) uses Tauri and plain TypeScript for that purpose. It does not select a production component library.

## Agreed screens

### New research

The sidebar exposes new research, recent conversations, pinned work, and settings. The main area offers company search and a message composer. Opening data directly creates a workspace that can receive messages later; a first message is not required to use research views.

### Research workspace

The left sidebar remains available. The conversation pane displays messages and agent activity, with its composer at the bottom. A resizable research area appears alongside it when a view is opened.

Research views use tabs and persist with the conversation. Reopening a conversation restores its views. Users can open views through search or Add view; agents can open the same kinds of views while answering. Useful views can remain open beyond the message that introduced them.

Agent-initiated view changes must preserve the user's current reading context. Opening another view must not replace existing content or unexpectedly switch the selected tab. The empty research area can reveal its first view.

### Settings

Provide essential application preferences and agent runtime configuration. Plugin activation/configuration continues to use the existing external setup for this release. Do not add a plugin management screen or a disguised equivalent inside Settings.

## Initial research views

| View | Initial contents | Required semantics |
| --- | --- | --- |
| Company overview | Company identity, latest available closing price, basic historical chart, selected fundamentals, links to filings | Identify the instrument and provider; label the trading date, currency, and retrieval time. Daily history must not be labelled a live quote. |
| Fundamentals | Selected reported metrics across periods and links to source evidence | Preserve units, reporting periods, filing dates, source concepts, and revisions. Missing values remain missing. Do not imply complete financial statements. |
| Filings | Filing list, document reader, and Ask about this for selected passages | Keep document identity and source references with a selection. Preserve the reading position during research actions. |
| Thesis and reviews | Thesis editor, explicit Run review action, assessment history, and original evidence | Separate editable thesis revisions, saved assessments, and newly fetched data. Show persisted run outcomes and recovery states. |

Advanced charting, cross-company comparison views, customizable dashboards, live/intraday prices, and plugin management are outside this first release. Conversations may still discuss multiple companies and open their individual views.

## Data access and stable workspaces

Agents may fetch fresh data through activated capabilities while answering, without routine confirmation. Show fetching activity and provide a stop action. Users may fetch data through direct research actions and start a new workspace for fresh research.

Background refresh is deferred. Creating, opening, switching or restoring a workspace does not schedule retrieval or launch a model turn. A new workspace contains fresh data only after a successful explicit research fetch; creation alone provides no freshness guarantee.

Research actions preserve existing chart range, table sorting/selection, document position, and selected tabs. Each view exposes its retrieval time and the effective date of the data. Failed fetches retain available evidence and show the failure. Empty, unavailable, partial, and previously retrieved data must be distinguishable.

Reviews are explicitly requested and retain frozen evidence. Agent runs and data fetch jobs have separate lifecycles; changing the active workspace is not implicitly an instruction to cancel an explicitly requested review. Late results remain associated with their originating workspace.

A dedicated Fetch latest data action within an existing workspace may be considered later. Scheduling policies, automatic retries and periodic freshness checks are outside the initial desktop milestone.

## Extensibility and proposed architecture

The shell owns conversations, navigation, view placement, persistence, and activity presentation. Its structure must not require a particular financial provider.

Distinguish three states: an activated provider capability, data already stored locally, and tools/evidence exposed to an agent turn. They are not interchangeable. Stored data should remain inspectable when a provider is unavailable; fresh retrieval requires an available capability.

The application layer coordinates financial repositories and ingestion functions, the agent runtime, durable reviews and workspace storage. User actions and agent requests should reach the same validated application operations. The desktop host must reuse the completed backend contracts.

Keep desktop transport at the boundary. Financial domain code, workspace transitions, and review coordination should not depend on desktop-framework or model-vendor protocol types. Favor pure functions for state transitions, capability selection, freshness decisions, and view-data projection; isolate storage, network access, clocks, and process lifecycle behind explicit interfaces.

Use structured, validated view requests rather than interpreting arbitrary assistant prose as UI commands. Whether plugins provide renderable schemas, map to application-owned renderers, or eventually provide their own UI is an open question for the capability spike. The first financial views do not establish support for arbitrary future plugin output.

## Original prerequisite analysis (historical)

The following analysis predates backend milestones 1–4. Consult the roadmap and current crate READMEs for completed capabilities; this section is not a list of desktop work still to implement.

The repository currently contains `lugus-agent` and `lugus-financial`, with no desktop application crate or frontend package.

- `AgentRuntime` accepts per-run `ToolSpec` values and a `ToolExecutor`, and emits text/tool lifecycle events. These are useful seams, but do not prove automatic capability discovery or view rendering. `RunRequest` currently requires a nonempty `thesis_id`; general conversation identity requires an explicit design rather than fabricated thesis IDs.
- Financial providers expose independent filings, fundamentals, and daily market-data capabilities through trusted external processes. Local repository reads do not trigger networking. Background refresh must explicitly coordinate ingestion, then read stored results.
- Provider market symbols and company filing identifiers are separate. Ticker-to-company resolution is missing. A query for IBM must not join financial records by a guessed symbol/name match.
- Financial snapshots preserve observations and revisions. Rules for selecting the values shown in an overview or chart need deterministic, source-aware projections; storage does not supply one reconciled current statement.
- Daily market history exists; live quotes, full statement assembly, and refresh scheduling do not.
- Durable reviews capture selected facts, filing metadata, and ingestion scope. They do not currently capture market-price evidence or document passages. The initial review UI must expose that evidence boundary; asking about a filing passage in chat does not automatically make it durable review evidence.
- General conversation history, context reconstruction across disposable agent turns, view persistence, and the connection between financial tools and research UI still need application-layer implementation.

## Prerequisite spike: activated capabilities to interactive research

Question: can the runtime expose the currently activated plugin capabilities to an agent and translate their results into useful, interactive research views without coupling the shell to a provider?

The spike should use existing interfaces and minimal disposable fixtures. Its output is a findings document and recommended contracts, not production UI or a general plugin framework.

Required probes:

1. Discover capabilities from configured provider instances and construct a bounded per-turn tool set. Exercise different capability combinations and tool-name collisions.
2. Route a tool call through an application executor to a financial capability, retaining provider identity and data provenance.
3. Produce a validated request to open a chart, table, or document through the same operation available to a user action. Determine how view context is made available to the agent.
4. Exercise unavailable providers, failed fetches, unsupported result shapes, invalid view requests, and partial capability coverage. Confirm cached data remains readable.
5. Evaluate application-owned renderers versus extensible rendering contracts. Identify what is feasible now and what requires further capability development.

Success means an end-to-end demonstrable path for supported results and explicit behavior for unsupported results. A tool list alone is insufficient evidence. Do not claim arbitrary plugin compatibility from a financial-only demonstration.

## Company identity prerequisite

Design an explicit mapping between companies and provider-specific instruments/filing identifiers. Preserve mapping source and allow multiple instruments per company. Search must surface ambiguity instead of guessing. Provider lookup sources, identifier lifecycle, and the initial resolution implementation require a separate technical decision.

The release's acceptance scenario is a company search that selects the intended entity and consistently binds the overview, fundamentals, and filings to that entity, while retaining their distinct provider identifiers.

## Delivery sequence

The user subsequently prioritized backend milestones before functional UI implementation. The [backend roadmap](../backend-milestones.md) supersedes the interleaved sequencing below; the screen scope remains unchanged. The [initial capability spike](../spikes/2026-09-10-capability-routing/README.md) records verified routing and its remaining limits.

1. Complete the capability spike and company-identity design; use the findings to define technical contracts.
2. Design the desktop host, conversation persistence, and state/event boundaries. Validate the preferred Tauri/TypeScript direction and choose UI libraries at this stage.
3. Implement the shell and a complete search-to-overview workflow, including direct view opening and agent-initiated view opening.
4. Add fundamentals and the filing reader with explicit selected context for questions.
5. Add the minimal thesis/review workflow using supported frozen evidence.
6. Complete restore/recovery behavior and integrated desktop verification before calling the initial release complete. Background refresh is deferred.

Each subsystem needs an implementation specification after its prerequisite decisions are resolved. This product scope is not a single executable plan for all six stages.

## Acceptance and verification requirements

- Start with either a message or a manually opened company view, and continue within the same workspace.
- Open multiple companies' views in one conversation; switching between them does not silently change the context of earlier messages or saved evidence.
- Restore conversations and view state after reopening the app, without reissuing completed model turns.
- Verify workspace creation, switching and restoration cause no automatic fetch or model turn; late results from explicit research remain associated with their origin and preserve user selections.
- Exercise partial data, missing capabilities, failed fetches, and offline reading without inventing values or silently mixing providers/revisions.
- Preserve exact financial decimal values in storage and transport. Any numeric conversion for plotting is a display projection; labels and calculations must use appropriate precision and retain source semantics.
- Verify chat context refers to the selected filing document and passage. Render external filing content without access to privileged desktop APIs.
- Run a review, fetch newer data in another workspace, and reopen the assessment to verify its original evidence remains unchanged. Existing review cancellation and recovery guarantees must hold through the desktop host.
- Check keyboard navigation, focus behavior, pane resizing, readable chart/table alternatives, and loading/error states in the running desktop application.

## Repository references

- [Financial capabilities and data semantics](../../../lugus-financial/README.md)
- [Agent runtime and durable reviews](../../../lugus-agent/README.md)
- [Runtime request/event contract](../../../lugus-agent/src/runtime.rs)
- [Agent tool contract](../../../lugus-agent/src/tools.rs)
- [Financial provider traits](../../../lugus-financial/src/capabilities.rs)
