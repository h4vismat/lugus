# Deterministic financial observation selection

Status: approved and implemented on branch `codex/company-resolution-observations`. Selection is a host-owned, pure, versioned policy over normalized evidence; providers retain ownership of source retrieval and source semantics.

## Scope

Produce coherent daily chart datasets and selected reported fundamentals with reproducible evidence references. Preserve all original observations and ingestion history. This layer does not reconcile providers, assemble full statements, calculate standalone quarters/TTM, perform currency conversion, or promise historical public-information accuracy.

Use distinct pure policies for market runs and fundamental disclosures. Both return a selection manifest including policy ID/version, query, full provider identity, selected run, observation references, source/retrieval metadata, and explicit limitations. Conflicts are data outcomes, not arbitrary ordering decisions.

## Required storage read boundary

Existing `snapshot` and `market_snapshot` expose all matching historical observations, while run membership is available through retrieval associations. Selecting directly from the flat snapshots risks combining incompatible source revisions.

Add repository-port reads that return a run's exact observations with their fingerprints and retrieval associations, plus run start/finish timestamps and query. Expose these through the repository abstraction rather than querying SQLite from a selector. Filings also need stable observation references; the current snapshot returns their payloads without observation IDs.

Define a host-assigned monotonic ingestion sequence within each repository for run recency. It describes fetch initiation order, not source publication time or financial authority. Existing append-only run IDs can seed migration values; future allocation must not reuse sequence numbers. Repository identity plus sequence makes references unambiguous across databases. Host timestamps remain separately available for diagnostics and future temporal queries.

The orchestration layer should serialize refreshes for the same normalized request scope. With concurrent overlapping runs, a late-finishing older run must not supersede a later-started completed run. The selector uses ingestion sequence, never completion-arrival order, to implement this explicit local recency policy.

## Common dataset eligibility

1. Require one explicit full `ProviderIdentity` and normalized query. Do not mix plugin versions or provider instances by default.
2. Consider completed runs whose requested scope contains the selection query. A successful run is not a guarantee of source coverage.
3. Choose the eligible run with the greatest ingestion sequence. Select only observations linked to that run, then filter to the requested scope. Retain the run's limitations and retrieval associations.
4. If a newer compatible refresh failed or is running, retain the last completed selection and expose that newer status. A partial run is inspectable separately and is never silently spliced into the default dataset.
5. If no compatible completed run exists, return no completed dataset with available-run diagnostics. Do not relabel partial evidence as complete or silently shorten the requested range.

Price containment requires the same instrument and a containing requested date range. Fundamentals containment requires the same company, a containing filing-date range, and a forms set containing the requested forms; an empty forms set means all forms. Page size and cursor are transport fields and do not affect semantic query identity. Runs must include the requested operation (`facts`, `filings`, or `both` as applicable).

A successfully retrieved empty dataset is an empty result, not permission to resurrect older values as current. Historical data remains accessible as a separately labelled prior dataset. Likewise, if a fact disappears from a newer complete source response, show it as absent from that selected snapshot without asserting the underlying disclosure was withdrawn.

Explicitly opening a historical run bypasses automatic run choice and records the selected run ID. It does not change the default current policy.

## Daily prices

Policy ID: `daily-series:1`.

Use the common run rule, then require one bar per exchange-local trading date for the requested instrument. Sort dates ascending. Conflicting duplicate dates yield an invalid/conflicting dataset; neither observation ID nor largest price chooses a winner.

Keep currency, exchange timezone, price basis, and precision metadata explicit. A change in series currency/timezone/basis must not become a single unlabeled series; return a semantic conflict requiring an explicit partition or query adjustment. Precision labels remain attached to their data.

Source-reported OHLC and adjusted close are separate series. An adjusted-close request with missing values produces gaps, not fallback to source-reported close. Do not infer split-unadjusted/as-traded semantics from `source_reported`. No synthetic bars or forward-fill is introduced.

The latest available closing price is the close at the maximum trading date in the selected source-reported dataset. Return that date, currency, provider, retrieval association, and completeness information. If the query produces no bars, return no price. It is not a live quote or a guarantee that the latest expected session is present.

Do not patch old dates from another ingestion snapshot. Re-fetch a sufficiently broad range when a coherent current series is needed. A future incremental provider capability can introduce explicit snapshot/version guarantees; v1 has no such guarantee across independent fetches.

## Reported fundamentals

Policy ID: `reported-facts:1`.

Begin with the chosen completed run. A selected metric query identifies company, exact source namespace/concept, unit, and period semantics. A canonical metric mapping may help discover concepts but does not silently combine distinct source concepts or mapping versions.

For each exact group `(company, namespace, concept, unit, period)`:

1. Apply explicit form and filing-date filters.
2. Find the greatest filing date among eligible observations for that group. A later disclosure of the same exact period is preferred by this display policy; it is not automatically labelled an amendment or restatement.
3. Retain all candidate disclosures on that date. If their exact decimal values agree numerically, return the value with all supporting references. Decimal equality is computed without binary floating point; source spellings remain preserved.
4. If values differ, return a conflict containing candidates and reasons. Do not use accession lexicographic order, database IDs, or retrieval timestamps to decide financial correctness. A later retrieval of the same accession is also not automatically authoritative if both values coexist in the selected run.

A source correction that appears alone in a newly selected run becomes the selected value for that run. Previous run selections preserve the previous value. If two corrected versions coexist within one run, the conflict remains explicit.

An amended form is eligible only if included in the requested forms scope (or all forms were requested). There is no automatic `/A` priority over another disclosure on the same date. Acceptance timestamps may support a future policy, but v1 facts do not consistently carry a comparable public timestamp, so same-date disagreement remains unresolved.

Keep instant facts separate from duration facts. Duration identity uses exact start and end dates. `fiscal_year` and `fiscal_period` remain filing metadata; they do not replace the actual observation period or classify every comparative fact as a current annual/quarterly result.

For a latest instant metric, choose the greatest instant date among the resulting groups; if that latest group conflicts, show the conflict rather than an older clean number. For duration metrics, return separately labelled exact-period groups. Do not create a generic latest revenue/net-income card until the caller selects a comparable period or a later fiscal-period classifier is designed.

An overview may initially show supported instant metrics. Duration metrics can appear with explicit date ranges in the fundamentals view. Missing or unmapped concepts remain unavailable/unmapped; do not manufacture revenue by combining arbitrary taxonomy concepts.

## Time, provenance, and saved evidence

Distinguish the financial period, filing/publication metadata, source retrieval time, host ingestion time, and selected run. A freshly retrieved old annual figure remains an old annual figure.

Initial modes are current stored selection and an explicitly pinned stored selection. A filing-date filter means only that eligible disclosures have filing dates in that range. It is not a historical market-information simulation: current Company Facts may include later source corrections, and date-only filing metadata does not establish intraday availability.

Do not expose a general `as_of` mode in this slice. Future public-as-of and locally-known-as-of queries require separate specifications, sufficient timestamp semantics, and correction history. Existing run timestamps alone do not prove when every page became publicly available or locally committed.

A saved selection manifest freezes its policy version and exact evidence references. Refresh produces a new manifest. Existing durable reviews continue storing their immutable evidence payloads; this layer must not reselect those payloads when reopening an assessment.

## Validation and examples

| Input | Expected result |
| --- | --- |
| Run A contains closes 100/101; newer run B contains 98/99 | Both chart points come from B; no per-date mixture |
| An older run finishes after a newer run | The newer initiated, completed compatible run remains selected |
| Newest refresh fails after one page | Prior completed dataset plus failed-refresh status; partial page available separately |
| A completed newer response is empty | Empty current dataset, with older evidence accessible as history |
| Two fetched revisions of the same bar are stored globally | Select only the revision associated with the chosen run |
| Annual and nine-month figures have the same end date | Separate exact duration groups |
| Same-period value 100 filed earlier, value 90 filed later | Select 90 with its later disclosure reference; retain 100 in evidence history |
| Same latest filing date contains 90 and 95 | Conflict; do not choose by accession or insertion order |
| Same latest filing date contains decimal strings `90` and `90.00` | Numerically equal value with both provenance references |
| New filing repeats an older comparative period | It can update that period's selection without becoming the latest financial period |
| Most recent instant period conflicts, older period is clean | Show the most recent period's conflict |
| Adjusted close is missing | Gap; no substitution with unadjusted/source-reported close |
| New provider plugin version is installed | No silent cross-version dataset merge |

Pure selector tests must demonstrate permutation invariance, stable manifests for repeated inputs, exact decimal comparison, scope containment, and preservation of conflict/provenance. Repository integration tests verify run membership, repeated retrievals, reopen, and preservation of old evidence after migration. Use a fake clock/sequence allocator where host chronology is tested.

## Implementation boundaries

Place pure selection types and policies in a focused `lugus-financial` module. Extend repository ports and SQLite reads additively for run-scoped evidence and chronology; preserve current snapshot methods for existing callers. Do not make storage select a default current value or change ingestion fingerprints.

Coordinate migration ordering with the company catalog change. Add fixtures and a CLI/example that explains selected run, policy, observation references, and conflicts before desktop consumption. No UI, background scheduler, provider fallback, full statement assembly, or durable-review evidence expansion is part of this slice.

## References

- [Current financial semantics](../../../lugus-financial/README.md)
- [Current market protocol](../../../lugus-financial/docs/protocol/market-data-v1.md)
- [SEC API documentation](https://www.sec.gov/search-filings/edgar-application-programming-interfaces)

The selection ordering above is an explicit Lugus policy, not a claim that SEC data arrives as a reconciled current statement.
