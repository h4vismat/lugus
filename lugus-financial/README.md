# lugus-financial

Provider-independent financial ingestion for Lugus. Rust owns capability contracts, validation, process lifecycle and SQLite persistence. Separately installed trusted plugins provide financial data through a versioned JSON-RPC process protocol.

Python plugins provide SEC EDGAR filings/structured fundamentals and yfinance daily market data through independent capabilities. Other plugins can implement either capability, use different languages, or introduce new identifier and taxonomy namespaces. The host does not embed Python or link to vendor SDKs.

## Data model

- Companies have namespaced identifiers. The initial workflow accepts a zero-padded SEC CIK, not a ticker.
- Facts retain source taxonomy/concept, exact decimal strings, units, instant or duration periods, accession number, filing date and retrieval timestamp.
- Only explicitly recognized concepts receive canonical metric mappings. Unknown concepts remain available. Statement assembly and derived quarter calculations are not performed.
- Reported SEC fundamentals never pass through binary floats. Market prices from yfinance already originate in binary-float data frames and are labelled `binary_float_source`; their decimal representations are retained without further rounding. Do not use SQLite REAL casts for financial arithmetic.
- Separate disclosures and changed source observations remain available. Re-fetching identical data creates a new ingestion association without duplicating observations.
- Local reads never trigger network calls. Partial/failed runs remain visible in query results; a snapshot is evidence history, not a single reconciled current statement.

## Architecture

`domain` contains pure data/validation/mapping functions; `capabilities` contains independent provider traits; `plugin` adapts trusted external processes; `storage` isolates SQLite; `application` coordinates ingestion. Plugin identity and provider-instance identity are separate. Configuration is sent to the process at initialization and is not persisted as provenance.

A plugin declares a command plus argument array in a local manifest. The host starts it without a shell, negotiates capability versions, bounds response size and time, and terminates invalid or unresponsive children. Call `close().await` to reap explicitly. Python and other runtimes are installed by the user; there is no plugin installer or sandbox in this slice.

See [protocol v1](docs/protocol/v1.md), the [SEC plugin instructions](plugins/sec-edgar/README.md), and the [approved architecture](docs/superpowers/specs/2026-09-09-financial-ingestion-design.md).

## Run the first workflow

From this crate directory, install Python 3.10 or newer and use the included manifest (the plugin has no third-party Python dependencies). To install the plugin elsewhere, copy its directory and supply that manifest path. To use a virtual environment, set the manifest's `command` to its Python executable.

```sh
export LUGUS_SEC_USER_AGENT='Lugus YourName your-email@example.com'
cargo run -p lugus-financial --example sec_ingest -- ingest ./lugus.sqlite ./plugins/sec-edgar/plugin.json 0000320193 2023-01-01 2024-12-31
cargo run -p lugus-financial --example sec_ingest -- query ./lugus.sqlite ./plugins/sec-edgar/plugin.json 0000320193 2023-01-01 2024-12-31
```

Replace the contact identity with your own. `ingest` explicitly performs live SEC requests. `query` reads SQLite without launching a plugin or requiring the environment variable. The JSON snapshot retains all matching observations and run statuses, rather than choosing a latest value.

Use a selected filing's `source_url` to store its primary document:

```sh
cargo run -p lugus-financial --example sec_ingest -- document ./lugus.sqlite ./plugins/sec-edgar/plugin.json 'https://www.sec.gov/Archives/edgar/data/320193/000032019323000106/aapl-20230930.htm'
```

The command returns a SHA-256 checksum; Rust callers use `Repository::stored_document` for offline bytes and `SqliteRepository::document_observations` for source history. `SqliteRepository::run_observations` exposes retrieval associations, including repeated observations at different retrieval times. Provider identity includes instance, plugin ID and version; retain that identity when querying older plugin-version history.

Request a smaller filing-date range if an initial history fetch exceeds the host deadline. A failed refresh retains committed pages and its failed status. Repeating the command starts pagination again and deduplicates identical observations. A process interrupted before finalization leaves a visible `running` run.

## Daily market data with yfinance

Install the separately executed Python plugin from this crate directory:

```sh
python3 -m venv plugins/yfinance/.venv
plugins/yfinance/.venv/bin/python -m pip install -r plugins/yfinance/requirements.txt
```

The manifest uses its local `.venv/bin/python`. No API key or `.env` access is needed.

```sh
cargo run -p lugus-financial --example market_ingest -- ingest ./lugus.sqlite ./plugins/yfinance/plugin.json AAPL 2024-01-01 2024-12-31
cargo run -p lugus-financial --example market_ingest -- query ./lugus.sqlite ./plugins/yfinance/plugin.json AAPL 2024-01-01 2024-12-31
```

The first command explicitly fetches daily history; the second reads offline without launching Python. A `yahoo:symbol` identifies a provider's instrument symbol, not a durable company or a CIK. Both requested dates are inclusive, and returned dates refer to the exchange-local trading day.

Bars retain provider-native OHLC, volume, optional adjusted close, source currency/timezone, retrieval time and precision. `source_reported` does not imply split-unadjusted, as-traded prices. Automatic OHLC adjustment and repair are disabled. Missing/invalid returned rows fail explicitly instead of becoming synthetic prices. See the [plugin notes](plugins/yfinance/README.md) for upstream transformations that precede Lugus validation.

`coverage` records observed first/last dates and remains `unverified`: successful retrieval does not establish that every expected trading session is present. The offline snapshot preserves all bar revisions, run statuses and retrieval associations; it does not silently select a latest price or mix providers. Consumers can query those observations through `MarketRepository` and ingest through `application::market::ingest_prices`.

Opening a database applies migration 1 → 2 transactionally, adding market tables while retaining existing SEC evidence. The yfinance adapter uses a pinned dependency and an isolated cache; it is an unofficial integration intended for personal research. [yfinance documentation](https://ranaroussi.github.io/yfinance/)

## Verification

From the workspace:

```sh
cargo test -p lugus-financial
cargo clippy -p lugus-financial --all-targets -- -D warnings
cargo fmt -p lugus-financial -- --check
python3 -m unittest discover -s lugus-financial/plugins/sec-edgar/tests -v
python3 -m unittest discover -s lugus-financial/plugins/yfinance/tests -v
```

Rust integration tests launch `python3`; install it on PATH before running tests. Tests use local fixtures and temporary SQLite databases. Live access is opt-in and needs an identifying SEC User-Agent with contact information.

## Source coverage

SEC Company Facts exposes standard-taxonomy, entity-wide facts rather than every fact in a filing. The plugin preserves what this source returns, including unmapped and IFRS concepts. Custom extensions and dimensional XBRL extraction are outside the initial scope. [SEC API documentation](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)

One plugin instance throttles its own SEC requests. Multiple processes or other applications sharing network access must coordinate their aggregate request rate. [SEC fair-access guidance](https://www.sec.gov/about/developer-resources)

Deferred: Alpha Vantage integration, automatic market-instrument binding, intraday data, background scheduling, agent tool integration, full statement assembly, and cross-provider reconciliation.

## Company resolution and offline catalog

The SEC plugin (`0.2.0`) now advertises `company_resolution:1`. The generic Rust
`CompanyResolutionProvider` port also supports future plugins. Search returns sourced
candidates; the host retains full provider identity, raw-source checksums, repeated
retrievals, and stable local company IDs. It never joins entities by name or assumes
an SEC ticker is a Yahoo instrument identifier.

From the workspace root, with `LUGUS_SEC_USER_AGENT` configured as above:

```sh
cargo run -p lugus-financial --example company_resolve -- resolve ./financial.db ./lugus-financial/plugins/sec-edgar/plugin.json '$IBM'
cargo run -p lugus-financial --example company_resolve -- resolve ./financial.db ./lugus-financial/plugins/sec-edgar/plugin.json '$PLTR'
cargo run -p lugus-financial --example company_resolve -- lookup ./financial.db ./lugus-financial/plugins/sec-edgar/plugin.json 51143
cargo run -p lugus-financial --example company_resolve -- query ./financial.db '$IBM'
cargo run -p lugus-financial --example company_resolve -- history ./financial.db 1
```

Quote cashtags in a shell to prevent environment-variable expansion. `$IBM` is an
explicit ticker search; bare `IBM` tries exact ticker first and falls back to name
search only after a successfully exhausted empty result. Failures and partial
searches do not trigger fallback. Exact CIK input also supports `sec:cik:51143`.
The CLI records the provider instance as `sec-edgar:local`.

`query`, `history`, `select`, and `selection` are offline. Use run and observation IDs
from a candidate result to persist an explicit choice, then reopen its returned
selection ID:

```sh
cargo run -p lugus-financial --example company_resolve -- select ./financial.db 1 1
cargo run -p lugus-financial --example company_resolve -- selection ./financial.db 1
```

A choice validates run membership and freezes its source scope and run status;
it does not rewrite an ambiguous or partial search as an automatically resolved run.
 Name matches remain candidates. A unique exact
identifier resolves only within its source scope and absent historical catalog
conflicts. Search directory coverage is limited; direct CIK lookup can retrieve
registrants absent from that directory. Catalog history retains source associations
rather than asserting that omitted listings have ceased to exist.

The public `resolution::application` functions coordinate bounded searches and
persistence. `resolution::catalog::CatalogRepository` exposes offline results and
history. See [company-resolution protocol](docs/protocol/company-resolution-v1.md).

## Observation selection

`selection::SelectionRepository` reads exact ingestion-run membership, preserving
retrieval associations and persistent repository identity. Pure `select_daily` and
`select_facts` functions produce versioned manifests and explicit conflicts.

Default selection uses the latest initiated completed run whose requested scope
contains the view query, from one full provider identity. New failed/running
refreshes remain visible while the previous completed dataset is selected. Completed
source responses may still have partial/unverified source coverage, which remains
explicit. No chart combines price revisions from separate source snapshots.

Fundamental selection groups the exact concept, unit, and reporting period, then
selects the latest filed disclosure in each group. Numerically equivalent decimal
strings retain all supporting references; differing values filed on the same date
remain a conflict. Annual, quarterly, and year-to-date durations are not combined.
No public-as-of guarantee, currency conversion, quarter derivation, or full statement
assembly is introduced. Existing durable review evidence remains unchanged.

Inspect a stored market dataset without fetching:

```sh
cargo run -p lugus-financial --example observation_select -- ./financial.db local yfinance 0.1.0 yahoo:symbol AAPL 2024-01-01 2024-12-31
```

Use the actual instance/plugin/version recorded by your ingestion. Inspect a reported
instant metric from an existing SEC dataset:

```sh
cargo run -p lugus-financial --example fundamentals_select -- ./financial.db local sec-edgar 0.2.0 51143 us-gaap Assets USD 2023-01-01 2026-09-10 latest-instant
```

Other modes are `instants` and `durations`. The scope must be contained by a completed
facts ingestion run. An empty response includes run diagnostics instead of silently
shortening scope. Old SEC observations retain their original `0.1.0` identity;
select that version explicitly when inspecting them.

Financial schema upgrades now apply additive v3 catalog and v4 selection metadata.
Existing observation payloads and fingerprints stay intact. New ingestion chronology
is monotonic within the repository; migrated cross-capability timestamp ties receive
a deterministic local order, not an inferred source-publication order.

### Bounded application reads

The application runtime uses `SqliteRepository::bounded_financial_run` and
`bounded_market_run` with a full provider identity, exact run ID, and
`storage::bounded::ReadLimits`. They check row counts and UTF-8 payload/metadata
bytes in the same SQLite snapshot before materializing evidence. Existing
all-history selection APIs remain available for library callers.

`bounded_scoped_resolution` retains exact run candidates and the existing
historical identity-conflict outcome. That policy can consult the full catalog,
so its preflight conservatively bounds all catalog retrievals. A catalog exceeding
the configured budget returns `LimitExceeded`; it is never truncated to hide a
conflict. `bounded_document` checks the exact provider/source/retrieval association
and original checksum before returning bounded bytes.

Workspace ownership, durable frozen dataset IDs, and view receipts belong to
[`lugus-app`](../lugus-app/README.md). Financial repository IDs or raw run IDs alone
do not grant workspace access.

### Resolution/selection verification record

On 2026-09-10, all 145 workspace/all-target Rust tests passed, as did
strict workspace Clippy, formatting, 31 SEC Python tests and 12 yfinance Python tests.
Independent review findings were fixed and re-reviewed. The company CLI resolve,
lookup, offline query/history, explicit select, and reopened selection flows passed
using a real synthetic provider process. Live SEC data retrieval was not exercised
in this implementation verification.
