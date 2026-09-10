# Company resolution capability and local catalog

Status: architectural direction approved in conversation; detailed specification proposed for review. Initial discovery covers SEC data. International discovery is added through plugins implementing the same capability. This document does not implement production code.

## Boundaries

Add optional `company_resolution: 1` to the existing capability handshake. The SEC plugin implements it alongside filings and fundamentals; no separate plugin package is required. The JSON-RPC envelope remains protocol v1. Existing plugins without this capability continue working.

Plugins search their sources and return source-native entity records and associations. Lugus owns internal company IDs, provenance persistence, candidate selection, and conflict handling. Matching company names never merges catalog identities. Financial observation selection remains a separate host policy.

Implement a `CompanyResolutionProvider` port in `lugus-financial` with search and lookup operations, an external-process adapter, pure validation/matching functions, and a catalog repository port. Extend the SEC plugin with a separate resolution module; reuse its HTTP transport, configured User-Agent, source URL validation, and process lifecycle.

## Identity model

- `CatalogCompanyId` is an opaque, host-assigned persistent identity. Do not replace the existing `CompanyId` used by financial queries; that type continues representing a namespaced external entity identifier.
- A candidate has a primary external entity identifier, source display name, optional sourced aliases, and zero or more reported listing associations. Plugins never choose catalog IDs or supply host provider identity.
- An entity identifier has namespace and value. The SEC adapter normalizes CIKs to ten digits in `sec:cik`; accept positive decimal CIK input up to ten digits. Unknown namespaces are supported structurally by the host and interpreted only by adapters that own their semantics.
- Listing associations retain source-native ticker and exchange labels with explicit namespaces (`sec:ticker`, `sec:exchange` for this adapter), plus the associated entity identifier. An exchange label is not automatically an ISO MIC. A reported listing is not a globally identified security.
- Company, listing/security, and provider instrument remain separate concepts. The first slice stores reported listing associations without inventing security IDs. A `yahoo:symbol` mapping needs explicit sourced validation or a user-declared association; matching ticker strings alone does not create it.
- Every imported record retains host-attached `ProviderIdentity`, source URL, source content checksum, provider retrieval timestamp, host recording timestamp, and resolution run ID. Null source effective dates remain unknown; retrieval time does not establish when a ticker became valid.

Catalog IDs can be allocated through an injected ID generator, with a uniqueness constraint in persistence. External identifiers are looked up before allocating an ID. Exact CIK identity permits reusing an existing catalog entry; different CIKs never merge by name. If an incoming set of associations would join two existing catalog entries, return a conflict and preserve the incoming evidence without merging.

## Capability operations

`company_resolution.search` accepts a tagged query:

- `name`: nonempty text, normalized only for matching; source spelling remains intact.
- `identifier`: namespaced value; a listing identifier may include a namespaced exchange qualifier.

Both search forms accept `page_size` in 1..100 and an optional opaque cursor. Query text is limited to 256 UTF-8 bytes; namespace and identifier values to 128 bytes each. These are proposed v1 bounds, validated before network access.

Search returns candidates, `next_cursor`, an opaque source-snapshot token, and coverage describing the searched source. A candidate includes match reasons such as exact identifier, exact normalized name, or name substring. Do not expose an uncalibrated numerical confidence score.

Pagination must retain one source snapshot, ordering, and query. Cursors are process-local, expire on reinitialization/new root search, and cannot be replayed for another query. A malformed row or failed source request fails explicitly; it is not an empty result or silently skipped candidate.

`company_resolution.lookup` accepts one entity identifier and returns its current source record with provenance. Unsupported namespaces return `unsupported`; a supported identifier absent from the source returns `not_found`; network/parse failures retain their distinct errors. A listing ticker is searched rather than passed to entity lookup.

The host validates returned identity against lookup input. For search it validates bounds, identifier syntax where known, snapshot consistency, and advertised match reasons. Provider identity is always attached by the host.

## SEC implementation and coverage

Use `https://www.sec.gov/files/company_tickers_exchange.json` for initial name/ticker search. Interpret rows by the top-level `fields` names (`cik`, `name`, `ticker`, `exchange`), not assumed column positions. Reject missing/duplicate required fields and malformed row values. Preserve distinct listing associations under the same CIK; identical repeated rows may be deduplicated.

Use `https://data.sec.gov/submissions/CIK{ten_digit_cik}.json` for explicit CIK lookup. Verify the returned CIK, retain its source name, and preserve reported ticker/exchange associations. Do not fabricate pairings from mismatched arrays. A malformed association array is a source-data error.

An exact CIK search delegates to lookup. Directory name/ticker search covers only the directory snapshot. A company absent there may still be found through direct CIK lookup; already stored lookups also remain locally searchable. Searching this directory is not a claim to discover every current or historical SEC filer.

Treat returned records as SEC registrant identities. Do not assert that every CIK represents an operating company or infer entity type when the source does not establish it. International entities represented in SEC data may appear; comprehensive international discovery is outside this adapter's scope.

For name search, use case-insensitive matching after trimming and collapsing whitespace; exact normalized names sort before substring matches. For SEC ticker search, trim and compare ASCII case-insensitively while retaining punctuation. Do not replace dots, dashes, exchange suffixes, or share-class markers. Sort ties by normalized source name and CIK for reproducibility; sorting never selects a winner from ambiguous candidates.

Fetch and validate a bounded source payload using the existing SEC transport limits, then paginate filtered candidates in memory. No extra network call occurs for continuation pages. Exhaustion means all matches in this source snapshot have been returned, not that the source covers all entities.

## Catalog persistence and resolution results

Separate explicit provider refresh/search from offline catalog reads. A remote search stores its query, provider, scope, status, pages, and retrieval associations. Local search never starts a plugin or network operation. Preserve partial/failed search status; incomplete searches cannot yield an automatic unique resolution.

Store candidate observations append-only and link repeated identical records to new retrievals. Accepting or refreshing a candidate adds provenance and a catalog association rather than overwriting older evidence. An empty search does not delete catalog entities or establish delisting. Associations missing from one result are not automatically withdrawn.

Host outcomes:

| Outcome | Rule |
| --- | --- |
| Resolved within scope | Explicit entity lookup, user-selected candidate, or one exact identifier match after the requested provider search is exhausted and no known catalog conflict exists |
| Candidates | Name search, or identifier search with multiple matches; preserve match reasons and listing distinctions |
| No match in scope | Successfully exhausted source search with no candidate; retain coverage and snapshot identity |
| Incomplete/unavailable | Failed or partial source search; cached candidates may be returned with that status, never as a new unique resolution |
| Identity conflict | Sourced identifiers disagree with accepted catalog associations; preserve evidence and require explicit reconciliation |

A resolved company does not imply a selected market instrument. If several listings exist, the company may resolve while instrument selection remains ambiguous. No market provider is chosen by catalog ordering.

For multiple future providers, exact namespaced entity identifiers may connect candidate observations when that namespace has declared identity semantics. Unknown or provider-local identifiers remain scoped appropriately. Do not implement generic fuzzy entity merging in this slice.

## Compatibility and implementation scope

Add catalog storage beside the existing financial evidence tables through an additive migration after schema v2. Preserve current company keys, observations, and durable-review references. Catalog ownership does not rewrite historical financial records.

Add protocol documentation and fixture coverage for the new optional capability. Version the SEC plugin consistently with the repository's plugin identity policy; retain previous plugin-version provenance. No new financial source SDK, plugin management UI, automatic Yahoo binding, company-merger reconciliation, or security-master database is included.

Implement and validate catalog storage separately from observation-selection storage changes so migrations can be ordered explicitly in their implementation plans.

## Acceptance cases

1. SEC fixture search for IBM returns a candidate with CIK, name, and listing association; a selected candidate can be reopened offline under the same catalog ID.
2. CIK lookup outside the ticker directory succeeds when submissions data exists. Directory absence remains a coverage limitation.
3. Two names sharing a substring remain candidates. Two tickers associated with one CIK do not create two companies. One ticker associated with multiple CIKs is ambiguous.
4. Punctuation variants and exchange labels do not create a Yahoo mapping. A company with multiple listings requires explicit instrument selection.
5. A truncated search cannot resolve uniquely. Failed refresh leaves prior catalog evidence available with its provenance.
6. Renamed entities retain old sourced names. Same-name entities with distinct CIKs remain separate. Conflicting accepted identifier associations are surfaced.
7. Malformed source fields, inconsistent lookup CIK, invalid cursors, mismatched arrays, and unsupported namespace/version are rejected before any catalog merge.
8. Migration preserves existing financial data and frozen reviews. Repeated imports do not duplicate catalog identity or destroy source history.

## Sources checked 2026-09-10

- [SEC data access](https://www.sec.gov/search-filings/edgar-search-assistance/accessing-edgar-data): CIK identity, directory coverage caveats, and source access requirements.
- [SEC ticker/exchange directory](https://www.sec.gov/files/company_tickers_exchange.json): verified field layout and the IBM/CIK association.
- [SEC API documentation](https://www.sec.gov/search-filings/edgar-application-programming-interfaces): submissions metadata and identifier lookup.

The source choice and matching rules above are Lugus design decisions, not guarantees made by the SEC.
