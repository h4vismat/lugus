# Lugus financial ingestion — proposed design

Status: approved by the user and implemented. See the implementation plan for verification evidence and protocol v1 for the concrete wire contract.

## Objective and agreed scope

Build the data foundation for an investment thesis tracker with future agent consumers. Start with company filings and structured fundamentals for US-listed companies. Use a separately installed Python provider plugin, trusted by the user, and local SQLite persistence. Keep capability contracts independent of vendor, plugin language, and storage engine. Market data and Brazilian providers come later.

## First usable workflow

A caller supplies a namespaced company identifier (initially `sec:cik`), an explicit filing-date range, and a configured provider instance. Lugus starts the SEC plugin, validates its capabilities, retrieves filing metadata and structured facts, and persists validated observations. The caller can query stored results offline and retrieve a selected filing's primary document on demand.

CIK is the initial input to avoid making ticker resolution a prerequisite. A company is distinct from its exchange listings. Ticker lookup can be added as a separate operation; tickers are not durable company identity.

Success means one company can be ingested, queried after restarting the host, and refreshed without duplicating identical facts. Distinct disclosures and changed observations remain traceable to their sources.

## Architecture

1. Domain: provider-independent identifiers, facts, filings, document references, requests, results, and typed errors.
2. Capabilities: independent Rust interfaces for filings and fundamentals. A provider can implement either or both. Market data remains a separate future capability.
3. Plugin host: launches a configured executable, negotiates versions, validates responses, and adapts external capabilities to the Rust interfaces.
4. Application: coordinates explicit provider selection, fetching, validation, persistence, and offline queries.
5. Storage: a repository contract implemented with SQLite and versioned migrations.
6. SEC Python plugin: source-specific HTTP access and decoding into the common fact and filing envelope.

Prefer immutable values and pure functions for decoding, validation, normalization, identity calculation, and metric mapping. Process, clock, network, and database access remain at the edges. Avoid a shared mutable global provider registry.

Definitions: a capability is a data contract; a plugin supplies executable implementations; a provider instance is one configured use of a plugin. Persist plugin identity and version alongside provider instance identity.

## Plugin contract

Use JSON-RPC 2.0 over standard input/output with one UTF-8 JSON object per line. Reserve standard output for protocol messages and standard error for diagnostics. Protocol versioning is separate from plugin versioning.

An explicitly selected local manifest declares plugin ID, plugin version, supported protocol major version, and executable plus argument array. Resolve relative executable paths against the manifest directory. Spawn without a shell. The initial host does not scan directories or install dependencies automatically; document installation in a Python virtual environment.

Proposed methods:

- `initialize`: negotiate protocol version and advertise supported capability versions and operations.
- `filings.list`: company, inclusive filing-date range, optional form filters, opaque cursor, and bounded page size; returns filing metadata and a next cursor.
- `filings.document`: a source document reference obtained from filing metadata; returns bounded base64 document bytes, media type, source URL, and retrieval timestamp.
- `fundamentals.facts`: company, inclusive filing-date range, opaque cursor, and bounded page size; returns structured reported facts and a next cursor. This range selects disclosure dates, not fiscal periods.

Start with one outstanding request per provider process. Enforce configurable deadlines and response/document size limits. Reject incompatible versions and unadvertised operations before dispatch. On protocol failure or timeout, terminate and reap the child; allow a subsequent explicit operation to restart it. Drain standard error with bounded retention to prevent deadlocks. No automatic restart loop.

Use typed errors for invalid requests, unsupported operations, configuration, rate limiting (with optional retry delay), unavailable sources, missing data, malformed source data, protocol errors, timeouts, and persistence failures. An empty successful result differs from a failed request. Plugin configuration is supplied at initialization, excluded from persisted provenance and routine logs.

## Financial facts and provenance

The canonical ingestion unit is a reported fact, rather than an eagerly assembled income statement or balance sheet. Each fact carries:

- Company identifier namespace and value.
- Original taxonomy namespace and concept name, including an optional label.
- Exact decimal value represented as a validated string, plus its original unit.
- Period as either an instant date or explicit start/end dates.
- Source filing identifier and form, filing date, and optional fiscal-year/period labels.
- Source URL, retrieval time, provider instance, and plugin version.
- Optional canonical metric identifier with a mapping version.

The SEC adapter must parse decimals without first converting through binary floating point. SQLite stores exact values as text. Reporting dates use dates; retrieval and known publication instants use UTC timestamps. Do not manufacture precise publication times from filing dates. Fiscal labels do not replace actual period boundaries.

Preserve all structured facts returned by the selected source, including those without canonical mappings. Mapping rules are pure, namespace-aware functions owned by Lugus and registered explicitly; a future taxonomy mapping must be addable without changing core fact storage or protocol. A plugin for another market can immediately ingest namespaced facts, while comparable metrics may require additional mapping rules.

Begin canonical mappings with a small explicit set: assets, liabilities, equity, net income, and operating cash flow. Retain unit and period semantics. Do not infer missing values, silently combine units, derive standalone quarters from year-to-date facts, or promise complete standardized statements in this slice.

Repeated disclosure of a prior period is not automatically a restatement. Retain filing identity and original observations; never overwrite a prior disclosure merely because a later filing reports the same concept and period. Historical queries distinguish when a filing was published from when Lugus observed its data. This supports evidence history but does not reconstruct historical source snapshots that Lugus never collected.

## Persistence and refresh behavior

SQLite tables cover provider instances (nonsecret metadata), companies/external identifiers, ingestion runs, filing observations, fact observations, document content, and ingestion-to-observation associations.

Use stable source identities and canonical content fingerprints for idempotency. An identical observation is reused and linked to each retrieval run; changed content creates a new observation. Preserve the source record fields needed to explain a fact and record the mapping version. Do not use a JSON field dump or unstable object ordering as an implicit identity algorithm.

Fetch and validate bounded pages before writing. Commit each page and its cursor atomically. Mark a run complete only after all pages succeed. A failed run exposes its status and successful prior pages; application query results explicitly identify incomplete ingestion rather than treating it as complete coverage. Resume a persisted cursor only when the same plugin version guarantees its validity; otherwise restart using idempotent writes.

Store primary documents as content-addressed SQLite BLOBs with checksums and metadata for this initial local implementation. A configurable size limit produces an explicit error. Abstract document storage separately if later volume justifies filesystem/object storage.

Provider refresh and offline reads are separate application operations. There is no implicit network request on a local query and no silent merging or fallback across providers.

## SEC adapter

Use the submissions API for filing metadata, including referenced older submission files when needed for the requested range. Use Company Facts for structured facts, filter by filing date, and preserve accession links even if corresponding filing metadata has not yet been ingested. Retrieve the selected primary document from SEC filing archives on demand. The adapter caches a fetched company response within the current pagination session rather than refetching it for every page.

The Company Facts API covers standard-taxonomy, entity-wide facts; custom extensions and dimensional detail are outside this source's coverage. Therefore the initial plugin does not promise complete XBRL coverage. Retain standard taxonomy namespaces without assuming every company reports using US GAAP.

Require an identifying SEC User-Agent with contact information in plugin configuration. Use a conservative request rate, bounded retry/backoff for transient errors and rate limits, and honor retry guidance when supplied. Requests for one SEC provider instance share a limiter. Document that multiple processes or applications behind the same network need coordinated limits; an individual plugin cannot enforce an aggregate external limit.

Sources checked September 9, 2026:

- [SEC EDGAR APIs](https://www.sec.gov/search-filings/edgar-application-programming-interfaces): submissions, historical pagination, Company Facts, and taxonomy/context coverage.
- [SEC developer resources](https://www.sec.gov/about/developer-resources): fair access, including the aggregate ceiling of 10 requests per second per user.

## Verification

- Pure-function tests for instant versus duration periods, exact decimals, missing values, unmapped concepts, and deterministic identity.
- SEC decoding fixtures for older filing pages, repeated disclosures, differing units, IFRS facts, absent optional fields, malformed payloads, and no available facts.
- Rust-to-Python process tests for negotiation, capability discovery, successful pagination, structured errors, timeouts, malformed output, and child cleanup.
- Temporary SQLite integration tests for migrations, atomic page writes, idempotent refresh, changed observations, failed-run visibility, and reopening for offline reads.
- An end-to-end fixture test ingests filing metadata and facts and retrieves a primary document without network access. A live SEC smoke test is opt-in and requires configured contact identity.

## Implementation sequence

1. Domain types, typed errors, and capability contracts; expose public modules and replace the placeholder library test.
2. Document and implement the minimal process protocol, with a fixture Python plugin.
3. SQLite migrations, repository implementation, and application fetch/read functions.
4. SEC plugin, fixture coverage, and selected primary-document retrieval.
5. Runnable Rust example and Python installation/configuration documentation; opt-in live verification.

Proposed layout: `src/domain/`, `src/capabilities/`, `src/plugin/`, `src/storage/`, `src/application.rs`, `plugins/sec-edgar/`, `tests/`, and `examples/`. Keep changes inside `lugus-financial` unless a specific workspace change becomes necessary and is discussed.

## Deferred scope

Market-data implementation, ticker search, a plugin marketplace or installer, embedded interpreters, sandboxing untrusted plugins, background scheduling, agent tools, full statement assembly, custom XBRL extraction, document-based fact extraction, and cross-provider reconciliation.
