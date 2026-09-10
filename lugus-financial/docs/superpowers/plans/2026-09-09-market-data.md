# Market Data Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development for independent plugin/storage work and review. Keep coupled domain and host work together.

**Goal:** Fetch yfinance daily market data and persist independently queryable evidence.
**Architecture:** Existing JSON-RPC host gains one optional capability. Market data uses its own domain and repository contracts while sharing process, identity and error primitives.
**Tech Stack:** Rust/Tokio/Serde/rusqlite; Python/yfinance in plugin-local venv.
**Spec:** ../specs/2026-09-09-market-data-design.md; exact wire contract ../../protocol/market-data-v1.md.

## Global constraints

Preserve existing SEC workflow and version-1 SQLite data. No Alpha Vantage API calls or .env key reads. No auto-commits or publishing; preserve existing staged work on current feature branch. Provider-native OHLC and adjusted close remain distinct; no claims of exact upstream decimal precision or verified calendar completeness.

## Tasks

- [x] Domain and host: replace src/market_data.rs prototype with InstrumentId, PriceQuery, PriceBar, PriceCoverage, PricePage, PriceBasis, PricePrecision, Completeness and pure validators/fingerprints. Add MarketDataProvider to src/capabilities.rs and Plugin adapter. Tests first in tests/market_domain.rs and tests/market_plugin.rs; run failures then implement and pass.
- [x] Python plugin: plugins/yfinance/{provider.py,main.py,plugin.json,requirements.txt,README.md,tests/test_provider.py}. Follow exact protocol. Inject history effect, normalize purely, propagate source errors, bound snapshot and line sizes, bind cursor. Demonstrate failing tests before implementation and passing unittest fixtures afterward.
- [x] SQLite/application: src/storage/market.rs, src/storage/market-v2.sql, src/application/market.rs, tests/market_storage.rs. Separate MarketRepository with start_market_run/save_prices_page/finish_market_run/market_snapshot; application ingest_prices. Migration tests create old schema and verify preservation. Test dedup/revisions, atomic pages, wrong query/order, stable coverage, failed runs and reopen.
- [x] Example and integration: examples/market_ingest.rs ingest/query DB MANIFEST SYMBOL START END; fixture subprocess-to-SQLite test in tests/market_end_to_end.rs. README install/run commands. Install isolated pinned Python deps and attempt live daily history; report source failures accurately.
- [x] Independent final review; fix material defects, then cargo test, cargo clippy all-targets -D warnings, cargo fmt --check, both Python plugin suites. Record results below.

## Progress

Plan and wire contract written. Parent handles domain/host/example; independent workers own plugin and storage. Existing branch/isolation choice persists from approved initial implementation.

## Completion evidence

- Implemented market_data v1 domain, host adapter, separate market repository/application, SQLite migration 2 and Python yfinance plugin pinned to 1.7.0.
- Installed dependency into ignored plugins/yfinance/.venv; cache is ignored and plugin-local.
- All 36 Rust tests pass; all-target Clippy with warnings denied and Cargo formatting pass.
- Python suites pass: 20 SEC tests and 12 yfinance tests.
- Independent domain/plugin and integrated storage/migration review reported no material defects.
- Actual Yahoo/yfinance retrieval returned 252 AAPL daily bars for inclusive dates 2024-01-01 through 2024-12-31. Rust ingestion stored them as market run 1 in lugus.sqlite; offline CLI query verified count/status and unverified observed coverage 2024-01-02..2024-12-31.
- Live database migrated from schema 1 to 2 and retained all 1,226 SEC observations.
- No Alpha Vantage API request or credential use. Existing staged work preserved, no commits or publication.

## Implementation notes

MarketRepository uses separate market tables to avoid overloading company filing queries with instrument/date semantics. Market snapshots retain historical revisions and do not select a latest price. Per-run retrieval associations are public. A completed run means ingestion succeeded, while source calendar completeness stays unverified. Source floats and upstream yfinance volume normalization are disclosed in plugin documentation.
