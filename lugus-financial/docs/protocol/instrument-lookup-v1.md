# Instrument lookup capability v1

Adds optional `"instrument_lookup":1` to protocol v1 initialization. A host dispatches
`instrument_lookup.lookup` only when exactly version 1 is advertised. Existing
`market_data:1` daily prices and pagination retain their semantics.

Request:

```json
{"jsonrpc":"2.0","id":2,"method":"instrument_lookup.lookup","params":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"}}}
```

Successful result (checksum below is illustrative):

```json
{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"issuer_name":"Apple Inc.","ticker":"AAPL","exchange":{"namespace":"yahoo:exchange","value":"NMS"},"kind":"equity","issuer_identifiers":[],"source_url":"https://finance.yahoo.com/quote/AAPL/","source_checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","retrieved_at":"2026-09-10T00:00:00Z"}
```

The returned instrument must come from source metadata and exactly match the request
namespace/value. Successful prices and request echoes do not establish identity.
Issuer name, ticker, exchange and kind are optional: absent/null means unknown, and
serialization retains nulls. A present field of the wrong type, empty text or invalid
value fails. Kind is `equity` or `other`; unknown type is null. Issuer identifiers are
namespaced source-reported IDs, capped at 16; an empty array means none available.
This capability does not associate an instrument with a company or claim a verified
security identity. That decision belongs to the application's evidence policy.

Unknown fields are rejected at every lookup nesting level, including identifiers.
Identifier namespace/value and ticker are at most 128 UTF-8 bytes; issuer name is at
most 1,024; source URL at most 4,096. Text must contain non-whitespace content and no
Unicode control characters. Checksum is exactly 64 hexadecimal characters and records
SHA-256 of the adapter's documented source projection. Retrieval time is a UTC timestamp.
The host's existing bounded response line and deadline apply before typed decoding.
Invalid result schema, source identity or metadata closes the plugin with a safe
`protocol` failure; unsupported capability/version does not dispatch; invalid requests
fail before IO. Cancellation retains the existing process guard/reaping behavior.

## Yfinance adapter 0.2.0

The dependency remains pinned to **yfinance 1.7.0**. Its `Ticker.get_info()` cannot be
used for independent identity evidence: `_fetch_info()` overwrites `symbol` with the
requested ticker. Instead, the adapter primes the public chart path with
`Ticker.history(period='5d', interval='1d', auto_adjust=False, back_adjust=False,
repair=False, actions=False, timeout=10)` and exceptions enabled, then reads only four
keys from public `Ticker.get_history_metadata()`:

| Source chart.meta field | Projection |
| --- | --- |
| `symbol` | Mandatory source instrument value and ticker |
| `longName` | Optional issuer name |
| `exchangeName` | Optional `yahoo:exchange` value |
| `instrumentType` | `EQUITY` → `equity`; other supplied text → `other`; missing → null |

The native instrument namespace is the adapter's documented `yahoo:symbol` namespace.
No CIK, LEI or share-class identifier is manufactured; `issuer_identifiers` is empty.
The source URL is `https://finance.yahoo.com/quote/{percent-encoded-source-symbol}/`.
Checksum covers exactly the four keys above, with missing values represented as null,
serialized with sorted keys, compact separators and unescaped UTF-8 (`ensure_ascii=False`),
then SHA-256 hashed. The fields are bounded and validated before serialization; unrelated
source fields are excluded. Retrieval time comes from the injected host clock after IO.
The hash identifies this source projection, not the full Yahoo response or price data.

The lazy metadata map is never enumerated: `tradingPeriods` could trigger another
request. Priming history explicitly avoids the exception swallowing in metadata's
initial fetch. Source errors remain safe typed `rate_limited`, `timeout`, `not_found`,
`malformed_data` or `unavailable` failures. Library stdout is redirected to stderr.
There is no adapter retry or fallback. The SDK may do internal request/cookie handling;
the host process deadline is the outer bound. The SDK can fail during history parsing
if required chart fields (including `instrumentType`, currency or timezone) are absent;
that produces a typed failure rather than invented metadata. Missing optional name or
exchange survives as null. A lookup leaves any active daily cursor unchanged.

Source reviewed locally and against primary upstream code:
[1.7.0 quote implementation](https://github.com/ranaroussi/yfinance/blob/1.7.0/yfinance/scrapers/quote.py),
[1.7.0 history implementation](https://github.com/ranaroussi/yfinance/blob/1.7.0/yfinance/scrapers/history.py).

## Retained evidence

Financial SQLite schema v5 adds only `instrument_observations`. Every successful save
appends a new row with the exact provider identity, request, response and recording time,
even for an identical repeated retrieval. Update/delete triggers reject mutations.
Provider instance, plugin ID and plugin version are all part of the evidence identity;
these fields are bounded to 128 UTF-8 bytes under this capability. Old schema v4 evidence,
repository identity, market data and run chronology are unchanged.

`InstrumentRepository::save_instrument_observation` validates before writing and returns
`InstrumentObservation`. `SqliteRepository::bounded_instrument_observation` reads exactly
one observation by provider and ID, returns `not_found` for another provider, and checks
`ReadLimits` before loading strings. The byte budget counts stored UTF-8 provider JSON,
request JSON, payload JSON and recording time plus 8 bytes for the observation ID; it
does not measure Rust allocator overhead or the caller's eventual output. Preflight and
decoding share one SQLite read transaction, with no history scans or network effects.
Callers must also bound their output serialization.
