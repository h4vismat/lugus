# Portfolio dashboard and S&P 500 comparison

## Outcome and approved direction

Help the user understand today's holdings and compare historical investment performance with the S&P 500. The user approved the overview layout and the interaction in which selecting a holding opens details inside the portfolio, with a separate Research action.

Visual reference: `.superpowers/brainstorm/1062754-1789483449/content/portfolio-layout.html`. Its figures and curves are illustrative and must not enter production. Preserve its soft green palette, serif page title, compact summary, prominent performance chart, allocation/concentration cards, and holdings table. Keep the current portfolio/account selectors and transaction, refresh, account-management, audit, and chat workflows.

This is an architectural extension: historical accounting exists, but historical valuation, return calculation, benchmark evidence, and persisted performance results do not.

## Existing foundations

- `lugus-portfolio` replays effective-dated transactions with explicit same-day ordering, opening balances, FIFO lots, dividends, fees, and splits. Financial arithmetic uses checked scale-18 decimals.
- `lugus-app/src/portfolio` and `src/store/portfolio` implement revisioned documents, audit history, scoped immutable chat snapshots, and latest-price receipts.
- `lugus-app/src/application/portfolio/prices.rs` owns bounded, cancellable background price jobs. Results are rejected when the ledger revision changes.
- `lugus-financial` stores daily market evidence with provider identity, retrieval information, conflicts, and coverage. Its bundled yfinance provider preserves source Close and Adjusted Close separately.
- `lugus-app/src/portfolio/prices.rs` deliberately accepts recent prices only. Historical source Close cannot simply be multiplied by old share quantities because its split basis can differ.
- `lugus-desktop/src/portfolio` is a plain TypeScript DOM UI. Keep that stack; use SVG for charts and existing exact decimal formatting for displayed financial values.

## Product scope

### Overview

1. Summary: current total value, invested market value and cash, selected-period portfolio return, benchmark return, and their difference in percentage points. Use neutral labels such as “Vs. benchmark,” which work for either sign.
2. Performance chart: Return % and Value $ modes; 1M, 3M, YTD, 1Y, All. Start with YTD. Show the actual start/end dates and any shortened coverage explicitly.
3. Allocation: ranked horizontal bars, including cash, switchable between individual holdings and stocks/ETFs/cash. Colors remain consistent across views.
4. Concentration: largest holding and the combined weight of the two largest positions, as in the approved mockup, plus cash amount/share. Describe position concentration; do not imply ETF underlying exposure has been analyzed.
5. Holdings: sortable symbol/name, market value, portfolio weight, cost basis, unrealized gain/loss, and unrealized gain as a percentage of remaining cost basis. Label the last column “Unrealized %” to distinguish it from period performance. Undefined percentages, including zero basis, display an em dash.

Today's summary and holdings remain visible while history loads or fails. Selecting a chart range changes performance figures and the chart, not today's holdings or total value. Realized P&L, dividend income, fees, deposits, and withdrawals remain available in a collapsible accounting summary; trade fees retain their existing “already included in P&L” explanation.

### Holding details and research

Selecting a holding opens a right-side detail panel with name, symbol, current value, quantity, weight, latest price/date, cost basis, unrealized gain/loss, remaining lots by account, and recent recorded transactions. Show unpriced reasons and simplified-opening-history labels. On small windows the panel occupies the available width.

Close button, Escape, and backdrop dismissal restore focus to the originating row control. Keep focus within the open panel; the background is inert. Loading and failure states appear inside the panel without discarding the overview. Selection and scroll position survive dismissal.

Research prepares the existing chat view with a fresh portfolio snapshot scoped to the selected account, the selected instrument as the company hint, and an editable draft question about that holding. Do not send the message automatically. Label the attached context accurately: it is an account/portfolio snapshot, not a holding-only snapshot. If creating the snapshot fails or its revision is stale, retain the detail panel and show a retryable error. ETFs may be researched as instruments without inventing a corporate/filings binding.

### Chart interaction and accessibility

Both lines share date tooltips and a visible legend. Differentiate them with solid/dashed strokes as well as color. Keyboard users can focus the chart and move between observations with arrow keys. Provide a paginated data table for exact chart values. Gain/loss labels include signs and text; color is supplementary. Honor reduced motion. Allocation labels and detail controls remain usable at a 600px window width; the holdings table may scroll within its container.

Charts break at unavailable observations. Never draw through gaps, turn missing values into zero, or renormalize partially priced holdings to 100%. With missing current prices retain the existing Priced subtotal semantics, list affected holdings, and suppress whole-portfolio allocation/concentration percentages.

## Benchmark and performance method

### Benchmark

Use the USD S&P 500 Total Return index, source identifier `yahoo:symbol:^SP500TR`, from a configured compatible yfinance provider. Its index level already incorporates reinvested dividends; compare Close ratios rather than applying an additional dividend adjustment. Display “S&P 500 total return” and expose provider, identifier, observation dates, and source links in methodology details.

S&P describes its total-return index as reinvesting constituent dividends on the ex-dividend date. [S&P Dow Jones Indices](https://www.spglobal.com/spdji/en/education/article/faq-sp-500-dividend-points-index/). Yahoo lists the corresponding [S&P 500 (TR) instrument](https://finance.yahoo.com/quote/%5ESP500TR/). A listing establishes an intended instrument, not guaranteed API coverage: provider fixture and live compatibility checks must verify actual history before release.

If the index is unavailable, show the portfolio series with a benchmark-unavailable state. Do not silently substitute the price-only S&P 500 or an ETF. Use the single compatible configured provider automatically; where several exist, require a persisted benchmark-provider choice rather than selecting one arbitrarily. Do not add a benchmark instrument to the user's holdings.

### Return %

Use daily time-weighted returns, with deposits at the start of the day and withdrawals at its end. This is a documented daily convention, not an assertion of exact intraday valuations. For each valid day:

`growth = (closing_value + withdrawals) / (previous_closing_value + deposits)`

Compound daily growth factors, then subtract one. Portfolio Performance documents this convention and compounding approach. [Method reference](https://help.portfolio-performance.info/en/concepts/performance/time-weighted/).

Buys/sells move value between securities and cash; they are not external flows. Recorded dividends remain in cash unless reinvested through a recorded purchase. Fees reduce value and therefore return. Do not use dividend-adjusted prices for held shares and also add recorded dividends. Benchmark return is the corresponding index-level ratio. The comparison gap is portfolio return minus benchmark return, in percentage points.

Use checked decimal arithmetic throughout accounting and return calculation. Convert only normalized chart geometry to JavaScript numbers; do not compute authoritative returns or monetary totals in the renderer.

### Value $

Plot actual closing portfolio value and the hypothetical value of investing the same starting capital and subsequent external flows in the benchmark:

`benchmark_value_today = (benchmark_value_yesterday + deposits) × index_growth_today − withdrawals`

At the selected-range boundary seed the hypothetical account from the actual portfolio valuation. Apply the same opening-account contributions described below. Label this “Hypothetical S&P 500 investment,” explain that it is a frictionless index comparison, and expose the starting value and flow convention. Show deposit/withdrawal markers with dates and amounts; combine same-day markers while preserving totals in the tooltip.

If a withdrawal would overdraw the hypothetical investment, stop that hypothetical value series and explain the reason. Do not introduce borrowing, clamp the balance to zero, or suppress the portfolio/index return comparison when those remain valid.

### Dates, opening balances, and zero capital

- Calculate daily closing values through the latest completed supported exchange session, with the effective endpoint shown. Today's current valuation may have a later timestamp and must remain separately labeled.
- 1M/3M/1Y subtract calendar months/years, clamping month-end dates; YTD includes returns from the previous year's closing baseline; All starts at the first funded, valuably reconstructable date. Retrieve the preceding valid close for a range baseline.
- Full-history accounts contribute zero before their start and derive funding from recorded deposits. Do not require prices for instruments before they are owned, after they have been completely sold, or for an empty account.
- Existing-balance accounts begin at their configured start, never at their lots' acquisition dates. Value the opening cash/shares using the prior valid closing prices on the matching share basis. Treat that value as an external account-entry contribution for aggregation, while leaving the accounting ledger's deposit totals unchanged. Same-day events are processed after opening setup.
- All-account performance includes an account only from its start date onward. An unpriceable account-entry contribution makes the affected aggregate return unavailable. Account-filtered history remains independent.
- With no invested capital and no funding, show no return. A zero-capital interval with no economic activity can retain the preceding cumulative result. Funding starts/resumes calculation using the deposit denominator. A total investment loss is represented as −100%; recapitalization after that starts a separately labeled return segment, never a division by zero or silent reset of the selected-period summary.
- Missing prices or required cash-flow boundary values make the affected period return unavailable. Absolute values may resume when complete pricing resumes. Do not compound across an unknown interval or silently move the starting date. A user can explicitly choose a shorter complete period.
- The benchmark comparison uses the same baseline and endpoint as the portfolio. Never show a full-range benchmark next to a shorter-range portfolio under one common period label.

## Historical price evidence and splits

Add a versioned historical-evidence contract in `lugus-financial`, separate from the existing `market_data.daily` contract. The new contract returns dated closes on the shares-outstanding basis of each trading date, source split actions, exchange-session coverage, and a manifest linking the observations to their provider and retrieval. Preserve the current daily API and its interpretation for existing consumers.

The bundled yfinance adapter must request split actions and sufficient source history through a common retrieval anchor, including splits after the requested chart end. Reconstruct a historical trading-date close by reversing the source's subsequent split adjustments. For a 2-for-1 split after a bar date, multiply that split-adjusted close by two. Same-date splits are excluded from that reverse factor because the closing price is already on that date's post-split basis. Keep the original close, the derived close, the factor, anchor, and source actions as evidence. Dividend adjustments are not used in this conversion.

Source and recorded splits must agree wherever an account holds the instrument across an action. A missing or contradictory ledger split blocks affected valuations with a correction message. Provider evidence must never silently insert a transaction into the user's ledger. Deduplicate source actions at instrument level; apply user-recorded actions once per account through existing replay.

Use an explicit supported exchange-session calendar in the historical adapter. Carry the preceding compatible close over scheduled closures only, with the original observation date retained. An absent observation on an expected session is a gap, not an inferred holiday. Unknown calendars, unresolved source basis, conflicting observations, and unsupported corporate actions produce explicit unavailable results. A provider's unverified coverage remains labeled unverified even when the received rows pass validation.

The historical-evidence adapter is the first implementation milestone. Confirm its split reconstruction against pinned-provider fixtures and a controlled live sample; if the provider cannot supply the required semantics, expose history as unavailable instead of weakening the calculation contract.

## Architecture and data flow

### Pure calculation: `lugus-portfolio`

Add focused `history.rs` and `performance.rs` modules. Inputs are validated ledgers, normalized historical closes with observation references, explicit session coverage, benchmark levels, and an inclusive range. Outputs are daily decimal values, external-flow amounts, return growth factors, benchmark values/returns, and typed unavailable reasons.

Reuse ledger validation, FIFO, and split logic. Introduce an ordered replay iterator so a long history applies each event once and values its resulting state by day, rather than replaying the entire ledger separately for every date. Verify iterator end states against the existing `replay` function. Keep provider I/O, SQL, and UI formatting outside this crate.

### Evidence: `lugus-financial`

Add the historical contract, provider capability/dispatch support, validation, and persisted bundles alongside existing market evidence. A bundle manifest identifies instrument, currency, provider instance/version, retrieval anchor, query bounds, source basis, normalization version, action coverage, and session calendar/version. Page bars and actions rather than storing or transporting an unbounded JSON array. Index normalized bars by manifest and date; retain immutable source observations for reproducibility.

Existing providers and saved ordinary price charts continue to work without the new capability. Historical prices from an incompatible provider are unavailable with a clear explanation; ordinary current portfolio operations remain usable.

### Orchestration and persistence: `lugus-app`

Add `src/application/portfolio/history.rs`, `src/store/portfolio/history.rs`, and history request/result types under `src/portfolio`. Extend the store through an additive SQLite migration; preserve previous portfolio documents, audits, price receipts, and chat snapshots.

A history request captures portfolio revision, selected account, date range, benchmark provider, and calculation version. Collect evidence for every instrument held during that range, including fully sold positions, plus splits needed to establish the price basis. Calculate from one immutable input set, persist the result, and publish it only if revision and bindings still match.

Reuse the existing job ownership/cancellation pattern. Bound jobs to one active history refresh per portfolio, at most four application-wide, two concurrent provider calls per job, a five-minute deadline, at most 100 instruments, and 100,000 source observations per job. Reject excess work with a message to narrow the range/account; never silently truncate All. Process historical evidence in pages of at most 200 rows and honor tighter configured output-byte limits.

Opening a portfolio reads saved results first. An online history request fetches missing coverage without requiring chat or an agent runtime. Refresh prices also schedules historical coverage refresh. Show separate current-price/history statuses and a cancel control. Offline mode reads saved evidence/results and explains missing coverage without starting providers.

Cache identity includes portfolio revision, account scope, range, instrument/benchmark bindings, evidence-manifest identities, and calculation version. Editing a backdated event, opening setup, split, or binding invalidates relevant results immediately. A whole-scope recomputation is acceptable initially; incremental invalidation is an optimization. Failed refreshes preserve the prior valid result with its original revision/date and a stale label. Never combine old calculated rows with a new ledger revision.

### Native transport and desktop UI

Extend `lugus-desktop/src-tauri/src/portfolio.rs` with typed history-start, history-status, history-cancel, and paginated history-read commands. Requests include expected revision; every result/page includes result ID, revision as a string, account scope, range, and calculation version. Metadata includes actual coverage, benchmark identity, summary returns, and typed limitations. Daily rows contain date, portfolio value, external inflow/outflow, portfolio return, benchmark index return, hypothetical benchmark value, and missing-data reasons; absent measurements are null.

Keep current overview responses small and usable independently. The frontend loads history asynchronously and rejects responses for old portfolio/account/range/result generations. Chart range changes and detail-panel loads must not let a slower prior request replace the current selection.

Split presentation into focused `performance.ts`, `allocation.ts`, and `holding-detail.ts` modules. Update `overview.ts` and `holdings.ts` for the approved layout, `panel.ts` for orchestration, `types.ts` for transport types, and `main.ts` for the research handoff. Add scoped portfolio CSS instead of altering the research/chat styling. Add loading, empty, cash-only, incomplete, offline, stale, and failed states alongside normal rendering.

For large ranges, use extrema-preserving visual downsampling while retaining endpoints, flow dates, and gap boundaries. Summary numbers always use the full calculation. Paginated accessible tables expose the underlying daily values.

## Verification and acceptance

1. Pure math fixtures: unchanged $100 plus a $50 deposit gives $150 and 0%; $100 increasing to $110 with no flow gives 10%; $110 close plus a $10 withdrawal gives 20% on $100 opening capital; two 10% days compound to 21%.
2. Dividend fixture: a holding moves from $100 to $95 and pays $5 into cash; total value remains $100 and return is 0%. Fees reduce the result exactly once.
3. Split fixtures: 10 shares at $100 become 20 at $50 with no gain; cover reverse splits, split dates, a split after chart end, sold positions, and multiple accounts. Source/ledger mismatch yields a gap.
4. Flow/opening fixtures: initial funding, existing lots valued at market rather than cost, accounts entering at different dates, same-day flows/trades, full withdrawal, zero capital, total loss, and an overdrafted hypothetical benchmark.
5. Coverage fixtures: scheduled closure, missing expected session, no benchmark, late IPO, unsupported currency/calendar, conflicting prices, and a missing baseline. No false zero, line bridging, or full-period summary over missing data.
6. Persistence/job integration: migration from schema 7, offline reopen, stale revision, corrected backdated transaction, changed binding, provider errors/timeouts, cancellation/shutdown, result pagination, output budgets, and immutable evidence receipts.
7. Desktop tests: controls and comparison figures, sorting including null values, allocation totals/cash, partial valuation, chart gaps and keyboard inspection, drawer focus/Escape, research context without auto-send, and account/range race rejection.
8. Browser/native smoke: compare production UI against the approved mockup at 1440px and 600px; run the existing transaction/FIFO/snapshot flow to detect regressions. Synthetic data stays in test fixtures only.

Run the focused Rust, provider Python, TypeScript, native transport, and browser suites at their respective milestones. Release acceptance requires a verified historical adapter and real end-to-end evidence-to-chart flow, not just a rendered dashboard with unavailable performance.

## Delivery order

1. Historical evidence contract and verified source normalization.
2. Pure replay/valuation/performance calculations with worked fixtures.
3. Persisted, revision-safe history jobs and native transport.
4. Approved overview, charts, holdings details, and research handoff.
5. End-to-end verification and documentation.

Each milestone is independently testable but belongs to this one portfolio feature. This version excludes sector/ETF look-through, targets/rebalancing, additional benchmarks, money-weighted return, forecasting, and tax reporting.

## Review status

Visual direction and holding interaction approved in chat. This written design makes the historical methodology, source requirements, failure behavior, and implementation boundaries concrete for review before the implementation plan.
