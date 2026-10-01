# Lugus research harness roadmap

Lugus owns financial evidence, calculations, investor knowledge, and research workflows. Models and agent runtimes are replaceable interpreters of those records. This roadmap preserves the approved direction and the remaining work so a new agent can continue without the original conversation or local planning files.

The investor selected two initial workflows: company comparison and investment-thesis review. Theses remain investor-authored free-form text. Agents propose claims; investors decide what becomes tracked knowledge. Initial evidence is reported financials and saved SEC filing passages.

## Current delivery status

As of 2026-09-30, the comparison foundation is implemented on `feat/company-comparison`, through commit `5513a20`. Check whether that branch has merged before starting follow-up work; a checkout without these changes is missing the foundation. Do not reimplement completed comparison work.

Completed capabilities:

- Two-company comparisons work without an agent runtime. Both identities are resolved from source evidence before financial retrieval.
- Annual US-GAAP/USD selection supports 1–5 actual fiscal periods, defaulting to three, with explicit revenue definitions and missing/conflicting data.
- Revenue growth and net margin use exact arithmetic, saved formula identities, input references, and independently rounded numeric/display values.
- Application schema 9 stores immutable comparison versions and research packages, with scoped headers and paged rows, sources, and package entries.
- Refresh retrieves evidence and publishes a new version. Opening saved research performs local reads and works offline.
- Jobs have stable retry identities, process leases, cancellation, shutdown cleanup, and orphan recovery. The UI preserves active jobs across panel navigation and retries transient status failures.
- The desktop exposes comparison creation, saved history, and source/formula inspection from the company workbench.

The existing workbench already saves theses, findings, brief revisions, and conversation-based review history. Separately, `lugus-agent::reviews` already implements durable structured review machinery. The remaining work must reconcile these paths, not introduce a third independent thesis store.

## Start here

1. Read this document and the [application research preparation design](application-research-preparation.md).
2. Confirm the comparison foundation is in the checkout. Inspect current code and migrations rather than assuming commit names or test counts still describe the latest tree.
3. Start with **Application ownership and tracked claims** below. Comparison-history polish is an independent small task.
4. Write a focused design and implementation plan for the selected milestone. Resolve its listed design decisions before coding; this roadmap does not freeze future Rust signatures or database schemas.
5. Implement and verify one milestone at a time. Record completed work, remaining gaps, and new decisions here so the next agent can resume.

Earlier brainstorming/specification files under `docs/superpowers/` are deliberately ignored by Git. This tracked document carries their approved product constraints; a fresh checkout must not require those local files.

## Code map

| Area | Current implementation | Follow-up responsibility |
| --- | --- | --- |
| Financial rules | `lugus-financial/src/comparison/`, `lugus-financial/src/selection.rs` | Authoritative selection, exact calculations, formula policies, explicit gaps |
| Comparison records and storage | `lugus-app/src/comparison/`, `lugus-app/src/store/comparison/` | Immutable packages, scoped dependencies, lifecycle persistence |
| Comparison execution | `lugus-app/src/application/comparison/` | Evidence preparation, admission, process ownership, cancellation |
| Research preparation | `lugus-app/src/research/` | Existing intent interpretation and source-backed preparation |
| Filing evidence | `lugus-app/src/passages/`, `lugus-app/src/application/passages.rs`, `lugus-app/src/store/passages/`, `lugus-app/src/agent/passages.rs` | Canonical text, exact coordinates, saved representations and passage tools |
| Company workbench storage | `lugus-desktop/src-tauri/src/companies.rs` | Move business operations behind the application boundary while preserving existing records |
| Existing structured reviews | `lugus-agent/src/reviews/` | Reconcile domain/coordinator/store contracts with application-owned workflows |
| Runtime abstraction | `lugus-agent/src/runtime.rs` | Reuse `AgentRuntime` and `ToolExecutor`; keep adapter-specific execution here |
| Desktop | `lugus-desktop/src/companies/workbench.tsx`, `lugus-desktop/src/comparison/`, `lugus-desktop/src-tauri/src/comparison.rs` | Render domain records, collect explicit investor decisions, bridge typed operations |

## Rules every milestone must preserve

- **Ownership:** `lugus-financial` owns financial rules, `lugus-app` owns research workflows and investor records, `lugus-agent` owns runtime adapters, and the desktop renders/submits decisions.
- **Investor control:** agent output never directly overwrites the thesis, accepted claims, or accepted findings. Manual claim creation and editing remain usable without a model.
- **Immutable history:** freeze exact investor revisions, evidence versions, comparison versions, policies, and baseline assessments. Later changes create new records.
- **Evidence integrity:** references must belong to the correct workspace/repository and selected package. A URL alone is not captured evidence. Preserve dependencies for the lifetime of saved research.
- **Fact separation:** reported facts, deterministic calculations, model assessments, and investor decisions are distinct. Prior model prose is not new independent source evidence.
- **Explicit retrieval:** opening saved records does not fetch. Refreshing evidence and reanalyzing an existing package are separate actions.
- **Limits and lifecycle:** preserve configured bounded and unlimited-research profiles. Use bounded paging, blocking boundaries for synchronous storage, stable retry IDs, and owned supervisors. No database transaction spans network work. Response-size limits must not prevent terminal persistence or orphan recovery.
- **Runtime portability:** declare required capabilities, reject unsupported runtimes before analysis, and never silently substitute another runtime/model/provider.
- **Honest results:** missing evidence and unknown model/usage metadata remain explicit. A schema-valid assessment is not proof that its narrative or citations are factually correct.

## Comparison history polish

Status: deferred minor from the completed comparison review. Independent of thesis work.

The saved selector currently numbers all records in a workspace as “Version N.” Different company pairs and refresh branches therefore look like one version sequence. Show company-pair and policy context, and derive refresh relationships from `previous_id`.

Start in `lugus-desktop/src/comparison/panel.tsx`, `types.ts`, and `controller.test.ts`. Existing headers already contain companies, request policy, and predecessor IDs; avoid a storage migration unless inspection establishes a real need.

Acceptance: create two unrelated comparisons and two refreshes branching from an earlier comparison. Labels distinguish the roots and refresh relationships, selecting any entry preserves its exact saved evidence, and opening the selector performs no retrieval.

## Application ownership and tracked claims

Status: next recommended milestone, the first part of delivery slice 2.

Move existing company/thesis operations behind application-owned interfaces. Preserve company IDs, conversation links, thesis revisions, findings, frozen briefs, and review history. Keeping the existing company database location is acceptable; physical database consolidation is not required.

Introduce tracked claims with stable identities and immutable revisions. A revision records investor-approved wording, origin, evaluation criteria, thesis association, and active/retired status. An investor can add and edit claims directly. A claim may cite a specific immutable comparison version.

Thesis edits require explicit claim reconciliation: retain, revise, or retire. While reconciliation is outstanding, show which thesis revision the active claims describe. Do not silently delete claims or treat an old claim set as describing new thesis text.

Decisions to settle in the focused design:

- The application store interface and compatibility adapter for the existing workbench database.
- Stable claim IDs, expected-revision mutation contracts, reconciliation records, and the minimum criteria representation.
- Namespaced mappings between legacy workbench and structured-review records. Never merge records because text or IDs resemble one another.

Acceptance:

- Existing company data survives migration/reopening, including conversation links and historical frozen briefs.
- Migration is transactional within each affected database and restartable across any imports. Do not retire legacy writes before preservation fixtures pass.
- Concurrent edits and stale expected revisions conflict explicitly without losing either history.
- Manual claims work without a runtime, and thesis edits expose reconciliation state.
- Retiring or revising a claim leaves historical reviews and comparison references readable offline.

## Claim proposals and investor acceptance

Status: follows application-owned thesis and claim records.

Add an explicit claim-extraction action over one frozen free-form thesis revision. Agents propose individual claims linked to exact passages of that revision. Proposed criteria may include a metric, peer, time period, or qualitative question. Unspecified criteria stay unresolved; the model must not silently establish the investor's intended meaning.

Investors accept, edit, or dismiss proposals. Only acceptance creates a tracked claim. Accepting a proposal against a superseded thesis revision must conflict and require reconciliation. Use the same application operations for direct UI actions and agent-assisted proposals.

Acceptance: proposals identify their exact source revision and text; fabricated text spans fail validation; accepted wording reflects investor edits; dismissed proposals create no claims; retries do not duplicate claims; stale acceptance and simultaneous thesis edits are covered by tests.

## SEC filing evidence packages

Status: another part of slice 2; can follow application ownership alongside proposal work.

Build on the existing document and passage operations. Prepare primary HTML annual and quarterly SEC filings and amendments when supported by configured capabilities. Preserve original HTML, immutable canonical text, extraction identity, and exact passage coordinates/checksums. Unsupported formats or extraction failures become explicit gaps.

Provide bounded local search over selected saved representations if the current API lacks it. Agents search/read captured documents and propose exact passages through existing coordinate validation. Freeze document versions and retrieval policy before assessment execution; every cited passage must remain tied to that run's package.

The UI must distinguish documents captured, documents successfully extracted, and passages actually examined. Search success does not establish complete coverage, and a management statement establishes what management said rather than proving the assertion true.

Acceptance: exact passages reopen offline; revised filings do not alter old citations; forged coordinates, cross-workspace references, and references outside the package fail; partial extraction and cancellation preserve completed preparation; searching a saved representation performs no source retrieval.

## Structured thesis reviews

Status: completes slice 2 after accepted claims and filing-package preparation.

Reconcile the existing `lugus-agent::reviews` domain/coordinator with application-owned workflows. Keep compatibility adapters where necessary. Preserve legacy conversation reviews as labeled prose; never automatically convert old prose or accepted findings into accepted claims or validated assessments.

A review request freezes the thesis revision, accepted claim revisions, research scope, explicitly selected prior assessment, and either a saved package or a request for fresh preparation. Baseline selection must not change when another review finishes concurrently.

Every captured claim receives an assessment entry containing:

- A disposition: supported, challenged, mixed, or insufficient evidence.
- Supporting and opposing references, with a written explanation.
- Missing evidence and unresolved criteria.
- Material changes from the selected prior review.
- Follow-up work and optional proposed thesis edits.

Validate output structure, exact claim coverage, revision ownership, evidence membership, and calculation references. Render numeric results from saved calculations rather than trusting model-provided copies. Permit bounded correction attempts under existing run limits; a runtime that finishes without an accepted submission leaves an incomplete analysis, not a successful review.

Record changes in source evidence, investor text/criteria, calculation policy, and model interpretation separately. If several changed, show each category instead of attributing the changed conclusion to one cause.

Acceptance:

- Every captured claim has exactly one entry, including unevaluable claims; omitted, duplicate, and foreign claims fail validation.
- Supporting and opposing evidence remains inspectable after restart and provider removal.
- Reviewing an older thesis revision creates a visibly historical assessment and cannot mutate current claims or apply text edits.
- Applying a proposed change is an explicit investor action against the current expected revision.
- Concurrent reviews retain their own frozen baseline and inputs.
- Malformed output, cancellation, runtime failure, and exhausted correction attempts preserve prepared evidence and earlier assessments.

## Runtime reanalysis and evaluation

Status: delivery slice 3. It builds on the existing comparison packages and the assessment contract completed in slice 2.

Expose reanalysis with a selected supported runtime, initially exercising the existing Codex and Claude adapters. Reanalysis uses the exact saved package and investor revisions and creates another assessment; it does not refresh evidence. Route chat-based comparison/review interpretation through the same application workflow contracts instead of maintaining parallel research rules.

Freeze runtime identity, requested model, reported model when available, workflow/instruction versions, timestamps, and available usage. Unknown metadata stays unknown. Runtime replacement must preserve packages, financial tables, accepted claims, and prior assessments. Do not promise identical prose across runs.

Add a small benchmark with manually checked financial examples and qualitative review criteria. Check calculation/reference correctness separately from citation relevance, handling of opposing evidence, uncertainty, and useful follow-up suggestions. Report deterministic fixture results separately from live model quality; live evaluation requires configured adapters and explicit runs.

Acceptance: run two supported adapters against byte-identical package contents; confirm no evidence fetch occurs and prior records remain unchanged; reject missing runtime capabilities before analysis; retain separate assessments and metadata; demonstrate a benchmark failure for a fabricated citation or unsupported calculation reference.

## Later extensions

These are approved directions for later investigation, not dependencies of the initial two workflows. Each needs a separate scope/design before implementation.

| Extension | Design work required |
| --- | --- |
| Broader web capture | Source acquisition, immutable snapshots, trust and citation rules |
| Recurring monitoring | Scheduling, explicit refresh policy, deduplication, notification and failure behavior |
| Public as of historical research | Publication-time selection and revision-aware evidence; current disclosures cannot answer this by themselves |
| Quarterly derivation | Period reconciliation and reviewed derivation policies |
| General statement assembly | Concept mappings, statement completeness, dimensions, and reconciliation |
| Currency conversion | Exchange-rate evidence, conversion dates, and unit/rounding policies |
| More than two companies | Selection limits, comparison presentation, and comparability rules |
| Valuation scenarios | Investor-owned assumptions, versioned formulas, and sensitivity analysis |
| External agent service transports and additional adapters | Authentication, capability contracts, scoped tools, cancellation, and structured submission |

## Verification and local execution

For the comparison foundation through `5513a20`, verification passed with 548 workspace Rust tests and 37 native desktop tests, frontend tests/build, strict Clippy, formatting, and browser acceptance. Six important independent-review findings were fixed with regression tests. These are historical results; rerun appropriate checks for new changes.

Root workspace:

```sh
cargo test --workspace --all-targets -- --test-threads=1
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all -- --check
git diff --check
```

The desktop is a separate Cargo workspace. From `lugus-desktop`:

```sh
npm ci
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --tests -- --test-threads=1
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo fmt --manifest-path src-tauri/Cargo.toml -- --check
cargo build --manifest-path src-tauri/Cargo.toml --example comparison_qa
node tests/comparison-browser.cjs
```

Browser QA needs Playwright and Chromium; `LUGUS_PLAYWRIGHT_MODULE`, `LUGUS_CHROMIUM`, and `LUGUS_COMPARISON_QA_BINARY` override their locations. See the [desktop test guide](../lugus-desktop/tests/README.md). The comparison harness uses disposable databases, synthetic provider evidence, and no model credentials. It proves plumbing and persistence, not live SEC coverage or model quality.

Agent integration fixtures require local loopback listeners. Sandboxes that deny binding will fail those tests. Serial execution avoided intermittent `ETXTBSY` errors from executable fixtures. An existing 20-second unlimited-research test timed out during competing builds and passed without that contention; keep environmental failures distinct from application regressions and do not weaken assertions merely to make a run green.

## Handoff checklist

- State the milestone being implemented and its prerequisites; do not treat the entire roadmap as one refactor.
- Preserve the ownership, evidence, revision, and migration rules above.
- Add regression tests for the milestone's acceptance cases before changing behavior.
- Verify existing data preservation and failure/cancellation behavior, not only successful model responses.
- Update this document with the delivered capability, integration status, remaining limitations, and the next concrete milestone.
