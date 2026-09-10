# Company resolution capability v1

Optional `"company_resolution":1` in the protocol-v1 capability handshake. The SEC plugin implements this alongside filings/fundamentals beginning with plugin version `0.2.0`. Other providers may implement the same interface for other namespaces; host catalog IDs are never supplied by plugins.

`company_resolution.search` parameters:

```json
{"query":{"kind":"identifier","identifier":{"namespace":"sec:ticker","value":"IBM"},"exchange":null},"page_size":100,"cursor":null}
```

A name query is `{"kind":"name","text":"International Business Machines"}`. Optional exchange qualifier uses `{"namespace":"sec:exchange","value":"NYSE"}`. Exact `sec:cik` search uses ten-digit normalized values. The host entry parser accepts `$IBM` (explicit ticker), bare `IBM` (ticker then name fallback after an exhausted no-match), and `sec:cik:51143` (normalized CIK). Cashtag punctuation is not converted into another provider's notation.

Result:

```json
{"items":[{"identifier":{"namespace":"sec:cik","value":"0000051143"},"name":"INTERNATIONAL BUSINESS MACHINES CORP","aliases":[],"listings":[{"ticker":{"namespace":"sec:ticker","value":"IBM"},"exchange":{"namespace":"sec:exchange","value":"NYSE"}}],"source_url":"https://www.sec.gov/files/company_tickers_exchange.json","source_checksum":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","retrieved_at":"2026-09-10T00:00:00Z","match_reasons":["exact_identifier"]}],"next_cursor":null,"snapshot":"opaque-snapshot","coverage":"SEC company ticker/exchange directory snapshot; not all SEC filers"}
```

The checksum above is illustrative; actual SEC results contain SHA-256 of the exact source response bytes. Provider identity and host recording time are attached outside the plugin response by Lugus. Name matches use one best reason: `exact_name` or `name_substring`; identifier matches use `exact_identifier`.

`company_resolution.lookup` parameters: `{"identifier":{"namespace":"sec:cik","value":"0000051143"}}`. Returns one candidate in the same shape with empty `match_reasons`. SEC lookup uses submissions data, validates the returned CIK, and can find registrants absent from the ticker directory. Unsupported identifier namespaces return `unsupported`; a supported CIK without submissions returns `not_found`.

Search queries have page sizes 1..100, name text up to256 UTF-8 bytes, identifier namespace/value up to128 bytes. The host accepts cursors up to1024 bytes; the SEC adapter accepts its own opaque tokens up to128 bytes. Source names are bounded to1024 bytes, aliases to100 entries, listing associations to1000, source URL to4096, checksum to64 hex characters. Empty/control-character fields and malformed response/match identity are rejected. Existing process line/time limits also apply.

Continuation pages retain snapshot and coverage values. Cursors are tied to the normalized query and initialized process and expire on a new root resolution search or reinitialization. Source or protocol errors are never converted into successful empty searches. Exact CIK search may yield an exhausted empty scoped page for a confirmed source not-found; lookup retains the explicit not-found error.

The catalog imports pages atomically and preserves partial/failed runs, candidate revisions, provider identity, original source checksums and repeated retrieval associations. An incomplete search cannot automatically resolve a single candidate. A unique exact ticker match resolves only within searched scope and absent known catalog conflicts; name results remain candidates. Ticker-to-company resolution does not create a Yahoo instrument mapping.
