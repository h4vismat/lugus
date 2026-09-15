# Historical prices v1

Optional capability `historical_prices: 1`, JSON-RPC method `historical_prices.daily`.
Ordinary `market_data.daily` is unchanged. Requests contain instrument, inclusive
start/end, a common normalization anchor, null/string cursor and page_size 1–200.

Each immutable snapshot contains every calendar date from the prior trading
session before start through anchor. A null market_close denotes a scheduled
closure. A session with null close denotes missing or incomplete price evidence.
Never infer a closure from an absent bar. Pages retain the same manifest and
advance consecutive calendar dates. The final page ends at anchor.

The manifest pins requested range, coverage start, last completed session,
currency, timezone, calendar/version, normalization version, source basis,
retrieval time and source completeness. Version 1 supports USD NYSE/NASDAQ
calendars from pandas_market_calendars 5.4.0. Provider identity is attached by
the trusted host when storing the snapshot.

Each date retains source close, derived trading-date close, factor to anchor,
optional positive integer split ratio, unsupported-action reason and source URL.
The bundled Yahoo adapter reverses splits strictly after each historical bar
through its current retrieval anchor, including splits after the chart end.
It does not apply dividend adjustments. Index ^SP500TR levels are already total
return and must remain unchanged. Missing evidence never becomes a zero price.

Ingestion completion does not promote source completeness. Consumers must
reconcile source actions against the ledger and reject incompatible share bases.
