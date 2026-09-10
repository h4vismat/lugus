# Instrument Binding Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkboxes for tracking.

**Goal:** Agents create evidence-backed company/instrument bindings and fetch prices through them without routine user confirmation.
**Architecture:** Add optional instrument lookup to the financial provider protocol and immutable financial evidence storage. Application-owned pure matching and scoped append-only binding history connect this evidence to existing resolution datasets; existing workers, host and tools perform lookup/bind/bound-price effects.
**Tech Stack:** Existing Rust/Tokio/SQLite/serde/sha2 and Python yfinance 1.7.0 adapter. No new vendor SDK or service.
**Spec:** docs/superpowers/specs/2026-09-10-instrument-binding-design.md

## Global Constraints

- Work atop781a539 in the existing isolated worktree, on codex/instrument-binding. No merge, push, publication or live credential use.
- Binding creation is an ordinary agent capability, not a manual-only workflow.
- Explicit provider and listing selection; no first-result default or automatic provider failover.
- Agent/model fields cannot grant verification authority, replace scope, or import raw evidence.
- Price retrieval and an echoed requested symbol are not issuer identity evidence.
- Preserve exact financial evidence, workspace ownership, immutable history and original dataset meaning.
- Existing finite worker, cancellation, lifecycle, input/output and offline guarantees apply to new operations.
- No conversations, background refresh, passage extraction, desktop UI or universal security master.

## Concrete v1 matching policy

Pure assessment consumes an exact SEC Candidate, a Listing selected from that candidate, and saved InstrumentMetadata. Support equity only; other types conflict and missing type is incomplete. Compare ticker by ASCII case folding with trim, preserving punctuation. Map SEC NASDAQ to Nasdaq and NYSE to Nyse; Yahoo NMS/NGM/NCM/NASDAQ to Nasdaq and NYQ/NYSE to Nyse. Unknown namespaces/labels return incomplete, never inferred equivalence. Keep this explicit table versioned; further venues can be added with fixtures later.

Company names normalize ASCII case, whitespace and punctuation separators only; do not remove legal suffix words, use substrings, transliteration or fuzzy matching. Apple Inc. and APPLE INC agree. Shared issuer identifier namespaces must not contradict the SEC candidate identifier. An exact shared issuer identifier can establish issuer agreement; otherwise normalized full issuer name must equal candidate name or a retained source alias. Missing issuer evidence is incomplete; a present different issuer is conflict. Even matching issuer IDs still require matching listing ticker/exchange and equity type. Validation is source_supported under policy instrument-binding-v1, not universal or historical identity proof.

The initial Yahoo adapter supports exact source symbol lookup. Source-reported symbol is mandatory and must correspond to requested native yahoo:symbol; missing identity cannot be filled from request. SEC/Yahoo punctuation conversion is not an acceptance rule. Multiple SEC listings can yield multiple bindings; every bound price request names one binding ID.

## Task 1: Instrument lookup protocol, adapter and immutable evidence

**Files:** Create lugus-financial/src/instruments.rs, storage/instruments.rs, tests/instruments.rs, docs/protocol/instrument-lookup-v1.md; modify lib.rs, plugin/mod.rs, storage/mod.rs/migrations as required. Add lugus-financial/plugins/yfinance/instruments.py and tests/test_instruments.py; modify provider.py/plugin.json/README version consistently to0.2.0, retaining pinned library1.7.0.
**Consumes:** ProviderIdentity, CompanyId, InstrumentId, Validate, existing Plugin strict RPC lifecycle and bounded storage helpers.
**Produces:** types re-exported from instruments module:
```rust
pub struct InstrumentLookup { pub instrument: InstrumentId }
pub enum InstrumentKind { Equity, Other }
pub struct InstrumentMetadata {
    pub instrument: InstrumentId,
    pub issuer_name: Option<String>, pub ticker: Option<String>,
    pub exchange: Option<CompanyId>, pub kind: Option<InstrumentKind>,
    pub issuer_identifiers: Vec<CompanyId>,
    pub source_url: String, pub source_checksum: String,
    pub retrieved_at: chrono::DateTime<chrono::Utc>,
}
#[async_trait::async_trait]
pub trait InstrumentProvider: crate::capabilities::Provider + Send {
    async fn lookup_instrument(&mut self, query: &InstrumentLookup)
        -> crate::error::Result<InstrumentMetadata>;
}
pub struct InstrumentObservation {
    pub id: i64, pub provider: ProviderIdentity, pub request: InstrumentLookup,
    pub metadata: InstrumentMetadata, pub recorded_at: chrono::DateTime<chrono::Utc>,
}
// Repository extension trait + SQLite inherent bounded exact read; report exact paths/signatures.
// save_instrument_observation(provider,request,metadata) -> FinancialResult<InstrumentObservation>
// bounded_instrument_observation(provider,id,ReadLimits) -> ReadResult<InstrumentObservation>
```
- [x] Write behavioral RED tests: Plugin negotiates instrument_lookup:1; calls instrument_lookup.lookup; rejects nested unknown fields/mismatched source symbol; unsupported version never dispatches. Metadata optional missing fields remain explicit; malformed supplied fields fail. IDs bounded128bytes, name1024, URL4096, issuer IDs at most16, checksum64hex, no controls; finite serialized read preflight before decoding.
```rust
assert_eq!(plugin.lookup_instrument(&query).await.unwrap().ticker.as_deref(), Some("AAPL"));
assert!(repo.bounded_instrument_observation(&other_provider, id, limits).is_err());
```
- [x] Run cargo test -p lugus-financial --test instruments --offline and Python new test module; observe missing behavior.
- [x] Implement strict typed schema and Plugin adapter using existing request guard, capability gating, deadlines and safe protocol failures. Preserve old market_data:1 semantics.
```rust
// Validate query, gate capability version, await existing request path,
// validate returned source identity before returning metadata.
```
- [x] Add additive financial schema v5 immutable instrument observation rows. Every lookup retrieval has exact provider/request/payload/recorded_at; bound bytes/metadata under the same read snapshot before decode. No all-history materialization. Existing v4 databases migrate without rewriting evidence.
- [x] Implement Python pure source projection + injected metadata transport. Prime pinned public Ticker.history(period='5d',interval='1d',auto_adjust=False,back_adjust=False,repair=False,actions=False,timeout=10) with exceptions enabled, then project only symbol/longName/exchangeName/instrumentType from get_history_metadata via individual get calls; optional data stays None; do not manufacture CIK or share-class IDs. Hash bounded canonical source fields used for projection, record source URL and retrieval time. New method requires initialization, validates input before IO and returns safe typed failures; stdout stays protocol-only. Do not use get_info identity: pinned1.7.0 overwrites source symbol with request. Do not iterate lazy metadata keys or use private SDK methods. Test Apple metadata, missing symbol, wrong symbol, missing name/exchange/type, ETF, rate limit/timeout, malformed/oversized source and unchanged daily behavior.
- [x] Run focused financial/plugin and all yfinance tests, strict affected Clippy/fmt; commit feat(financial): add sourced instrument lookup evidence. Write full task1 report with actual signatures and schema/version effects.

## Task 2: Pure binding policy, worker lookup and durable binding store

**Files:** Create lugus-app/src/bindings.rs, bindings/policy.rs, store/bindings.rs, tests/bindings.rs; extend domain/catalog/agent_contract fetch schema, provider/worker/budget/recording, references/store/evidence/SQLite schema and fixtures. Modify only financial adapters required to expose Task1 repository methods.
**Consumes:** Task1 InstrumentLookup/Metadata/Observation/Provider and bounded repository reads; existing frozen resolution DatasetHeader/rows; Scope, Catalog, FetchResult and ApplicationStore.
**Produces:** pure assess_binding(candidate,listing,metadata)->BindingAssessment with Supported/Incomplete/Conflict and bounded reasons; immutable BindingRecord, BindRequest, RevokeBindingRequest and BindingStatus; ApplicationStore scoped create/read/list/revoke/prepare operations (exact names/signatures in report), optional binding IDs on FetchProvenance/FetchReference/DatasetHeader with serde defaults for old records.
- [x] Write RED table tests for concrete policy above: Apple accepted; wrong issuer/exchange/ticker/type conflicted; missing fields/unknown exchange incomplete; stable matching CIK and contradictory CIK; multiple listings require exact chosen listing membership; punctuation remains meaningful.
```rust
assert!(matches!(assess_binding(&apple, &nasdaq_listing, &aapl), BindingAssessment::Supported { .. }));
assert!(matches!(assess_binding(&apple, &nasdaq_listing, &wrong_issuer), BindingAssessment::Conflict { .. }));
```
- [x] Implement pure policy in isolated small module. No IO or agent judgement flag controls it.
- [x] Add Operation::InstrumentLookup and FetchCommand::InstrumentLookup{instance_id,query}; gate instrument_lookup:1 with immutable/live offerings and strict schema. Extend ManagedProvider/WorkerRepository for new trait, implementing required existing fixture adapters. Worker wraps metadata call in same Budget, persists only validated/within-budget successful observation, and records exact observation in FetchProvenance. One-shot failed lookup retains safe failed fetch receipt with no fabricated financial run. Add serde(default) optional instrument observation to persisted FetchReference. Test cancellation, unsupported capabilities, item/byte limits, source failure and exact retrieval ownership.
- [x] Add real-store RED tests using scoped resolution dataset+lookup fetch receipt: bind, reopen, wrong workspace/repository/provider/evidence rejected, identity_conflict/incomplete resolution rejected, explicit selected Candidate allowed when source outcome is Candidates, duplicate identical request returns same ID, changed request conflicts, concurrent supersession/revocation and old records immutable.
- [x] Implement application-owned immutable binding records referencing exact source dataset/observation and lookup fetch/financial observation. BindRequest has company_dataset_id, company_observation_id, explicit Listing, instrument_fetch_id and optional supersedes binding ID. Copy evidence from trusted stored references; rerun pure assessment, reject incomplete/conflict before mutation. Do not accept provider metadata, actor or validation boolean from callers.
- [x] Add indexed append-only history and transactional request dedupe with schema migration from app v1. Record ID is revision ID; supersedes names exact old active binding of same workspace/repository/company, atomically marks old binding superseded through immutable history. Revoke appends history; repeated same request idempotent, conflicting input fails. Preserve original records and reads forever. Bounded list pagination uses SQL limits and UTF8 byte preflight, never all records then truncate.
- [x] Define preparation semantics: preparing a bound fetch validates active binding and returns immutable evidence snapshot; revocation forbids later preparations, while already prepared/inflight work may complete with its original binding. New provider identity/version needs explicit revalidation; process generation is checked at live dispatch, not persisted as a permanent binding requirement. Binding read/revoke are offline.
- [x] Extend trusted record_fetch to accept optional binding ID on provenance, validate exact stored binding/workspace/provider/native price query association; attach binding ID to frozen datasets. A revoked/superseded immutable record can still describe work prepared earlier. Worker never invents a binding ID; Task3 host supplies it from scoped preparation. Old unbound records remain readable.
- [x] Run focused app tests, financial migration checks, strict Clippy/fmt; commit feat(app): persist validated instrument bindings. Report exact APIs including all defaults/migrations for Task3.

## Task 3: Shared host/tools and automatic agent binding acceptance

**Files:** Add lugus-app/src/application/bindings.rs and agent binding helpers as needed; modify application.rs, agent.rs, CLI example/support/synthetic setup, README and tests/agent_binding.rs, tests/cli.rs. Update milestone documentation on completion.
**Consumes:** Task2 scoped store APIs, optional fetch/dataset binding provenance and instrument lookup command.
**Produces:** shared async host operations and strict tools for create/read/list/revoke bindings and fetch bound prices; executable agent acceptance.
- [x] Write RED host/ToolExecutor tests for automatic binding using stored evidence; cross-scope/forged validation arguments, unoffered lookup/provider, revoked binding and stale provider version; successful same-version process restart; cancellation/finalization and old bound dataset after supersession.
```rust
// Agent tools: resolve/read/create resolution dataset, lookup instrument,
// create_binding from returned IDs, fetch_bound_prices, read/create price dataset.
// Assert source_supported policy and exact binding ID survives offline reopening.
```
- [x] Bridge every store effect through existing spawn_blocking boundary, validate inputs before clone and outputs including full ToolResult escaping. Keep application.rs focused by extracting new binding methods into a child module. Preserve supervised lifecycle; new operations must not bypass existing admission or captured-turn offering checks.
- [x] Bound-price request contains binding ID + dates/page size only, never a replacement provider or instrument. Prepare binding snapshot, create existing Prices command, authorize captured offering/current provider identity and dispatch through existing supervised job path. Attach binding ID from preparation in host-owned terminal provenance before trusted persistence. Record failed/cancelled bound fetches honestly. Admission after preparation may reject unavailable/full queue with no fake success.
- [x] Add agent tools for create/read/list/revoke/fetch bound prices and instrument lookup. Binding creation requires no human confirmation and uses host-attached run/request provenance. Strict schemas/runtime decoding reject unknown nested fields, verification flags, actor/scope replacement and raw source payloads. Manual clients use identical operations. Existing direct native-price fetch remains supported.
- [x] Extend real synthetic provider to emit matching SEC Apple listing and source-reported AAPL/Nasdaq equity metadata; add adversarial wrong issuer/exchange/missing/multiple listing fixtures. Update deterministic AgentRuntime fixture to load real thesis and chain actual returned references through lookup->binding->bound prices->dataset->view. No hand-seeded binding or user-declared shortcut. Runtime limits count the actual expanded tool chain.
- [x] Verify standalone CLI: resolve Apple, obtain listing evidence, agent creates binding, fetch AAPL prices, show bound dataset/view provenance, shutdown/remove plugin manifest, reopen binding/history/old dataset offline. Demonstrate incompatible evidence refusal and no first-listing selection. Document command files, provider selection and current policy limits.
- [x] Run workspace/all-target --offline --locked Rust tests, strict workspace/all-target Clippy, fmt, SEC/yfinance Python tests and actual CLI. Root may own final full-suite execution to avoid duplicates. Commit feat(app): enable automatic agent instrument binding.

## Final review and completion

- [x] Whole branch review against781a539 for source evidence, pure policy, immutable binding history, migration/bounds, provenance and existing lifecycle guarantees; fix confirmed findings with regressions and scoped re-review.
- [x] Record actual verification counts and update roadmap pointer to durable conversations/workspaces. Preserve main pending files and local branch/worktree; no merge/push.


## Completion record — 2026-09-10

Implemented on local branch `codex/instrument-binding`, based on `781a539`, through `20d5fb8`. Financial lookup: `ffe8191`; application binding policy/store: `cd72ca6`; status-capacity fix: `67b7043`; host/tools/actual agent CLI: `20d5fb8`. Task reviews and whole-branch review completed, with no Critical or Important issues remaining. No merge or push performed.

Independent final verification: `cargo test --workspace --all-targets --offline --locked` passed 278 tests (45 suites, 0 failed/ignored); workspace/all-target/all-feature strict Clippy, formatting and whitespace checks passed. 31 SEC and 23 pinned-yfinance Python tests passed, including actual installed-library source metadata characterization. Standalone locked/offline CLI build and fresh real-thesis AgentRuntime workflow passed: automatic Apple/AAPL binding, four adversarial refusals, exact price/dataset/view provenance, supersession from returned source references, old-binding fetch rejection, manifest-removed offline reads with original provider configuration, and offline revocation. No live provider or model request was exercised.

Review correction: binding admission initially bounded only the record, so a larger status envelope could strand it. The fix reserves the largest permitted serialized future status before mutation and preflights transition results, with exact-boundary and escaped-ID regressions.

Implementation adjustments: pinned yfinance get_info overwrites the source symbol, so lookup uses bounded public history plus selected source chart metadata. The price projection tool schema now advertises the decoder's existing `close`/`adjusted_close` enum spelling. Both have focused regressions.

Nonblocking follow-up from final review: strengthen `lugus-app/tests/cli.rs` adversarial cases beyond nonzero exit by preserving/asserting a safe refusal kind and checking unchanged binding IDs after each attempt. Current rejection behavior is covered by policy/store/host tests and the independent standalone check (four refusals produced no extra bindings). No incorrect runtime behavior was found.

Next milestone: durable conversations and research workspaces. Read `docs/superpowers/backend-milestones.md`; retain this branch/worktree until integration is explicitly requested.
