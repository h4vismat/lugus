# Market-data capability v1

Extends protocol/v1.md with optional `"market_data":1` capability. No change to SEC methods. Example plugin identity `yfinance`, version `0.1.0`. Initialize config is `{}`; no API key required.

Method `market_data.daily`, query:
```json
{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"start":"2024-01-01","end":"2024-12-31","cursor":null,"page_size":100}
```
Dates inclusive. page_size 1..1000. Yfinance requires yahoo:symbol; Rust contract accepts arbitrary nonempty namespaces. Symbol is passed explicitly, not inferred from company identity.

Result PricePage:
```json
{"items":[{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"date":"2024-01-02","open":"187.15","high":"188.44","low":"183.89","close":"185.64","volume":82488700,"adjusted_close":"184.73","currency":"USD","exchange_timezone":"America/New_York","price_basis":"source_reported","precision":"binary_float_source","source_url":"https://finance.yahoo.com/quote/AAPL/history/","retrieved_at":"2026-09-09T00:00:00Z"}],"next_cursor":null,"coverage":{"first_date":"2024-01-02","last_date":"2024-12-31","completeness":"unverified"}}
```

OHLC and adjusted_close use existing plain exact Decimal string grammar; these strings cannot restore precision lost upstream. volume is a nonnegative integer bounded by u64. adjusted_close is nullable. currency and exchange_timezone come from source metadata. SourceReported basis means provider native OHLC; do not relabel as raw as-traded or silently apply dividend adjustments. Precision supports binary_float_source or decimal_source (future adapters).

Coverage endpoints describe the WHOLE fetched snapshot (including across pages), not just the current page. Null endpoints require both null and no returned records. completeness enum: unverified, partial, complete; yfinance always returns unverified because no trading-calendar completeness is established. No synthetic weekend/holiday bars. Items are sorted strictly ascending, with at most one bar per date, and lie inside query and coverage. Cursor session rules/error envelope/line limit are those in v1.md.

Yfinance options: interval='1d', start=query.start, end=query.end+1 day, auto_adjust=False, back_adjust=False, repair=False, rounding=False, actions=False, keepna=True, timeout=10; configure exceptions to propagate for the pinned library version. Metadata currency and exchangeTimezoneName required. Preserve the returned local trading date. Reject all invalid rows, duplicates, negative or nonfinite values, nonintegral volume and OHLC inconsistencies. yfinance dataframes are converted using typed tuples rather than iterrows coercion.
