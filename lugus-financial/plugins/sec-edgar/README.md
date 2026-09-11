# SEC EDGAR provider

A separately executed Python 3.10+ provider implementing [Lugus protocol v1](../../docs/protocol/v1.md). Uses [EdgarTools](https://github.com/dgunning/edgartools) 5.57.0 for SEC HTTP retrieval. The host runs `plugin.json` from this directory; Python is never embedded in the Rust host.

## Installation and configuration

Copy this directory to the desired plugin location, then create a virtual environment:

```sh
python3 -m venv /absolute/path/to/sec-edgar/.venv
/absolute/path/to/sec-edgar/.venv/bin/python -m pip install -r /absolute/path/to/sec-edgar/requirements.txt
```

The supplied manifest uses `.venv/bin/python`, resolved relative to the plugin directory. An installed copy can instead use the absolute interpreter path. Keep `args` as `["main.py"]` and select the manifest explicitly in the host. On Windows, use `.venv/Scripts/python.exe`. Existing installations must install the requirements and update their manifest to provider version `0.3.0`.

Use an isolated virtual environment with the pinned requirements. `httpx2` is not supported by this adapter. Protocol initialization and injected-fixture tests work without EdgarTools; a real retrieval with missing or incompatible dependencies returns an actionable `configuration` error.

Send configuration only through `initialize`:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_version":1,"config":{"user_agent":"Lugus Your Name your-email@example.com"}}}
```

Replace the example with your identifying contact information. The provider does not persist configuration or emit it in diagnostics. It requires HTTPS and the identifying User-Agent on every request. Stdout contains only flushed JSON-RPC responses; no batches or notifications are accepted.

## EdgarTools boundary

`edgar_transport.py` adapts EdgarTools's reusable HTTP client to Lugus's transport interface. All five SEC operations use that edge. Lugus retains pure JSON decoding and normalization: high-level library JSON/DataFrame conversions are deliberately avoided because binary floats lose decimal precision and regenerated documents cannot preserve source checksums. This integration does not add standardized statements or new filing extraction capabilities.

Lugus owns retries, cooldowns, byte limits, redirect validation and snapshot pagination. The adapter uses no EdgarTools retry wrappers, so attempts cannot multiply. HTTPS certificate validation is enforced, HTTP/1.1 connections are reused, and redirects are checked before each destination is contacted (at most five hops). A stream returns each network chunk to the caller so slow trickles cannot hide the overall deadline.

EdgarTools's response cache is disabled: successful root queries retrieve new evidence, while Lugus owns durable storage and explicit reuse. Import-time library cache files are isolated in a temporary directory and cleaned up when the process exits. The configured identity is sent as a request header, without calling interactive identity prompts or persisting the identity in library configuration.

## Data and pagination

- `filings.list`: submissions API recent filings plus referenced historical files overlapping the inclusive filing-date range. The source URL points at the archive primary document when supplied, or its filing directory when absent. Acceptance timestamps without an explicit timezone remain null.
- `fundamentals.facts`: Company Facts observations across all returned standard namespaces and units, including concepts without canonical mappings. Binary floating point is never used for source values. Repeated disclosures retain accession identity; duration and instant periods stay distinct.
- `filings.document`: exact document bytes as base64, restricted to HTTPS `www.sec.gov/Archives/edgar/data/` paths, including redirect validation. The default and maximum document limit is 10 MiB; oversize responses fail rather than truncate.

Company identifiers must use `sec:cik` with exactly ten zero-padded ASCII digits, for example `0000320193`. Both list methods filter by **filing date**, not the reporting period, and honor form filters. Cursors are opaque, tied to method and normalized query (including page size), single-use, and process-local. Pagination reuses the fetched snapshot. Starting a new query without a cursor or reinitializing expires the previous snapshot; finish each pagination sequence before starting another. Restart a failed ingestion from a null cursor and use the host's idempotent storage.

Company Facts covers entity-wide, standard-taxonomy facts. Custom taxonomy extensions and dimensional detail are outside this source's coverage. This provider does not derive quarters, convert units, infer missing values, or assemble standardized statements.

## Bounds and errors

One transport limiter is shared by requests in this process: two request starts per second. Each HTTP request permits at most three attempts, a 10-second socket timeout, and a 40-second overall reading/retry budget. Only transient failures and HTTP 429/500/502/503/504 are retried. Retry-After is honored; delays exceeding the bounded retry budget are returned to the caller as typed errors instead of retrying early. HTTP 404, malformed source data, and valid empty results remain distinct.

Each source response is bounded to 32 MiB; historical submissions share that aggregate byte budget. Protocol input/output lines are limited to 32 MiB. The host's process deadline also bounds queries that require multiple historical HTTP requests. A single process cannot coordinate the SEC aggregate limit across other applications or processes sharing a network; operators must coordinate that rate externally.

## Offline verification

```sh
lugus-financial/plugins/sec-edgar/.venv/bin/python -m unittest discover -s lugus-financial/plugins/sec-edgar/tests -v
```

Run from the workspace root. Tests use injected transport fixtures, the installed EdgarTools client with simulated HTTP responses, and an actual JSON-RPC subprocess recovering from a source timeout. They cover exact values and bytes, source checksums, pagination, retries, cooldowns, identity changes, dependency failures, redirects and streaming deadlines. No live SEC access occurs. System Python can run the original fixture tests and skips the EdgarTools-specific tests when the dependency is absent. Live checks require an explicitly configured contact identity and are outside the default test suite.

## Company resolution (capability v1)

The optional `company_resolution: 1` handshake capability adds two operations:

- `company_resolution.search`: `{query: {kind: "name", text: "IBM"}, page_size: 100, cursor: null}` or `{query: {kind: "identifier", identifier: {namespace: "sec:ticker", value: "IBM"}, exchange: null}, page_size: 100, cursor: null}`.
- `company_resolution.lookup`: `{identifier: {namespace: "sec:cik", value: "0000051143"}}`.

Search returns `{items, next_cursor, snapshot, coverage}`. Each candidate contains
its `sec:cik` identifier, source name, aliases (currently empty), reported listings,
source URL, SHA-256 checksum of the exact downloaded bytes, retrieval timestamp,
and match reasons. Listing associations use `sec:ticker` and `sec:exchange`; these
are SEC labels, not market-provider symbols or ISO exchange codes. Lugus owns
catalog identity and ambiguity handling. The plugin never chooses among matches.
Cashtags are parsed by the host; pass `IBM`, not `$IBM`, to ticker search.

Name and ticker search use the SEC ticker/exchange directory. Fields are decoded
by name and malformed rows fail the search. Listings are grouped by CIK. Names
match after whitespace normalization and lowercase conversion; exact names sort
before substrings. Tickers match ASCII case-insensitively after trimming, retaining
punctuation. An optional `sec:exchange` qualifier restricts ticker matches.

Direct CIK search/lookup uses submissions, including registrants absent from the
directory. CIK input accepts 1–10 positive ASCII decimal digits and outputs ten
digits. Lookup checks the returned CIK and rejects unequal ticker/exchange arrays.
Lookup `not_found` remains an error; CIK search represents it as an exhausted empty
page with explicit CIK coverage. Neither path claims complete SEC filer discovery.

Search pages retain a single process-local snapshot and deterministic order.
Continuation pages do not fetch. Cursors are single-use, query/page-size bound,
and expire on another root resolution search or successful reinitialization.
Financial ingestion has a separate pagination session. The existing HTTP byte
limits, SEC URL restrictions, User-Agent, rate limiting and retry policy apply.
Name queries are bounded to 256 UTF-8 bytes; namespace/identifier/cursor values to
128 bytes; page sizes are integers from 1 through 100. Unsupported namespaces,
invalid requests and malformed source data retain separate error kinds.
## Fetch recovery

The transport retains source cooldowns across requests. Long Retry-After values
are returned immediately and remain enforced on later requests. HTTP 401/403
requires checking source access and identifying User-Agent configuration; repeated
unchanged requests are not an automatic recovery. Hard host deadlines still close
the process. See the [recovery report](../../../docs/data-fetch-recovery.md).
