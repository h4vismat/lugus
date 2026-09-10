# Daily market data with a yfinance plugin

Status: user approved switching the first market-data plugin to yfinance. Extends the established trusted-process and SQLite architecture.

## Scope

Daily historical OHLCV, optional adjusted close, source metadata, revision history, offline reads and a runnable example. Initial identifiers use `yahoo:symbol` (for example AAPL); these identify provider symbols, not durable companies. No ticker-to-CIK inference. No Alpha Vantage requests or API key use.

## Contract

Independent MarketDataProvider capability, advertised as market_data version 1; operation market_data.daily. PriceQuery has instrument, inclusive start/end, cursor and page_size. Separate PriceBar and PricePage types; company filings queries remain unchanged. See docs/protocol/market-data-v1.md for the exact JSON.

Use source_reported price basis. yfinance history is called with auto_adjust=false, back_adjust=false, repair=false, rounding=false, actions=false, keepna=true and daily interval. Retain adjusted close separately. Yahoo source OHLC must not be described as guaranteed raw as-traded prices. Decimal serialization preserves the representation received from yfinance, which already uses binary floats; precision metadata makes this limitation explicit. Reject invalid/nonfinite/negative prices, fractional/negative volume and contradictory OHLC ranges without binary-float comparisons in Rust.

The inclusive end date is converted to yfinance's exclusive end by adding one day. Use the exchange-local date of each bar and metadata currency/timezone. Do not guess USD. Sort ascending by date; reject duplicate dates or out-of-query records. Empty data is not silently interpreted as a valid no-trading interval: the plugin reports not_found when source yields no prices without an explicit error. Missing/invalid rows fail instead of silently dropping them.

Each page includes the whole fetched snapshot's first/last observed dates and completeness=unverified. Observed endpoints do not establish full date/calendar coverage. Process-local cursors bind method and entire query, expire on initialization/new root query, and reuse one downloaded snapshot. Page validation and ingestion enforce progress, ordered dates, stable coverage and query correspondence.

## Persistence

SQLite migration 1 to 2 adds separate market runs, price observations and retrieval associations. Existing filings/facts/documents remain untouched. MarketRepository is separate from the filings/fundamentals Repository. Retain provider instance and plugin version. Price fingerprints exclude retrieval time and include source semantics; changed bar content creates a revision, identical refreshes reuse observations. Atomic pages store records, retrieval associations, coverage and next cursor together. Failed/running states stay visible; offline snapshots expose historical observations and runs. Restart ingestion from page one.

## Python plugin

Installed separately in a local virtual environment, with a pinned yfinance dependency. Pure row normalization and protocol dispatch are testable with injected history transport. The library is imported lazily; initialize can validate protocol without making a network call. Suppress library stdout to protect protocol framing, bound input/output sizes, configure library cache inside plugin-local .cache and respect the host deadline. Typed exceptions distinguish source errors, rate limiting, configuration and malformed responses; never treat a download failure as successful empty coverage. No retries for rate limits and no implicit fallback provider.

## Verification

Domain validation tests; process adapter negotiation/pagination/errors; migration preserving a version-1 database; idempotent refresh and corrections; atomic rollback, query/date/order validation, partial failure and offline reopening. Python fixture tests for timezones, end inclusivity, decimals/volume, missing data, metadata, errors and cursor binding. A live AAPL historical query follows offline checks if network access is available; record actual results, without claiming calendar completeness.

Sources checked September 9, 2026: https://ranaroussi.github.io/yfinance/ and https://raw.githubusercontent.com/ranaroussi/yfinance/main/yfinance/scrapers/history.py (date and adjustment options). yfinance is unofficial and intended for personal/research use.
