# Portfolio dashboard verification — September 15, 2026

## Result

Implemented the approved dashboard inline on `feat/portfolio-dashboard`. Current holdings, cash and allocation remain independent of the selected historical range. The historical chart compares exact cash-flow-adjusted portfolio returns with the fixed S&P 500 total-return index; value mode shows the same-cash-flow hypothetical investment. Missing values remain gaps. Transactions, accounting, audit, current-price refresh and scoped chat snapshots remain available.

## Final verification

All commands below passed on the implementation tree:

| Check | Result |
|---|---|
| `cargo fmt --all -- --check` and native manifest equivalent | Passed |
| `git diff --check` | Passed |
| `cargo test --workspace --quiet` | 506 tests passed; no failures/ignored tests |
| Additional final `cargo test -p lugus-app --test portfolio_dashboard --quiet` | 2 tests passed, including the added two-holding/zero-basis fixture |
| `cargo test --manifest-path lugus-desktop/src-tauri/Cargo.toml --quiet` | 18 tests passed |
| `cargo build --manifest-path lugus-desktop/src-tauri/Cargo.toml --example portfolio_qa --quiet` | Passed |
| `npm --prefix lugus-desktop test` | 30 tests passed |
| `npm --prefix lugus-desktop run build` | TypeScript and production bundle passed |
| `.venv/bin/python -m unittest discover -s tests` from the yfinance plugin directory | 37 tests passed |
| `node lugus-desktop/tests/portfolio-browser.cjs` | Passed; no browser errors |
| `node lugus-desktop/tests/portfolio-dashboard-browser.cjs` | Passed; no browser errors |
| `node lugus-desktop/tests/portfolio-live-browser.cjs` | Passed; real fetch/store/calculate/read/render; no browser errors |

Browser commands used `LUGUS_PLAYWRIGHT_MODULE=/home/havismat/.npm/_npx/e41f203b7505f1fb/node_modules/playwright` and `LUGUS_CHROMIUM=/usr/bin/chromium`. A first Python invocation from repository root lacked the plugin import path; rerunning from its documented directory passed. Existing Rust warnings about an unused import and deprecated `fetch_update` remain.

## Evidence and artifacts

- Existing transaction/FIFO/opening-account/snapshot flow: `/tmp/lugus-portfolio-browser-KLHYyt`.
- Dashboard flow: `/tmp/lugus-dashboard-browser-k8AWuM`, including `dashboard.png`, `narrow.png` and `gap.png`. Reviewed the desktop and 600px layouts. Assertions cover keyboard chart values, period races, exact paged data, source links, sorting, scoped drawer pages/focus, refresh from Holdings, preserved draft/no-send and persisted offline reopening.
- Controlled source normalization check: `/tmp/lugus-history-live.json`. Real AAPL August 2020 split evidence and recent `^SP500TR` passed the pinned provider checks through normalization anchor September 15, 2026.
- Live native-backed dashboard: `/tmp/lugus-live-dashboard-snNwbl/report.json` and `live-dashboard.png`. A labelled synthetic AAPL opening account was created only in disposable QA databases. Saved 258 calendar-day performance rows through September 14, 2026. Source runs `1` (AAPL, NASDAQ) and `2` (`^SP500TR`, NYSE) used yfinance 0.3.0, calendar 5.4.0, normalization 1, anchor September 15; retrieved at 19:06:48Z and 19:06:50Z. Source coverage began December 30, 2025. The UI correctly showed the pre-opening period without portfolio returns and an unpriced current holding because this check refreshed historical evidence only.

Artifacts are temporary local QA files and source downloads; none are committed.

## Inline review and implementation choices

Reviewed protocol identity, migration compatibility (financial schema 6/application schema 8), exact source factors, immutable publication, cancellation ownership, bounded reads, null values, page identities and draft handling. Review fixes covered yfinance 0.3.0 compatibility in the existing price-chart tool, agent-independent provider startup, changed-manifest protocol invalidation, exact serialized byte limits, cancellation during normalization, adaptive staging, compact publication failure records, source metadata, pending-start cancellation races and refreshing history from every portfolio tab.

The existing `portfolio_qa` example and managed-worker fixture were extended instead of introducing duplicate test hosts. Exact split/opening/dividend/fee/return tests are at the pure-engine layer; native/browser tests exercise the composed transport and UI. Backdated revision publication and process ownership have store tests; browser interaction is not an exhaustive cross-product of every ledger and provider failure. No automated external code-review agent was used because the user chose inline execution.

## Practical limits

- Historical data requires yfinance 0.3.0 and the pinned calendar dependency. The benchmark has no substitute if `^SP500TR` is unavailable.
- Source completeness is unverified. Explicit unsupported corporate actions or unmatched ledger splits leave gaps; unreported source actions cannot be detected reliably.
- Existing holdings begin performance on their account setup date, with market-valued opening contributions. They do not reconstruct returns from their original lot acquisition dates.
- One owned history job per portfolio, four globally, two simultaneous source fetches per job, five-minute deadline, 100 instruments, 100,000 source rows and 128 MiB accumulated evidence; configured per-page byte limits also apply. Conservative source loading from account setup can make long histories reach the limit. Limits fail explicitly rather than publish truncated calculations.
- Saved results keep their original revision/date. Offline mode can display saved ranges, but cannot fetch an uncached range.
- The live check demonstrates compatibility on this date, not upstream completeness or future availability.

Merged locally into `main` by fast-forward to `3c50eac` after the user requested local integration. Post-merge native tests (18), frontend tests (30), production build and provider tests (37) passed. The first parallel workspace run encountered four `Text file busy` errors launching temporary Codex test fixtures; the focused reproduction passed, and `cargo test --workspace --quiet -- --test-threads=1` passed all 507 tests. Formatting and diff checks passed. The feature worktree and branch can now be cleaned up. Nothing was pushed.
