# Portfolio Dashboard Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved portfolio overview, holding detail panel, and a persisted daily comparison with the S&P 500 Total Return index.

**Architecture:** Extend financial evidence with a separate historical-price capability and immutable paged observations. Calculate valuations and returns in the pure portfolio crate, orchestrate revision-bound history jobs in the application, and render independent current-holdings and historical-performance sections in the desktop.

**Tech Stack:** Rust 2024, existing checked scale-18 Decimal, SQLite/rusqlite, Tokio, Python/yfinance 1.7.0, pandas_market_calendars 5.4.0, TypeScript/DOM/SVG, Tauri, Node test runner and Playwright.

**Spec:** `docs/superpowers/specs/2026-09-15-portfolio-dashboard-design.md` (approved by the user after commit `62d051c`).

## Execution outcome — September 15, 2026

Implemented inline at the user's request on `feat/portfolio-dashboard`, in `.worktrees/portfolio-dashboard`. The task-level record below supersedes the original prescriptive step checklists; those are retained as design history, not open work.

- [x] Tasks 1–4: historical contract, pinned provider, immutable evidence storage and managed-worker integration.
- [x] Tasks 5–7: shared replay cursor, exact daily returns/benchmark calculations, owned revision-bound persisted jobs.
- [x] Tasks 8–9: bounded native transport, exact dashboard metrics, current summary, allocation and sortable holdings.
- [x] Tasks 10–11: chart/ranges/data/source details, cached history controller, holding drawer and unsent research handoff.
- [x] Task 12: inline review, Rust/Python/frontend/native suites, deterministic and live browser checks, documentation.

Implementation choices: extended the existing `portfolio_qa` bridge and managed-worker Python fixture instead of adding duplicate QA hosts/providers. Split/fee arithmetic is tested in the pure engine; browser fixtures focus on interaction, account scope, missing prices, refresh and persistence. Used the existing plugin environment via `uv` because system Python lacked `ensurepip`. Followed the user's inline choice for implementation and review; no subagents were used. Final verification and precise limitations are recorded in [the verification report](../verification/2026-09-15-portfolio-dashboard.md).

## Global Constraints

- “Financial arithmetic uses checked scale-18 decimals.”
- “Keep that stack; use SVG for charts and existing exact decimal formatting for displayed financial values.”
- “Do not silently substitute the price-only S&P 500 or an ETF.”
- “Do not send the message automatically.”
- “Never draw through gaps, turn missing values into zero, or renormalize partially priced holdings to 100%.”
- “Bound jobs to one active history refresh per portfolio, at most four application-wide, two concurrent provider calls per job, a five-minute deadline, at most 100 instruments, and 100,000 source observations per job.”
- “Process historical evidence in pages of at most 200 rows and honor tighter configured output-byte limits.”
- “Never combine old calculated rows with a new ledger revision.”
- “Release acceptance requires a verified historical adapter and real end-to-end evidence-to-chart flow, not just a rendered dashboard with unavailable performance.”
- Preserve existing daily-price semantics, transaction validation/FIFO, account management, audit, and immutable chat snapshots. No production fixture data.

## Execution and review boundaries

Read the spec together with this plan. Create or verify an isolated worktree before implementation; copy the approved mockup into the worktree's ignored `.superpowers/` directory because it is currently untracked. Do not add runtime session keys/state files to Git. The docs directory is ignored in this repository, so stage only these explicit documentation files with `git add -f` when committing them.

This is one dependent feature, with twelve reviewable tasks. Tasks 1–4 establish data ingestion, tasks 5–6 establish calculation, tasks 7–8 connect persistence/native transport, tasks 9–11 implement presentation, and task 12 verifies the complete path. Tasks 5 and 9 can be developed independently of source ingestion once interfaces below are established; integration still follows the dependency order.

Use failing behavioral tests before behavior changes. Each task lists a focused red/green command and a commit boundary. Run broader tests only at integration boundaries or when a change affects shared behavior. The live check in task 2 is a release prerequisite; a provider outage is recorded as a blocked live check while deterministic implementation can continue.

## Interface conventions

- Rust financial evidence uses `lugus_financial::domain::Decimal`; portfolio/app calculations use `lugus_portfolio::Decimal`. Convert by checked parsing of plain decimal strings, never through floating point. Evidence exceeding scale-18/range produces an explicit incompatible-price issue.
- All transport revisions and observation/run identifiers are decimal strings. Internal SQLite IDs may remain i64 and ledger revisions u64; use the existing revision serializer for u64 and add a string-ID serializer at new historical transport boundaries.
- Percentage fields use **percentage units**: `"12.6"` means 12.6%, not 0.126. Growth factors use ratios: `"1.126"`.
- Every source manifest, calculation version, and persisted result is immutable. Old evidence and results may be read with their original metadata, never relabeled as current.

## Task 1: Versioned historical-price contract and plugin adapter

**Files**

- Create `lugus-financial/src/historical_prices.rs`, `lugus-financial/src/plugin/history.rs`, `lugus-financial/docs/protocol/historical-prices-v1.md`.
- Modify `lugus-financial/src/lib.rs`, `lugus-financial/src/capabilities.rs`, `lugus-financial/src/plugin/mod.rs`.
- Create `lugus-financial/tests/history_domain.rs`, `lugus-financial/tests/history_plugin.rs`, `lugus-financial/tests/fixtures/history_plugin.py`.

**Interfaces**

New capability: `historical_prices: 1`; RPC: `historical_prices.daily`. Existing `market_data.daily` and `PriceBar` remain unchanged. Define these public serde types in `historical_prices.rs`, using existing `InstrumentId`, financial `Decimal`, `Completeness`, and chrono date types:

```rust
pub struct HistoryQuery {
    pub instrument: InstrumentId,
    pub start: NaiveDate,
    pub end: NaiveDate,
    pub anchor: NaiveDate,
    pub cursor: Option<String>,
    pub page_size: usize,
}
pub struct SplitRatio { pub numerator: u64, pub denominator: u64 }
pub struct HistoryManifest {
    pub instrument: InstrumentId,
    pub requested_start: NaiveDate,
    pub requested_end: NaiveDate,
    pub coverage_start: NaiveDate,
    pub anchor: NaiveDate,
    pub last_completed_session: NaiveDate,
    pub currency: String,
    pub exchange_timezone: String,
    pub calendar: String,
    pub calendar_version: String,
    pub normalization_version: u32,
    pub source_basis: String,
    pub completeness: Completeness,
    pub retrieved_at: DateTime<Utc>,
}
pub struct HistoryDay {
    pub date: NaiveDate,
    pub market_close: Option<DateTime<Utc>>,
    pub source_close: Option<Decimal>,
    pub close: Option<Decimal>,
    pub factor_to_anchor: Decimal,
    pub split: Option<SplitRatio>,
    pub unsupported_action: Option<String>,
    pub source_url: String,
}
pub struct HistoryPage {
    pub manifest: HistoryManifest,
    pub items: Vec<HistoryDay>,
    pub next_cursor: Option<String>,
}
#[async_trait]
pub trait HistoricalPricesProvider: Provider + Send {
    async fn fetch_history(&mut self, query: &HistoryQuery) -> Result<HistoryPage>;
}
```

The provider emits one row for each calendar day from the previous session before `start` through `anchor`, even when `end < anchor`. `market_close: null` explicitly denotes a scheduled closure; an expected session with `close: null` denotes missing data. Prices on an incomplete current session are null, but its observed split action may be retained. This lets the consumer verify all subsequent split factors without unbounded metadata arrays. The source snapshot manifest is identical on every page. The host attaches provider identity and immutable observation IDs at ingestion.

- [ ] **1. Write a contract regression test before adding the module.**

```rust
use lugus_financial::{domain::Validate, historical_prices::HistoryQuery};
use serde_json::json;
#[test]
fn rejects_oversized_history_pages_and_reversed_anchor() {
    let mut value = json!({"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},
        "start":"2020-08-28","end":"2020-08-31","anchor":"2026-09-15",
        "cursor":null,"page_size":201});
    let query: HistoryQuery = serde_json::from_value(value.clone()).unwrap();
    assert!(query.validate().is_err());
    value["page_size"] = json!(200);
    value["anchor"] = json!("2020-08-27");
    let query: HistoryQuery = serde_json::from_value(value).unwrap();
    assert!(query.validate().is_err());
}
```

- [ ] **2. Run red:** `cargo test -p lugus-financial --test history_domain`. Expected: missing historical module/type.
- [ ] **3. Implement the types and validators.** Require nonempty bounded identifiers, `start <= end <= anchor`, page size 1–200, nonempty bounded cursors, matching manifest/query fields, positive split ratios and factors, nonnegative finite price strings, paired source/derived close nullability, exchange timezone `America/New_York`, supported calendar identities, and strictly ordered rows inside declared coverage. Reject prices on calendar closures, prices after a session's completion timestamp, and unsupported normalization versions. Use `#[serde(deny_unknown_fields)]` on request structs.
- [ ] **4. Add `HistoryPage::validate_for(&HistoryQuery) -> Result<()>` and the Plugin implementation in child module `plugin/history.rs`.** Reuse private `supports`, `call`, and `close` from the parent module. Protocol violations close the plugin; typed source errors retain normal existing lifecycle behavior.

```rust
self.supports("historical_prices")?;
query.validate()?;
let page: HistoryPage = self.call("historical_prices.daily", serde_json::to_value(query)?).await?;
if page.validate_for(query).is_err() {
    let _ = self.close().await;
    return Err(Error::new(ErrorKind::Protocol, "invalid historical-price page"));
}
Ok(page)
```

- [ ] **5. Add fixture modes** `ok`, `wrong_instrument`, `wrong_anchor`, `wrong_order`, `invalid_split`, `unsupported`, `rate_limited`, `blocked`. Assert pagination and unchanged manifest; invalid pages close the connection; unsupported capability does not invoke the RPC. Use the existing `market_plugin.rs` fixture lifecycle pattern, with the new RPC and types.
- [ ] **6. Run green and compatibility:** `cargo test -p lugus-financial --test history_domain --test history_plugin --test market_plugin`.
- [ ] **7. Commit:** `feat(financial): define versioned historical price evidence`.

## Task 2: Pinned yfinance normalization and exchange calendar

**Files**

- Create `lugus-financial/plugins/yfinance/history.py`, `lugus-financial/plugins/yfinance/history_calendar.py`, `lugus-financial/plugins/yfinance/tests/test_history.py`, `lugus-financial/plugins/yfinance/tests/test_history_calendar.py`, `lugus-financial/plugins/yfinance/live_history_check.py`.
- Modify `lugus-financial/plugins/yfinance/provider.py`, `lugus-financial/plugins/yfinance/common.py`, `lugus-financial/plugins/yfinance/plugin.json`, `lugus-financial/plugins/yfinance/requirements.txt`, `lugus-financial/plugins/yfinance/README.md` and `lugus-financial/plugins/yfinance/tests/test_provider.py`.
- Modify `lugus-app/src/portfolio/prices.rs` and `lugus-app/tests/portfolio_prices.rs` to accept the new provider version for the unchanged ordinary-close contract.

**Interfaces:** `normalize_history(rows, metadata, query, retrieved_at, calendar_rows) -> dict` returns an immutable `{manifest, items}` snapshot; `HistoricalProvider(fetch, clock, calendar, recovery).daily(params) -> dict` pages it. `session_days(calendar_name, start, anchor, retrieved_at) -> list[dict]` returns calendar dates with optional UTC market-close timestamps. Both provider classes share the existing source recovery object but maintain independent pagination snapshots.

Pin `pandas_market_calendars==5.4.0` alongside `yfinance==1.7.0`; the calendar library documents market-close schedules, holidays and early closes. [Package](https://pypi.org/project/pandas_market_calendars/5.4.0/), [calendar usage](https://pandas-market-calendars.readthedocs.io/en/latest/usage.html). Bump the bundled plugin to `0.3.0` in manifest and handshake; preserve `0.2.0` current-price compatibility with an explicit allowlist and regression test, rather than replacing every old fixture version.

- [ ] **1. Write the independent split-factor test.** Export `reverse_split_factor(day: date, splits: list[tuple[date, int, int]], anchor: date) -> decimal.Decimal` from `history.py`.

```python
import unittest
from datetime import date
from decimal import Decimal
from history import reverse_split_factor

class SplitFactorTests(unittest.TestCase):
    def test_split_after_chart_end_still_changes_old_share_basis(self):
        actions = [(date(2020, 8, 31), 4, 1)]
        factor = reverse_split_factor(date(2020, 8, 28), actions, date(2026, 9, 15))
        self.assertEqual(factor, Decimal(4))
        self.assertEqual(Decimal('124.8075') * factor, Decimal('499.2300'))
        self.assertEqual(reverse_split_factor(date(2020, 8, 31), actions,
            date(2026, 9, 15)), Decimal(1))
```

- [ ] **2. Run red:** from `lugus-financial/plugins/yfinance`, `PYTHONPATH=. python3 -m unittest discover -s tests -p 'test_history*.py'`. Expected: missing `history` import.
- [ ] **3. Implement factor conversion with rational accumulation.** Use Python `Fraction` for source ratios and cumulative factors; serialize normalized prices/factors once with scale-18 half-even rounding, retaining original price and split ratios. Check finite/nonnegative values and reject ratios exceeding u64 numerator/denominator. Do not use repeated rounded factors to derive closes. Same-date actions are excluded from the reverse factor. Set manifest `source_basis` to `yahoo_split_adjusted_close` for securities; the fixed benchmark uses `total_return_index`, a factor of one and its source index level unchanged. These are the two supported v1 basis identifiers and the host validates them with the instrument identity.

```python
from fractions import Fraction
from decimal import Decimal, localcontext

def reverse_split_factor(day, splits, anchor):
    ratio = Fraction(1, 1)
    for effective, numerator, denominator in splits:
        if day < effective <= anchor:
            ratio *= Fraction(numerator, denominator)
    with localcontext() as ctx:
        ctx.prec = 80
        return Decimal(ratio.numerator) / Decimal(ratio.denominator)
```

- [ ] **4. Implement explicit calendar selection.** Read source exchange metadata through a new historical fetch boundary, leaving the existing ordinary-price metadata path unchanged. Map verified Yahoo NASDAQ codes `NMS/NGM/NCM` to NASDAQ and `NYQ/PCX/ASE` to NYSE; the fixed `^SP500TR` benchmark uses the US equity session calendar explicitly. Reject other exchanges/timezones. Record calendar package/version in the manifest. Include scheduled closures and use actual UTC close times for early closes/DST. Calculate the preceding session via the calendar, not a guessed weekday/14-day window.

- [ ] **5. Fetch through the common anchor with `actions=True`, `auto_adjust=False`, `back_adjust=False`, `repair=False`, `rounding=False`, `keepna=True`, and the existing timeout/recovery bounds.** Preserve the Stock Splits column in a historical-only dataframe conversion. Distinguish a missing action column from an all-zero action column. Require one complete response through the anchor before emitting any page; never combine action factors from independent partial snapshots. Detect source capital-gain/spinoff/other explicit unsupported action fields and mark affected rows; do not claim to detect corporate actions the source does not report. Missing price cells become null only when identified as missing; malformed/nonfinite non-missing numbers fail validation.
- [ ] **6. Test normalization** using synthetic frames for a 4:1 split, 1:10 reverse split, multiple splits, an action after requested end, dividend-adjusted close differing from Close, missing action column, missing session bar, weekend, Thanksgiving/early close, wrong currency/calendar, duplicate dates, current unfinished session, expired cursor, changed query, snapshot/byte cap, and stdout isolation. Test `market_data.daily` still requests `actions=False` and returns the original schema.
- [ ] **7. Install pinned requirements and run green.** In the plugin directory create `.venv` with `python3 -m venv .venv` only if absent, then run `.venv/bin/python -m pip install -r requirements.txt` and `PYTHONPATH=. .venv/bin/python -m unittest discover -s tests`. At repository root run `cargo test -p lugus-app --test portfolio_prices`.
- [ ] **8. Add and run a controlled live checker.** `live_history_check.py --output /tmp/lugus-history-live.json` calls the actual new provider for AAPL around the August 2020 split and for a recent completed month of `^SP500TR`, with anchor set to the current New York date. Read every cursor, assert the 4:1 event/factor relationship and source-to-derived close relation, positive benchmark levels, matching USD/calendar metadata, and no fabricated prices across missing sessions. Save manifests, selected observations and pass/fail checks locally; do not commit source downloads or claim source completeness. A listing or a mocked provider is not this check.
- [ ] **9. Commit:** `feat(yfinance): normalize historical closes with split evidence`.

## Task 3: Persist and ingest immutable historical evidence

**Files**

- Create `lugus-financial/src/storage/history.rs`, `lugus-financial/src/storage/historical-v6.sql`, `lugus-financial/src/application/history.rs`, `lugus-financial/tests/history_storage.rs`, `lugus-financial/tests/history_ingestion.rs`.
- Modify `lugus-financial/src/storage/mod.rs` and `lugus-financial/src/application.rs` to register modules and financial schema 6.

**Interfaces**

```rust
pub struct HistoryRun {
    pub id: i64, pub provider: ProviderIdentity, pub query: HistoryQuery,
    pub status: RunStatus, pub manifest: Option<HistoryManifest>,
    pub row_count: usize, pub error: Option<String>,
}
pub struct HistoryObservation { pub id: i64, pub day: HistoryDay }
pub struct HistoryReadPage {
    pub run: HistoryRun, pub items: Vec<HistoryObservation>, pub next_offset: Option<usize>,
}
pub trait HistoryRepository {
    fn start_history_run(&mut self, p: &ProviderIdentity, q: &HistoryQuery) -> Result<i64>;
    fn save_history_page(&mut self, run: i64, page: &HistoryPage) -> Result<()>;
    fn finish_history_run(&mut self, run: i64, error: Option<&Error>) -> Result<()>;
    fn history_run(&self, p: &ProviderIdentity, run: i64, limits: ReadLimits) -> ReadResult<HistoryRun>;
    fn history_page(&self, p: &ProviderIdentity, run: i64, offset: usize, limits: ReadLimits)
        -> ReadResult<HistoryReadPage>;
}
pub async fn ingest_history<R: HistoryRepository + ?Sized, P: HistoricalPricesProvider + ?Sized>(
    repo: &mut R, provider: &mut P, query: &HistoryQuery,
) -> Result<i64>;
```

- [ ] **1. Write storage/ingestion fixture tests before implementation.** Use the task 1 plugin fixture, temporary SQLite, and a page size of one. Save a first page, attempt a repeated/nonadvancing page, finish the remaining pages, reopen, and compare exact observations. Add a malicious manifest-change case and ensure no partial snapshot is published as complete. Test provider/run mismatch and preflight byte budgets before payload decoding.

```rust
// Inside history.rs tests, after building a two-row fixture using HistoryDay:
assert!(repo.history_page(&wrong_provider, run, 0, ReadLimits {
    max_items: 1, max_bytes: 4096,
}).is_err());
let page = repo.history_page(&provider, run, 0, ReadLimits {
    max_items: 1, max_bytes: 4096,
}).unwrap();
assert_eq!(page.items.len(), 1);
assert_eq!(page.next_offset, Some(1));
```

- [ ] **2. Run red:** `cargo test -p lugus-financial --test history_storage --test history_ingestion`.
- [ ] **3. Add the additive financial migration:**

```sql
CREATE TABLE historical_runs (
 id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id),
 query TEXT NOT NULL, status TEXT NOT NULL, manifest TEXT, cursor TEXT,
 seen_cursors TEXT NOT NULL DEFAULT '[]', last_date TEXT, row_count INTEGER NOT NULL DEFAULT 0,
 started_at TEXT NOT NULL, finished_at TEXT, error TEXT
);
CREATE TABLE historical_observations (
 id INTEGER PRIMARY KEY, provider_id TEXT NOT NULL REFERENCES providers(id),
 fingerprint TEXT NOT NULL, payload TEXT NOT NULL, UNIQUE(provider_id,fingerprint)
);
CREATE TABLE historical_run_days (
 run_id INTEGER NOT NULL REFERENCES historical_runs(id), ordinal INTEGER NOT NULL,
 date TEXT NOT NULL, observation_id INTEGER NOT NULL REFERENCES historical_observations(id),
 PRIMARY KEY(run_id,ordinal), UNIQUE(run_id,date)
);
CREATE INDEX historical_days_by_date ON historical_run_days(run_id,date);
PRAGMA user_version=6;
```

- [ ] **4. Implement transactional page acceptance and completion.** Validate consecutive calendar dates including page boundaries, immutable manifest, cursor progress, source identity, observation count, and final row equal to anchor. An expected session may have a null price; a missing calendar row is a protocol error. Fingerprint source-normalized observation content and include normalization/calendar version in the run identity. Mark source coverage unverified as reported, independent from successful ingestion. Commit page/cursor together; finalization requires cursor exhaustion and declared coverage completion.
- [ ] **5. Implement exact-run bounded reads.** Preflight metadata and row-byte sizes within the read transaction, select by provider/run/ordinal, and return the next offset from actual rows returned. Never materialize unrelated runs. Feed the same validation through `ingest_history`, modeled on `application/market.rs` with history-specific terminal checks.
- [ ] **6. Run green:** `cargo test -p lugus-financial --test history_storage --test history_ingestion --test market_storage`.
- [ ] **7. Commit:** `feat(financial): persist paged historical evidence runs`.

## Task 4: Connect history to the existing managed worker

**Files**

- Modify `lugus-app/src/domain.rs`, `lugus-app/src/catalog.rs`, `lugus-app/src/provider.rs`, `lugus-app/src/worker.rs`, `lugus-app/src/worker/budget.rs`, `lugus-app/src/worker/recording.rs`, `lugus-app/src/store/evidence.rs`, `lugus-app/src/store/sqlite.rs`, `lugus-app/src/store.rs`.
- Create `lugus-app/src/application/history_evidence.rs`; register in `lugus-app/src/application.rs`.
- Create `lugus-app/tests/history_worker.rs`; modify `lugus-app/tests/fixtures/worker.py`, `lugus-app/tests/lifecycle.rs`, `lugus-app/tests/worker/startup_cleanup.rs` for the new supertrait on injected providers.

**Interfaces:** Add `Operation::HistoricalPrices`, `FetchCommand::HistoricalPrices { instance_id: String, query: HistoryQuery }`, and `RunKind::Historical`. Add `HistoricalPricesProvider` to `ManagedProvider`, `HistoryRepository` to `WorkerRepository`, and bounded history run/page methods to `EvidenceRepository`. Add application method:

```rust
pub async fn history_evidence_page(
    &self, scope: &Scope, fetch_id: &str, page: PageRequest,
) -> Result<HistoryReadPage>;
```

- [ ] **1. Write a worker integration test** that submits a `HistoricalPrices` command through `Harness::scope`, waits, reads its fetch receipt, and asserts `RunKind::Historical`. Read pages via `history_evidence_page`; a different workspace must fail. Fixture mode `history_ok` advertises the new capability and emits the task 1 contract. Mode `history_blocked` writes the existing barrier and waits for cancellation.

```rust
let job = h.app.submit_manual(&scope, FetchCommand::HistoricalPrices {
    instance_id: "p".into(),
    query: serde_json::from_value(serde_json::json!({
        "instrument":{"namespace":"yahoo:symbol","value":"TEST"},
        "start":"2026-01-02","end":"2026-01-05","anchor":"2026-01-05",
        "cursor":null,"page_size":2
    })).unwrap(),
}).unwrap();
let status = h.app.wait(&scope, &job.id).await.unwrap();
let fetch = h.app.read_fetch(&scope, status.fetch_id.as_ref().unwrap()).await.unwrap();
assert_eq!(fetch.runs[0].kind, RunKind::Historical);
```

- [ ] **2. Run red:** `cargo test -p lugus-app --test history_worker`.
- [ ] **3. Extend every exhaustive command/operation match**, including strict deserialization, instance selection, capability mapping, root-cursor rejection, worker dispatch and fetch-receipt run validation. Add `(Operation::HistoricalPrices, "historical_prices", 1)` to the catalog. Do not expose a new generic agent tool; historical portfolio fetches are native/application operations.
- [ ] **4. Implement the budget and recording adapters.** Count every history row and page against the existing worker limits and source bytes. Recording starts/captures/finalizes Historical runs even on failure. Cancelled RPCs invalidate/reap their process through existing guards. Worker dispatch calls `ingest_history`; no ad hoc Python subprocess or direct network code in the desktop/application.
- [ ] **5. Implement scoped history evidence reads.** First read the fetch through the application's existing scoped store; require HistoricalPrices command, one matching Historical run, compatible repository/provider, and successful ingestion before selecting rows. SQL remains inside blocking store operations. Use existing read limits and adapt financial bounded errors.
- [ ] **6. Run green and regressions:** `cargo test -p lugus-app --test history_worker --test lifecycle --test portfolio_prices`; `cargo test -p lugus-app --lib` for startup/budget/dispatch tests. Assert deadline, cancellation, changed cursor, page limits and source failure never become a successful result.
- [ ] **7. Commit:** `feat(app): ingest historical evidence through managed workers`.

## Task 5: Ordered replay and historical valuation

**Files**

- Create `lugus-portfolio/src/history.rs`, `lugus-portfolio/tests/history.rs`.
- Modify `lugus-portfolio/src/replay.rs`, `lib.rs`; retain `fifo.rs` as the single trade implementation.

**Interfaces:** Export `ReplayCursor<'a>::new(&'a Ledger) -> Result<Self>`, `advance_to(&mut self, day: Day) -> Result<&AccountState>`, and `advance_to_cancellable(&mut self, day: Day, cancelled: &dyn Fn() -> bool) -> Result<&AccountState>`. The ordinary advance delegates with `|| false`; history jobs use the cancellable form. Add these types in `history.rs` (all derive serde and equality):

```rust
pub struct HistoricalClose {
    pub instrument_id: String, pub date: Day,
    pub session_close: Option<chrono::DateTime<chrono::Utc>>,
    pub close: Option<Decimal>, pub split: Option<(u64, u64)>,
    pub observation_id: String, pub unsupported_action: Option<String>,
}
pub struct HistoryIssue {
    pub code: HistoryIssueCode, pub date: Day,
    pub instrument_id: Option<String>, pub account_id: Option<String>,
}
pub enum HistoryIssueCode {
    MissingPrice, MissingBaseline, UnknownCalendar, IncompatiblePrice,
    SplitMismatch, UnsupportedAction, MissingBenchmark, BenchmarkSelectionRequired, ZeroCapital,
    TotalLossRestart, BenchmarkOverdraft,
}
pub struct DailyValuation {
    pub date: Day, pub value: Option<Decimal>,
    pub deposits: Option<Decimal>, pub withdrawals: Decimal,
    pub opening_contribution: Option<Decimal>,
    pub benchmark_level: Option<Decimal>,
    pub observations: Vec<String>, pub issues: Vec<HistoryIssue>,
}
pub struct ValuationHistoryInput<'a> {
    pub ledgers: &'a [Ledger], pub closes: &'a [HistoricalClose],
    pub benchmark: &'a [HistoricalClose],
    pub baseline: Day, pub end: Day,
}
pub fn historical_values(input: &ValuationHistoryInput<'_>) -> Result<Vec<DailyValuation>>;
pub fn historical_values_cancellable(input: &ValuationHistoryInput<'_>,
    cancelled: &dyn Fn() -> bool) -> Result<Vec<DailyValuation>>;
```

`deposits` contains recorded deposits only. `opening_contribution` contains market-valued existing-account entries only. Both are nonnegative; null means the contribution cannot be valued. The performance denominator uses their sum. `withdrawals` is a positive amount. The output contains a baseline row followed by calendar-day rows through end, so weekend/holiday cash events preserve their actual effective dates. Historical security prices may be zero when explicitly observed; missing data is always null. Do not relax ordinary current-price acceptance to implement this.

- [ ] **1. Write replay-equivalence and decreasing-date tests.**

```rust
use lugus_portfolio::{Ledger, ReplayCursor, replay};
use serde_json::json;
#[test]
fn incremental_replay_matches_existing_accounting() {
    let ledger: Ledger = serde_json::from_value(json!({
        "account_id":"a","start":"2026-01-01","opening":{"kind":"full_history"},
        "events":[
            {"id":"d","date":"2026-01-01","order":0,"kind":{"kind":"deposit","amount":"100"}},
            {"id":"w","date":"2026-01-03","order":0,"kind":{"kind":"withdrawal","amount":"25"}}
        ]
    })).unwrap();
    let mut cursor = ReplayCursor::new(&ledger).unwrap();
    for text in ["2026-01-01", "2026-01-02", "2026-01-03"] {
        let day = text.parse().unwrap();
        assert_eq!(cursor.advance_to(day).unwrap(), &replay(&ledger, day).unwrap());
    }
    assert!(cursor.advance_to("2026-01-02".parse().unwrap()).is_err());
}
```

- [ ] **2. Run red:** `cargo test -p lugus-portfolio --test history`.
- [ ] **3. Extract shared replay phases:** validated opening state, sorted event/index validation, and applying one effective event. Cursor owns its working state, event index and FIFO/action-order sets. Preserve `replay` semantics: future event identifiers/orders are validated, but future transactions are not financially applied. Reject advancing backwards. Process every applicable event exactly once; do not clone accumulated sale matches for every valuation date. Add `PortfolioError::Cancelled`; `historical_values` calls the cancellable variant with `|| false`, and the latter checks its callback before each date and at least every 1,000 applied events.
- [ ] **4. Implement historical valuation with dated lookup maps and one cursor per active account.** An account contributes zero before `ledger.start`. For each date advance all active cursors, aggregate cash and quantities and select that date's normalized close. Carry the preceding observation only over explicit calendar closures, retaining its observation date. A missing expected-session close blocks that holding until a valid observed close resumes. Unknown calendar/basis is an issue, never forward-filled. Only holdings with nonzero quantity need a close on that date.
- [ ] **5. Reconcile source and ledger splits before accepting a holding value.** Compare reduced integer ratios and effective dates per instrument and per account holding across the action. A missing or contradictory action invalidates that account/instrument from the mismatch onward until ledger correction/recomputation, even if later quotes are available. Also compare ledger-only actions with source coverage. Do not invalidate dates after a full sale merely because a later split exists; later source actions still participate in source normalization. Splits are per instrument in evidence and applied once per account through replay.
- [ ] **6. Value existing-account entry before its same-day events.** Use opening cash plus opening quantities times the prior session's close in the opening share basis; never use cost basis as market value. Add this amount only to `opening_contribution`. Missing required opening prices yield null contribution and issues. If an account started before the requested baseline, replay through that baseline and use its actual value; do not add its original entry contribution again.
- [ ] **7. Add deterministic valuation fixtures:** cash-only account, opening lots acquired years before the account start, two accounts entering on different dates, fully sold holding, 2:1 and 1:10 split, missing/extra split, weekend dividend/deposit, missing session, unknown source/calendar, post-range split evidence, and late instrument inception. For each split fixture assert value conservation and unchanged cost basis. Include the existing FIFO example's partial sales to verify cursor equivalence beyond deposits.
- [ ] **8. Run green:** `cargo test -p lugus-portfolio`; commit `feat(portfolio): replay and value historical account states`.

## Task 6: Cash-flow-adjusted returns and benchmark calculation

**Files:** Create `lugus-portfolio/src/performance.rs`, `lugus-portfolio/tests/performance.rs`; export from `lib.rs`.

**Interfaces**

```rust
pub struct PerformancePoint {
    pub date: Day, pub value: Option<Decimal>,
    pub deposits: Option<Decimal>, pub withdrawals: Decimal,
    pub opening_contribution: Option<Decimal>,
    pub portfolio_growth: Option<Decimal>,
    pub portfolio_return_percent: Option<Decimal>,
    pub segment_return_percent: Option<Decimal>, pub segment: u32,
    pub benchmark_return_percent: Option<Decimal>,
    pub hypothetical_value: Option<Decimal>, pub issues: Vec<HistoryIssue>,
}
pub struct PerformanceSummary {
    pub portfolio_return_percent: Option<Decimal>,
    pub benchmark_return_percent: Option<Decimal>,
    pub difference_pp: Option<Decimal>,
}
pub struct PerformanceSeries {
    pub baseline: Day, pub end: Day,
    pub points: Vec<PerformancePoint>, pub summary: PerformanceSummary,
}
pub fn daily_growth(previous: &Decimal, close: &Decimal,
    inflows: &Decimal, withdrawals: &Decimal) -> Result<Option<Decimal>>;
pub fn calculate_performance(days: &[DailyValuation]) -> Result<PerformanceSeries>;
pub fn calculate_performance_cancellable(days: &[DailyValuation],
    cancelled: &dyn Fn() -> bool) -> Result<PerformanceSeries>;
```

`days[0]` is the explicit baseline, excluded from selected-period cash flows. `portfolio_return_percent` is the cumulative return from that baseline, null after unknown coverage. `segment_return_percent` permits a visibly separate recapitalization segment after total loss; it is not a substitute for the selected-period summary. An unchanged zero-capital interval carries prior accumulated return only when value/flows/previous capital are all known and zero.

- [ ] **1. Write the arithmetic regression test.**

```rust
use lugus_portfolio::{Decimal, daily_growth};
fn d(v: &str) -> Decimal { Decimal::parse(v).unwrap() }
#[test]
fn deposits_and_withdrawals_are_not_investment_gains() {
    assert_eq!(daily_growth(&d("100"), &d("150"), &d("50"), &d("0")).unwrap(), Some(d("1")));
    assert_eq!(daily_growth(&d("100"), &d("110"), &d("0"), &d("10")).unwrap(), Some(d("1.2")));
    assert_eq!(daily_growth(&d("0"), &d("0"), &d("0"), &d("0")).unwrap(), None);
}
```

- [ ] **2. Run red:** `cargo test -p lugus-portfolio --test performance`.
- [ ] **3. Implement checked growth and percent conversion.**

```rust
let denominator = previous.checked_add(inflows)?;
if denominator.is_zero() { return Ok(None); }
let numerator = close.checked_add(withdrawals)?;
Ok(Some(Decimal::parse("1")?.allocated(&numerator, &denominator)?))
```

Reject negative values/flows and unordered/nonconsecutive input dates. The accumulator multiplies daily growth without cent-rounding and converts to percent only by `(factor - 1) * 100`. Zero denominator with economic activity is an explicit unavailable state. Initial zero capital plus a positive deposit is valid. Existing-account contributions join deposits in the denominator once. Total loss produces factor zero; subsequent positive funding begins a new numbered segment and the full-period summary is unavailable because it spans the reset.

- [ ] **4. Implement the independently valid benchmark series.** When benchmark levels at baseline/current date are positive, return is `100 * (current / baseline - 1)`. Calculate hypothetical value using prior value plus contributions, multiplied by the daily index ratio, minus withdrawals. The hypothetical path stops permanently for the selected range on missing flow/index data or overdraft. Do not suppress index return merely because the hypothetical value is unavailable. If portfolio coverage is incomplete, difference is null; both line labels retain the same requested baseline/end. Check cancellation before each date in the cancellable variant; the ordinary function delegates with `|| false`.
- [ ] **5. Add worked series tests** for two 10% days => 21%; dividend cash offsetting a price drop => 0%; fee reducing cash => negative return; weekend cash flow with unchanged prices; missing baseline/middle/end; cash-only 0%; never-funded null; full withdrawal then re-funding; 100% loss/recapitalization; negative hypothetical balance; benchmark-only missing data. Use `serde_json::from_value::<Vec<DailyValuation>>` with explicit complete fields, so test data are visible rather than hidden behind a calculation helper. Assert null summary and preserved absolute values after a gap.
- [ ] **6. Run green:** `cargo test -p lugus-portfolio`; commit `feat(portfolio): calculate daily returns and benchmark values`.

## Task 7: Revision-safe history jobs, cache, and application storage

**Files**

- Create `lugus-app/src/portfolio/history.rs`, `lugus-app/src/portfolio/history_lease.rs`, `lugus-app/src/store/portfolio/history.rs`, `lugus-app/src/store/portfolio/history-v8.sql`, `lugus-app/src/application/portfolio/history.rs`, `lugus-app/src/application/portfolio/history_input.rs`, `lugus-app/tests/portfolio_history.rs`, `lugus-app/tests/portfolio_history_store.rs`.
- Modify `lugus-app/src/portfolio/mod.rs`, `lugus-app/src/portfolio/types.rs`, `lugus-app/src/store/portfolio/mod.rs`, `lugus-app/src/store/portfolio/write.rs`, `lugus-app/src/store/sqlite.rs`, `lugus-app/src/application.rs`, `lugus-app/src/application/portfolio.rs`.
- Keep `lugus-app/src/application/portfolio/prices.rs` as the reference for current-quote lifecycle; history has independent status/admission.

**Interfaces:** Define the following app types in `portfolio/history.rs`, using `#[serde(deny_unknown_fields)]` on request/range structs and the existing revision serializer. Enum serde names are snake_case.

```rust
pub struct HistoryRange { pub start: Option<Day>, pub end: Day }
pub enum HistoryRefreshMode { Missing, Force }
pub struct PortfolioHistoryRequest {
    pub request_id: String, pub portfolio_id: String, pub account_id: Option<String>,
    pub expected_revision: u64, pub range: HistoryRange, pub refresh: HistoryRefreshMode,
}
pub struct HistoryKey {
    pub portfolio_id: String, pub revision: u64, pub account_id: Option<String>,
    pub requested_range: HistoryRange, pub calculation_version: u32,
    pub benchmark_provider: Option<ProviderIdentity>,
    pub bindings_fingerprint: String,
}
pub enum HistoryStatus {
    Running, Complete, Partial, Failed, Cancelled, TimedOut, StaleRevision,
    Interrupted, InterruptedOrExternal,
}
pub struct HistoryEvidenceRef {
    pub instrument_id: Option<String>, pub fetch_id: String,
    pub run_id: String, pub provider: ProviderIdentity,
    pub manifest_fingerprint: String,
}
pub struct PortfolioHistoryResult {
    pub id: String, pub key: HistoryKey, pub status: HistoryStatus,
    pub baseline: Option<Day>, pub effective_end: Option<Day>,
    pub summary: PerformanceSummary, pub row_count: usize,
    pub evidence: Vec<HistoryEvidenceRef>, pub issues: Vec<HistoryIssue>,
    pub input_fingerprint: Option<String>,
    pub created_at: DateTime<Utc>, pub finished_at: Option<DateTime<Utc>>,
    pub error: Option<String>,
}
pub struct PortfolioHistoryPage {
    pub result_id: String, pub key: HistoryKey,
    pub items: Vec<PerformancePoint>, pub next_offset: Option<usize>,
}
```

Application methods: `start_portfolio_history(PortfolioHistoryRequest) -> Result<PortfolioHistoryResult>`, `portfolio_history_status(portfolio_id: String, id: String) -> Result<PortfolioHistoryResult>`, `cancel_portfolio_history(portfolio_id: String, id: String) -> Result<()>`, `portfolio_history_latest(portfolio_id: String, account_id: Option<String>, range: HistoryRange) -> Result<Option<PortfolioHistoryResult>>`, and `portfolio_history_page(portfolio_id: String, id: String, page: PageRequest) -> Result<PortfolioHistoryPage>` (async except cancellation). Latest returns the original result/key and a UI-derived stale indication when current inputs differ; it does not rewrite its revision.

Add `PortfolioDocument.benchmark_instance_id: Option<String>` with `#[serde(default)]`, and mutation `SetBenchmarkProvider { instance_id: Option<String> }` to the existing command/audit path. Validate a non-null choice against an offered compatible historical provider in `Application::execute_portfolio`; the store validates bounded ID and increments revision. Null means auto-select exactly one compatible provider. Multiple choices produce a benchmark-selection issue; portfolio-only performance can still load. The fixed benchmark symbol is not user-editable in this version.

`PortfolioStore` additions mirror the application with synchronous methods: `portfolio_history_acquire(portfolio_id: &str) -> Result<HistoryLease>`, `portfolio_history_begin(&PortfolioHistoryRequest, &HistoryKey, &HistoryLease) -> Result<(PortfolioHistoryResult, bool)>`, `portfolio_history_save_rows(id: &str, offset: usize, rows: &[PerformancePoint]) -> Result<()>`, `portfolio_history_finish(&PortfolioHistoryResult) -> Result<PortfolioHistoryResult>`, `portfolio_history_read(portfolio_id: &str, id: &str) -> Result<PortfolioHistoryResult>`, `portfolio_history_latest(portfolio_id: &str, account: Option<&str>, range: &HistoryRange) -> Result<Option<PortfolioHistoryResult>>`, and `portfolio_history_page(portfolio_id: &str, id: &str, page: PageRequest) -> Result<PortfolioHistoryPage>`.

Define `HistoryLease` in `history_lease.rs` with private `file: std::fs::File`, `store_key: PathBuf`, and `portfolio_id: String`. Acquire an OS file lock using `File::try_lock` on `canonical_database_path + ".portfolio-history-" + sha256(portfolio_id) + ".lock"`; retain its file until job cleanup and unlock on drop, matching the existing conversation lease's ownership technique. `begin` verifies the supplied lease matches the store and portfolio. This avoids expiring a live job merely because its heartbeat is delayed. A failed lock acquisition returns Conflict and cannot mutate another process's job.

- [ ] **1. Write store tests before creating the migration.** In `portfolio_history_store.rs`, open a schema-7 fixture store using existing `SqliteApplicationStore`/clock/ID setup, create a portfolio and capture its revision. Begin a history request twice and assert the same ID/one admission; reused request ID with different input fails. Write a bounded result page, revise the ledger with a backdated transaction, then finish and assert `StaleRevision`. Reopen offline and ensure the older complete result retains its original revision/evidence.

```rust
let lease = store.portfolio_history_acquire(&request.portfolio_id).unwrap();
let (first, started) = store.portfolio_history_begin(&request, &key, &lease).unwrap();
assert!(started);
let (retry, started) = store.portfolio_history_begin(&request, &key, &lease).unwrap();
assert!(!started);
assert_eq!(first.id, retry.id);
assert!(store.portfolio_history_read("another-portfolio", &first.id).is_err());
```

- [ ] **2. Run red:** `cargo test -p lugus-app --test portfolio_history_store`.
- [ ] **3. Add schema 8 and store methods.** Update accepted application schema range 1–8 and migrate only when old version < 8.

```sql
CREATE TABLE portfolio_history_jobs (
 id TEXT PRIMARY KEY, request_id TEXT NOT NULL UNIQUE,
 portfolio_id TEXT NOT NULL REFERENCES portfolios(id),
 input TEXT NOT NULL, key TEXT NOT NULL, payload TEXT NOT NULL,
 status TEXT NOT NULL, ordinal_count INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX one_active_portfolio_history
 ON portfolio_history_jobs(portfolio_id) WHERE status='running';
CREATE TABLE portfolio_performance_days (
 result_id TEXT NOT NULL REFERENCES portfolio_history_jobs(id),
 ordinal INTEGER NOT NULL, date TEXT NOT NULL, payload TEXT NOT NULL,
 PRIMARY KEY(result_id,ordinal), UNIQUE(result_id,date)
);
CREATE TABLE portfolio_history_evidence (
 result_id TEXT NOT NULL REFERENCES portfolio_history_jobs(id),
 ordinal INTEGER NOT NULL, payload TEXT NOT NULL,
 PRIMARY KEY(result_id,ordinal)
);
PRAGMA user_version=8;
```

Rows are staged against a running result, contiguous in ordinal/date; read APIs expose only published Complete/Partial rows. Publication checks original revision, benchmark selection and all instrument bindings in one transaction. On mismatch mark StaleRevision and do not publish rows. Store bounded error text. Paginate evidence through its table when the metadata's evidence list would exceed the configured response budget; transport representation is handled in task 8. Keep previous complete results rather than updating them in place.

- [ ] **4. Build `history_input.rs` with the pure adapter** `history_inputs(doc: &PortfolioDocument, account: Option<&str>, baseline: Day, end: Day, bundles: &[(HistoryEvidenceRef, HistoryReadPage)]) -> Result<OwnedHistoryInput>`. Define `OwnedHistoryInput { ledgers: Vec<Ledger>, closes: Vec<HistoricalClose>, benchmark: Vec<HistoricalClose>, baseline: Day, end: Day }`; pages for each bundle are accumulated under the job-wide limits before this conversion. Verify source-derived close/factors against the complete action chain with checked Decimal/rational operations. Deduplicate observations and source actions; retain IDs in every valuation row. Identify all ever-held instruments in the requested range, including opening lots and positions sold before today.
- [ ] **5. Implement date resolution and evidence reuse.** With start=null find earliest nonzero opening balance or funding date; do not use earliest lot acquisition. The calculation baseline is the calendar day immediately before the resolved start, including transactions effective through that baseline. Fetch the preceding exchange session to price that baseline when it falls on a closure. This avoids accidentally including a weekend deposit before a Monday range start. Clamp the effective endpoint to the latest completed session shared by required supported calendars, using source manifests, not the current machine's weekday. Report baseline/end and partial coverage rather than shifting the requested start to conceal gaps. Cache reusable successful evidence by provider, binding, query/anchor, source/normalization/calendar versions and manifest; reuse only complete action coverage through the calculation anchor. Force refresh creates fresh evidence; Missing reuses adequate evidence. A current anchor change may require a new bundle because later splits can change the source basis. After evidence selection, hash the canonical key plus ordered evidence references/manifests into `input_fingerprint`; use that full fingerprint to identify an identical calculation, not the pre-fetch key alone.
- [ ] **6. Implement owned jobs using the existing admission/guard pattern.** Add a history map to `Admission`, include it in shutdown cancellation and joins, and enforce the spec's job/concurrency/deadline/source limits in addition to existing worker budgets. Make durable begin failures and in-memory admission failures terminal, not orphaned Running jobs. Use a drop guard to cancel child fetches. Do not hold the application store mutex during network I/O. Cancellation must remain effective during long calculations: run them off the async executor with periodic cancellation checks between days and await completion before reporting terminal cleanup.
- [ ] **7. Persist complete input receipts, compute and stage rows, then publish.** Job status is Complete only with full requested portfolio/benchmark coverage; Partial retains valid independent series plus reasons. Missing benchmark may produce a usable portfolio-only result. Resource-limit, timeout and cancelled work never masquerade as partial success. A process reading a Running job it does not own reports InterruptedOrExternal. A new request can mark an old Running record Interrupted only after acquiring the exclusive portfolio HistoryLease, then atomically replace admission in the same store transaction. A live owner's lock prevents replacement. Reusing an old request ID returns its original terminal result; a retry of interrupted work uses a new request ID.
- [ ] **8. Add app integration fixtures/tests** for history without agent runtime, no compatible provider, ambiguous benchmark provider, cached offline reads, edited opening setup, changed binding, stale completion, rapid re-request, process interruption/recovery, cancellation during fetch/calculation, shutdown, two-account scope, and a missing benchmark. Fixtures use deterministic historical dates and injected current-time/calendar anchors; do not rely on real market dates.
- [ ] **9. Run green:** `cargo test -p lugus-app --test portfolio_history --test portfolio_history_store --test portfolio_store --test portfolio_conversations`; commit `feat(app): persist revision-bound portfolio history jobs`.

## Task 8: Native history transport and exact dashboard metrics

**Files**

- Create `lugus-app/src/portfolio/dashboard.rs`, `lugus-app/tests/portfolio_dashboard.rs`, `lugus-desktop/src-tauri/tests/portfolio_history.rs`.
- Modify `lugus-app/src/portfolio/mod.rs`, `lugus-app/src/portfolio/types.rs`, `lugus-app/src/store/portfolio/mod.rs`, `lugus-desktop/src-tauri/src/portfolio.rs`, `lugus-desktop/src/portfolio/types.ts`.

**Interfaces:** Add `#[serde(default)] pub dashboard: Option<DashboardMetrics>` to `PortfolioView`, so saved snapshots remain deserializable. Define:

```rust
pub struct AllocationMetric {
    pub key: String, pub label: String, pub value: Decimal, pub percent: Decimal,
}
pub struct HoldingMetric {
    pub instrument_id: String, pub unrealized_percent: Option<Decimal>,
}
pub struct DashboardMetrics {
    pub invested_market_value: Option<Decimal>,
    pub by_asset_type: Vec<AllocationMetric>,
    pub largest_instrument_id: Option<String>,
    pub largest_weight: Option<Decimal>, pub top_two_weight: Option<Decimal>,
    pub holdings: Vec<HoldingMetric>,
}
pub fn dashboard_metrics(view: &PortfolioView) -> Result<DashboardMetrics>;
```

Compute money/percent aggregates in Rust. The renderer may order exact decimal strings and format them, but must not calculate authoritative totals/returns using `Number`. Null whole-portfolio metrics for incomplete valuations; holding unrealized percent can still be available for a priced holding. Zero basis => null. Empty/cash-only portfolios have no largest security and valid cash allocation when funded.

Native commands: `HistoryStart { request }`, `HistoryStatus { portfolio_id, id }`, `HistoryCancel { portfolio_id, id }`, `HistoryLatest { portfolio_id, account_id, range }`, `HistoryRead { portfolio_id, id, offset }`, `HistoryEvidence { portfolio_id, id, offset }`, and `HistoryProviders`. Add evidence pagination to PortfolioStore/Application in this task: `portfolio_history_evidence(portfolio_id, id, PageRequest) -> Result<PortfolioPage<HistoryEvidenceRef>>`, with result ID/key included in the native envelope. HistoryProviders returns eligible provider instance ID/name/version; ordinary Providers remains unchanged for current-price binding.

- [ ] **1. Write exact metric regression tests** for a complete two-holding portfolio, cash-only value, null valuation and zero basis. Include a value beyond JavaScript's safe integer limit and assert exact cents/percent strings.

```rust
// After constructing the PortfolioView from an explicit JSON fixture:
let metrics = dashboard_metrics(&view).unwrap();
assert_eq!(metrics.invested_market_value.unwrap().to_string(), "115000");
assert_eq!(metrics.top_two_weight.unwrap().to_string(), "60");
assert_eq!(metrics.by_asset_type.iter().find(|a| a.key == "cash")
    .unwrap().percent.to_string(), "8");
```

- [ ] **2. Run red:** `cargo test -p lugus-app --test portfolio_dashboard`.
- [ ] **3. Implement metrics with Decimal addition/allocated.** Calculate unrealized percent as `100.allocated(unrealized, basis)` only for positive basis and a known gain. Sum asset-type values with cash separately, then divide by complete total. Use stable tie-breaking by instrument ID for largest holdings. Populate dashboard for current overview and previews while allowing old snapshots to retain dashboard=null.
- [ ] **4. Implement native dispatch via existing `bounded_page`.** Start with at most 200 rows, shrink under configured output limits without skipped offsets. All status/read requests validate portfolio ownership; unknown fields/ranges fail before provider work. Metadata/evidence returned from separate bounded reads must match result ID/key. Every page carries original range, account, calculation version and revision; serialize new IDs/revisions as strings. Do not include up to 100 full manifests in the ordinary summary response; return evidence count and a paginated evidence endpoint. Bound header issues to the first 20 distinct `(code,instrument_id,account_id)` examples and include `issue_count` for the total; full dated issues remain on paginated daily rows. Reducing header examples to honor a smaller configured output budget must preserve that count and an explicit “more details in data table” indication.
- [ ] **5. Add transport tests:** create portfolios through Bridge; exercise cached history with no agent runtime, wrong-portfolio result access, invalid date/revision fields, cancelled jobs, schema-7 snapshot reading, and >200 rows with 32KB output limits. The union of pages must equal the persisted date set without duplicates; evidence pages cannot be substituted from another result.
- [ ] **6. Add exact TypeScript interfaces** matching Rust field names and enum values for requests, status/header, points, issues, dashboard, and evidence pages. Keep decimal/revision fields as strings and measurements nullable.
- [ ] **7. Run green:** `cargo test -p lugus-app --test portfolio_dashboard`; `cargo test --manifest-path lugus-desktop/src-tauri/Cargo.toml --test portfolio --test portfolio_history`; `npm --prefix lugus-desktop run build`. Commit `feat(desktop): expose bounded history and dashboard metrics`.

## Task 9: Current overview, allocation, and sortable holdings

**Files**

- Create `lugus-desktop/src/portfolio/allocation.ts`, `lugus-desktop/src/portfolio/dashboard-state.ts`, `lugus-desktop/src/portfolio-dashboard.test.ts`.
- Modify `lugus-desktop/src/portfolio/overview.ts`, `lugus-desktop/src/portfolio/holdings.ts`, `lugus-desktop/src/portfolio/format.ts`, `lugus-desktop/src/portfolio/panel.ts`, and `lugus-desktop/src/style.css`.

**Interfaces**

```typescript
type HoldingSort = 'symbol'|'market_value'|'allocation_percent'|'basis'|'unrealized'|'unrealized_percent';
export function compareDecimal(a:string|null,b:string|null,direction:'asc'|'desc'):number;
export function formatPercent(value:string|null, signed?:boolean):string;
export function sortHoldings(view:View, key:HoldingSort, direction:'asc'|'desc'):Holding[];
export function renderAllocation(root:HTMLElement,view:View):void;
export function renderHoldings(root:HTMLElement,view:View,onOpen:(instrumentId:string,trigger:HTMLElement)=>void):void;
export function renderOverview(root:HTMLElement,view:View,onOpen:(instrumentId:string,trigger:HTMLElement)=>void):{
    performanceRoot:HTMLElement;
    updateSummary:(summary:PerformanceSummary|null,periodLabel:string,stale:boolean)=>void;
};
```

Keep the current portfolio controls and all existing tab actions. The overview gets the performance loading region, allocation and concentration cards and the same reusable holdings table as the Holdings tab. The app stays functional with empty history. Old snapshot dashboard=null is rendered using available existing metrics without inventing missing derived metrics.

- [ ] **1. Add exact decimal-order and formatting tests.**

```typescript
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {compareDecimal,formatPercent} from './portfolio/format.ts';
test('sorting is exact and null remains last in either direction',()=>{
  assert.equal(compareDecimal('9007199254740993.01','9007199254740993.02','asc'),-1);
  assert.equal(compareDecimal(null,'1','desc'),1);
  assert.equal(compareDecimal('-1','0','desc'),1);
  assert.equal(formatPercent(null),'—');
  assert.equal(formatPercent('12.605',true),'+12.60%');
});
```

- [ ] **2. Run red:** `npm --prefix lugus-desktop test`.
- [ ] **3. Extend formatting through existing BigInt coefficients.** Compare full scale-18 coefficients; use half-even rounding for two-digit percent display; normalize negative zero. Return -1/0/1 rather than coercing BigInt differences to Number. Render null last in both sort directions, tie-break by symbol then instrument ID. Use Rust-produced `unrealized_percent`, asset totals and concentration; color/sign are presentational only.
- [ ] **4. Build the approved current-holdings layout.** Use the mockup as a visual reference, not a source of synthetic rows. Render all data with DOM textContent helpers. Allocation widths can use bounded numeric conversions of authoritative percentages solely for CSS geometry. Current total value must remain fixed when history ranges change. Keep accounting metrics/fee disclosures in an expandable section. Show price dates, stale quote warnings and simplified openings; suppress invalid whole-portfolio charts when incomplete. Until task 11 connects the drawer, the row callback opens the existing lot/metric detail display; do not leave the row control inert or remove access to lots during this task.
- [ ] **5. Make sorting accessible:** actual header buttons, `aria-sort` on the active header, visible direction, stable sort and row controls. Empty/cash-only portfolios have useful text and transaction action. Portfolio CSS is scoped under the existing portfolio panel; at 600px the table scrolls within its container and does not widen the whole page. Add breakpoint adjustment for the portfolio-open body grid so chat's minimum-width columns do not force overflow.
- [ ] **6. Run green:** `npm --prefix lugus-desktop test` and `npm --prefix lugus-desktop run build`; browser integration is exercised in task 12. Commit `feat(desktop): refresh portfolio allocation and holdings overview`.

## Task 10: Performance chart, history controller and data table

**Files**

- Create `lugus-desktop/src/portfolio/performance.ts`, `lugus-desktop/src/portfolio/history-state.ts`, `lugus-desktop/src/portfolio/history-controller.ts`, `lugus-desktop/src/portfolio/history-geometry.ts`, `lugus-desktop/src/portfolio-history.test.ts`.
- Modify `lugus-desktop/src/portfolio/panel.ts`, `lugus-desktop/src/portfolio/state.ts`, `lugus-desktop/src/portfolio/types.ts`, `lugus-desktop/src/main.ts`, and `lugus-desktop/src/style.css`.

**Interfaces**

```typescript
export type Period='1M'|'3M'|'YTD'|'1Y'|'All';
export type ChartMode='return'|'value';
export interface HistorySelection {
  portfolioId:string; accountId:string|null; revision:string;
  generation:number; period:Period; resultId:string|null;
}
export function historyRange(period:Period,end:string):{start:string|null;end:string};
export function acceptsHistory(current:HistorySelection,origin:HistorySelection):boolean;
export function sampleHistory(rows:PerformancePoint[],budget:number):PerformancePoint[];
export function historyGeometry(rows:PerformancePoint[],mode:ChartMode):{
  portfolio:string[]; benchmark:string[];
  points:{date:string;x:number;portfolioY:number|null;benchmarkY:number|null}[];
};
export function mountPerformance(root:HTMLElement,callbacks:{
  onPeriod:(period:Period)=>void;onRefresh:()=>void;onCancel:()=>void;
  onDataPage:(offset:number)=>Promise<PortfolioHistoryPage>;
}):{
  loading:()=>void;
  render:(header:PortfolioHistoryHeader,rows:PerformancePoint[],stale:boolean)=>void;
  fail:(message:string)=>void;
  dispose:()=>void;
};
export function createHistoryController(api:PortfolioApi,root:HTMLElement,
  updateSummary:(summary:PerformanceSummary|null,label:string,stale:boolean)=>void):{
  select:(view:View,accountId:string|null,online:boolean)=>Promise<void>;
  refresh:()=>Promise<void>;dispose:()=>void;
};
```

`PortfolioHistoryHeader` is the bounded native summary from task 8: identical to `PortfolioHistoryResult` except evidence list becomes `evidence_count`, and issue examples are bounded with a separate total `issue_count`. Evidence is fetched separately and full dated issues appear in the daily data pages. Preserve the result key, actual baseline/end, original revision and status. `sampleHistory` may exceed its requested visual budget when mandatory gap/flow/extremum points require it; it must never drop a gap to meet a cosmetic cap.

- [ ] **1. Write date and selection tests.**

```typescript
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {historyRange,acceptsHistory} from './portfolio/history-state.ts';
test('calendar periods clamp month ends and preserve explicit scope',()=>{
  assert.deepEqual(historyRange('1M','2024-03-31'),{start:'2024-02-29',end:'2024-03-31'});
  assert.deepEqual(historyRange('1Y','2024-02-29'),{start:'2023-02-28',end:'2024-02-29'});
  assert.deepEqual(historyRange('YTD','2026-09-15'),{start:'2026-01-01',end:'2026-09-15'});
  const old={portfolioId:'p',accountId:null,revision:'4',generation:1,period:'YTD',resultId:'r'} as const;
  assert.equal(acceptsHistory({...old,period:'1M',generation:2},old),false);
  assert.equal(acceptsHistory({...old,revision:'5'},old),false);
});
```

- [ ] **2. Run red:** `npm --prefix lugus-desktop test`.
- [ ] **3. Implement period boundaries from date components in UTC.** Clamp target day to target month's length; no local timezone arithmetic that can change the ISO date. All sends start=null. Use the server's as-of date as end. Selection compares portfolio, account, revision, range generation and result ID. A completed result must be revalidated after every awaited page. Leaving the panel stops polling/listeners; it does not silently cancel a reusable background job. Explicit Cancel does.
- [ ] **4. Implement history loading independently from current overview.** Add `setOnline(online:boolean):void` to the object returned by mountPortfolio and call it from main.ts after runtime info resolves, using `!info.offline`; default to false until initialized. Read HistoryLatest, paint compatible cached rows with original metadata, then online request Missing coverage. Offline does not call HistoryStart. On Force refresh retain the prior series with a refreshing/stale indicator. Poll status without busy loops, expose cancellation and preserve current holdings when history fails. If a different range is requested while one job for the portfolio runs, stop observing the old request and queue only the latest requested range behind that job. Explicit user cancellation cancels it immediately. Ignore intermediate queued ranges. Use the existing revisioned command dialog for benchmark provider choice; no provider setting in localStorage. The existing Refresh prices action starts current-price refresh and calls the history controller's Force refresh, keeping their status/failure handling independent.
- [ ] **5. Implement joint chart geometry.** Calculate x positions from actual dates and one shared y-domain for both visible series. Build separate SVG path segments at null observations or return-segment changes; do not reuse independently scaled single-series geometry. Subtract the exact minimum using BigInt coefficients before conversion, then convert a bounded normalized ratio to Number; this preserves small changes on large balances. Retain exact strings for tooltips. Render a flat line for constant values, a point for single observations, and textual empty state for no usable values. In Return mode use `portfolio_return_percent`, displaying recapitalization segments separately with a clear segment label; never replace the full-period summary with a segment return.
- [ ] **6. Implement visual downsampling and inspection.** Split at gaps first. Preserve first/last dates, every flow date, each gap's adjacent points, and local bucket min/max for both series. Retain the full bounded received rows for keyboard inspection, or fetch the necessary raw page when it is not loaded. Arrow/Home/End keys move a focusable chart cursor; tooltips announce date/exact values. The accessible table reads original native pages and reports pending/error states independently. A long chart must not fetch every original table page just to display its first page.
- [ ] **7. Add methodology and status UI:** S&P 500 total return, daily flow convention, dividends/fees, actual comparison dates, benchmark source/evidence link, independent current-price and history freshness, hypothetical value explanation/overdraft reason. Portfolio and benchmark use solid/dashed lines and signed labels. Show flow markers in Value mode; aggregate same-day deposit/withdrawal/opening amounts separately in the tooltip.
- [ ] **8. Add test fixtures for geometry/async behavior:** a gap in just one series, a gap in both, huge amounts with small differences, negative returns, flat/single series, 10,000 points with mandatory gaps/extrema/flows, stale page after range/account changes, outdated result after a ledger edit, offline no-start, failed refresh retaining prior data, and cancellation. Use deferred Promise callbacks with a fake API to control response order rather than timers.
- [ ] **9. Run green:** `npm --prefix lugus-desktop test`; `npm --prefix lugus-desktop run build`. Commit `feat(desktop): compare portfolio performance with the S&P 500`.

## Task 11: Holding detail panel and research handoff

**Files**

- Create `lugus-desktop/src/portfolio/holding-detail.ts`, `lugus-desktop/src/portfolio/research-context.ts`, `lugus-desktop/src/portfolio-research.test.ts`.
- Modify `lugus-desktop/src/portfolio/panel.ts`, `lugus-desktop/src/main.ts`, `lugus-desktop/src/portfolio/types.ts`, and `lugus-desktop/src/style.css`.
- Add `HoldingRows` to `lugus-desktop/src-tauri/src/portfolio.rs`; add application `portfolio_holding_rows(portfolio_id: String, account_id: Option<String>, instrument_id: String, section: String, page: PageRequest, revision: u64) -> Result<PortfolioPage<serde_json::Value>>` in `lugus-app/src/application/portfolio.rs`, with a synchronous borrowed-string equivalent on PortfolioStore implemented in `lugus-app/src/store/portfolio/mod.rs`.

**Interfaces**

```typescript
export interface ResearchIntent {instrumentId:string;symbol:string;name:string;draft:string}
export function holdingResearchIntent(instrument:Instrument):ResearchIntent;
export function openHoldingDetail(options:{
  view:View;instrumentId:string;accountId:string|null;trigger:HTMLElement;api:PortfolioApi;
  onResearch:(intent:ResearchIntent)=>Promise<void>;
}):{close:()=>void};
```

Extend the existing `onUse(view,accountId)` callback to `onUse(view,accountId,intent?:ResearchIntent)`. The generic Use in chat call remains valid without intent. `HoldingRows { portfolio_id, account_id, instrument_id, section, offset, revision }` accepts `lots` or `transactions`, validates the instrument belongs to the portfolio, filters first, and paginates deterministically. Transactions are newest first by `(date, order, account_id, event_id)`; lots retain account/acquisition/FIFO order. Do not fetch all 10,000 portfolio transactions just to show a holding's recent activity.

- [ ] **1. Add a research-draft test before changing main.ts.**

```typescript
import assert from 'node:assert/strict';
import {test} from 'node:test';
import {holdingResearchIntent} from './portfolio/research-context.ts';
test('research drafts name the selected instrument',()=>{
  const intent=holdingResearchIntent({id:'i',symbol:'MSFT',name:'Microsoft',
    asset_kind:'stock',currency:'USD',binding:null});
  assert.equal(intent.instrumentId,'i');
  assert.equal(intent.symbol,'MSFT');
  assert.match(intent.draft,/Microsoft/);
});
```

- [ ] **2. Run red:** `npm --prefix lugus-desktop test`.
- [ ] **3. Implement an accessible native `<dialog>` styled as a right drawer.** Use showModal for browser focus containment/inert background. Set title/description, focus Close on entry, handle Escape and outside-coordinate backdrop clicks, and restore focus/scroll on dismissal. Cancel/ignore pending detail loads when closed or selection changes; errors remain in its role=status region. Render initial holding metrics immediately, then paginated lots/transactions. Include simplified/assumed history explanations and quote date.
- [ ] **4. Implement prepared research context.**

```typescript
export function holdingResearchIntent(instrument:Instrument):ResearchIntent {
  return {instrumentId:instrument.id,symbol:instrument.symbol,name:instrument.name,
    draft:`Help me review my ${instrument.name} (${instrument.symbol}) holding in the context of my portfolio.`};
}
```

In main.ts reuse snapshot creation for the selected account. Only after snapshot creation succeeds should portfolioPanel.hide() run, company-hint be set to intent.symbol, the snapshot attachment text be updated, and the editable draft inserted. If the composer already has user text, preserve it and append the prepared question with a blank line; never discard an unsent draft. Do not call the send/submit handler. On snapshot revision conflict keep the drawer open with refresh/retry controls. For an ETF set only an instrument hint, not a fabricated resolved company.
- [ ] **5. Add native filtered-page tests** for a holding belonging to another portfolio, account mismatch, stale revision, lots across two accounts and >200 activities. Assert deterministic pages and no unrelated symbols. Add browser behavior checks for focus containment/restore, backdrop, Escape, failed detail load/retry, and research draft without a sent message; task 12 owns the executable browser harness.
- [ ] **6. Run green:** `npm --prefix lugus-desktop test`; `npm --prefix lugus-desktop run build`; `cargo test --manifest-path lugus-desktop/src-tauri/Cargo.toml --test portfolio_history`. Commit `feat(desktop): inspect holdings and prepare contextual research`.

## Task 12: End-to-end verification and release documentation

**Files**

- Create `lugus-desktop/tests/portfolio-dashboard-browser.cjs`, `lugus-desktop/src-tauri/examples/portfolio_history_qa.rs`, `lugus-desktop/tests/fixtures/history_provider.py`.
- Modify `lugus-desktop/tests/README.md`, `lugus-desktop/tests/portfolio-browser.cjs`, `lugus-desktop/README.md`, `lugus-portfolio/README.md`, and `lugus-app/README.md`.
- Create `docs/superpowers/verification/2026-09-15-portfolio-dashboard.md` to record actual commands/results, fixture and live checks, remaining limitations, and screenshots.

**Interfaces:** `portfolio_history_qa` implements the existing examples' JSON-line bridge against fresh temporary SQLite and the deterministic history fixture, without any real agent runtime. The browser harness builds the real desktop bundle and talks to this bridge through the same local HTTP shim used by `portfolio-browser.cjs`. The fixture contains explicit historical source days, cash flows and a stock split; it never links into production.

- [ ] **1. Create the deterministic fixture:** portfolio begins with $100 on Jan 2; buys 10 shares at $10; next session closes at $11 ($110, +10%); then a $50 deposit with flat price ($160, still +10%); a 2:1 split produces 20 shares at $5.50 (unchanged value); a $5 fee produces $155. Benchmark levels are 100, 105, 105, 105, 105. Include matching explicit session/closure rows and source split factors through the anchor. Compute expected final return exactly as `1.1 * (155/160) - 1 = 6.5625%`, benchmark 5%, difference 1.5625pp. Cost basis remains $100. Freeze the fixture clock so ranges/as-of do not drift with real time.
- [ ] **2. Write browser assertions against the real native-backed UI.**

```javascript
await page.getByRole('button',{name:'Portfolio',exact:true}).click();
await page.getByText('+6.56%',{exact:true}).first().waitFor();
await page.getByRole('button',{name:'Open TEST holding',exact:true}).click();
await page.getByRole('dialog',{name:/TEST/}).waitFor();
await page.keyboard.press('Escape');
await page.getByRole('button',{name:'Open TEST holding',exact:true}).waitFor();
await page.getByRole('button',{name:'Value $',exact:true}).click();
await page.getByText('Hypothetical S&P 500 investment',{exact:true}).waitFor();
```

Use these accessible names in the production renderers. Complete the checks with data-table values, allocation totals, sorting, both account scopes, chart range changes, holding activity pagination, and research draft/no-submit. Correct a backdated fee in the UI and verify the refreshed chart and summary change while the old persisted result retains its old revision. Hide one required source price in a separate fixture mode and verify a real SVG break/null summary, not a zero.
- [ ] **3. Run focused integrations before the full suite:** build the QA example and bundle, run the new browser harness, then update/run the existing `portfolio-browser.cjs` to use the new table/detail controls while retaining all transaction/FIFO/cash/snapshot assertions. Tests must use fresh temporary databases and clean up the native child/server/browser in finally blocks. Capture browser console errors and fail on them.
- [ ] **4. Inspect screenshots** at 1440×1000 and 600×900 for overview, drawer, incomplete history, cash-only, and offline states. Compare against the approved mockup. Confirm narrow viewport body width, table-local scrolling, keyboard chart values and drawer focus restoration. Avoid blanket snapshot tests for incidental CSS; assert user-visible behavior and inspect the visual artifacts.
- [ ] **5. Run required final commands after all code changes.**

```bash
cargo fmt --all -- --check
cargo test --workspace
cargo fmt --manifest-path lugus-desktop/src-tauri/Cargo.toml -- --check
cargo test --manifest-path lugus-desktop/src-tauri/Cargo.toml --tests
npm --prefix lugus-desktop test
npm --prefix lugus-desktop run build
cargo build --manifest-path lugus-desktop/src-tauri/Cargo.toml --example portfolio_history_qa --example portfolio_qa
```

From the plugin directory, run `PYTHONPATH=. .venv/bin/python -m unittest discover -s tests`. From `lugus-desktop`, run `node tests/portfolio-dashboard-browser.cjs` and `node tests/portfolio-browser.cjs` with the existing `LUGUS_PLAYWRIGHT_MODULE`/`LUGUS_CHROMIUM` overrides if needed. Do not install browser dependencies into unrelated projects.
- [ ] **6. Finish the live compatibility check from task 2** if it could not run earlier. Check actual end-to-end fetch/store/calculate/read/render with a small isolated live portfolio whose transactions are explicitly labeled QA, using public historical observations only. Do not populate the user's portfolios. Record actual source dates/manifest IDs and precise success/failure. Source outages prevent claiming live verification, even when fixtures pass; report that limitation separately rather than publishing a completed result.
- [ ] **7. Update documentation** with the return convention, benchmark symbol/total-return meaning, cash/dividend/fee handling, opening-date limitation, supported provider/calendar versions, missing-price behavior, refresh/cancel/offline usage and screenshot paths. Replace the portfolio README's claim that no historical performance engine exists. Keep unsupported corporate actions and source completeness limitations visible in methodology docs.
- [ ] **8. Self-review the final diff and seek code review using the applicable skill.** Verify task coverage against the matrix below, run `git diff --check`, inspect migrations/serialization compatibility and confirm no runtime mockup/session files or downloaded source data are staged. Commit `test: verify historical portfolio dashboard end to end` and record completion only after observing the required results. Finish the branch using the repository's integration workflow; do not push/merge solely because the implementation tests passed.

## Spec coverage and acceptance matrix

| Approved requirement | Implementing tasks | Required evidence |
|---|---|---|
| Historical source basis, split actions, calendars | 1–3 | Contract, normalization, split and calendar tests; live check |
| Provider lifecycle, scope, source budgets | 4, 7 | Worker cancellation/provenance/budget tests |
| Effective-dated replay, FIFO, opening accounts | 5 | Cursor equivalence, opening contribution and split cases |
| Daily return, dividends/fees, total loss | 6 | Exact worked decimal fixtures |
| S&P total-return and hypothetical cash flows | 2, 6, 7 | Live symbol check, independent series/overdraft fixtures |
| Immutable cache, backdated edits, offline | 3, 7, 12 | Migrations, stale completion, reopen and correction checks |
| Pagination, output budgets, metadata consistency | 3, 8, 11 | Native bounded-page/no-skip tests |
| Current summary, allocation, holdings, accounting | 8, 9 | Exact metrics and approved-layout browser inspection |
| Chart modes/ranges/tooltips/gaps/large histories | 10, 12 | Geometry, race, downsampling and browser checks |
| Accessible data and responsive layout | 9–12 | Keyboard/table/drawer/600px checks |
| Holding details and research without auto-send | 11, 12 | Filtered pages, snapshot scope, preserved draft |
| Existing workflows and saved snapshots | 8, 12 | Existing native/browser suites and old-schema fixtures |

## Plan review checklist

- [x] All files and cross-task interfaces resolve to existing paths or definitions in this plan.
- [x] All spec requirements map to the acceptance matrix.
- [x] No hidden source fallback, price-basis assumption, or missing-data-as-zero shortcut.
- [x] Dates, percentage units, nullability, revision strings and result identity match across Rust and TypeScript.
- [x] Each task has a behavioral verification and a reviewable commit boundary.
- [x] Tests and live verification are reported only after execution, not inferred from this plan.
