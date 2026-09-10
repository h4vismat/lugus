# SEC EDGAR provider

A separately executed Python 3.10+ provider implementing [Lugus protocol v1](../../docs/protocol/v1.md). Uses only Python's standard library. The host runs `plugin.json` from this directory; Python is never embedded in the Rust host.

## Installation and configuration

Copy this directory to the desired plugin location, then create a virtual environment:

```sh
python3 -m venv /absolute/path/to/sec-edgar/.venv
```

Set `command` in your installed copy of `plugin.json` to the absolute path `/absolute/path/to/sec-edgar/.venv/bin/python`. Keep `args` as `["main.py"]`. Alternatively, the supplied manifest uses `python3` from the host's PATH without third-party dependencies. Select the manifest explicitly in the host.

Send configuration only through `initialize`:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_version":1,"config":{"user_agent":"Lugus Your Name your-email@example.com"}}}
```

Replace the example with your identifying contact information. The provider does not persist configuration or emit it in diagnostics. It requires HTTPS and the identifying User-Agent on every request. Stdout contains only flushed JSON-RPC responses; no batches or notifications are accepted.

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
python3 -m unittest discover -s plugins/sec-edgar/tests -v
```

Run from the repository root. Tests use injected transport fixtures and an actual JSON-RPC subprocess; no live SEC access occurs. Live checks require an explicitly configured contact identity and are outside the default test suite.
