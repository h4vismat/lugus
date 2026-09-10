# Agent-created company-to-market-instrument bindings

Status: approved by the user on 2026-09-10, including automatic agent-created, source-supported bindings. Implementation follows the associated plan.
Base: application runtime on `codex/application-runtime`, commit `781a539`.

## Intended behavior

An agent researching Apple should use SEC listing evidence to discover AAPL,
check the selected market provider's instrument, persist the supported association,
and fetch prices without asking the user to enter the mapping. Ask for clarification
only when the intended company/listing/share class/provider remains ambiguous.
Binding creation is an ordinary agent capability, not a manual-only workflow.

The SEC side already retains company identifiers, ticker/exchange listings, source
URL/checksum, retrieval time, and exact observation/run membership. Apple's 2025
10-K explicitly reports AAPL on Nasdaq. The current yfinance adapter exposes daily
prices and currency/timezone, but no instrument identity lookup contract.

Sources inspected:
- https://www.sec.gov/Archives/edgar/data/320193/000032019325000079/aapl-20250927.htm
- https://ranaroussi.github.io/yfinance/reference/api/yfinance.Ticker.get_info.html
- Local SEC resolution and yfinance provider adapters.

Implementation source clarification: pinned yfinance1.7.0 `get_info()` overwrites
the returned symbol with the requested symbol, so it cannot establish independent
identity. The adapter instead primes bounded public history and reads selected
source fields from public history metadata. An offline characterization against the
installed library verifies that mismatched source symbols are preserved and rejected.
This changes the adapter method, not the approved evidence contract. See the
[lookup protocol](../../../lugus-financial/docs/protocol/instrument-lookup-v1.md).

## Approaches

1. Recommended: source-supported automatic bindings. Add a provider-neutral instrument
   lookup capability, retain its evidence, and let agents bind matching SEC listings
   through deterministic application validation. This supports the requested automatic
   workflow while retaining inspectable reasons for each association.
2. Agent inference from ticker strings alone. Smallest change, but cannot distinguish
   an issuer listing from a different provider instrument that happens to use the same
   text. Do not make this the acceptance policy.
3. Require a separate security-master service for every binding. Potentially stronger
   identifiers, but introduces another source dependency and is unnecessary for clear
   matches supported by the existing sources. The lookup port can support such sources
   later without changing application ownership.

## Evidence and validation contract

Introduce an optional, versioned instrument-lookup capability owned by known adapters.
The request contains an explicit provider-native instrument ID. Its bounded response
contains source-reported canonical instrument identity, issuer name, ticker, exchange,
instrument type, optional stable issuer/security identifiers, and source provenance.
Keep the requested ID separate from returned source identity: echoing an input or
successfully fetching prices is not independent identity evidence. Implement the first
adapter in the existing yfinance plugin; test against its pinned library boundary.

An agent uses a selected SEC company observation and reported listing to propose a
market instrument. Application-owned pure rules assess the saved evidence:
- The company observation and listing must belong to the caller's frozen resolution
  reference. Reject unresolved company identity conflicts.
- Prefer matching stable issuer/security identifiers where both sources provide them.
  Otherwise require concordant issuer names, ticker, exchange and instrument type.
- Use explicit, versioned normalization rules for names and known exchange labels.
  No fuzzy-name-only match or universal ticker punctuation replacement. Provider-specific
  symbol translations may generate candidates but do not prove a match.
- Preserve source limitations and classify acceptance as source-supported under the
  recorded policy, not globally verified security identity or historical identity proof.
- Missing fields, contradictions and multiple unresolved candidates stay explicit;
  agents may gather more evidence, and seek user clarification when needed.

Before implementation, the executable plan must enumerate the first supported
normalization cases and fixture payloads. Unsupported cases return an explicit outcome.
No source field or boolean supplied by the model grants verification authority.

## Durable bindings and use

Persist workspace-scoped bindings in the application layer, keeping financial evidence
in its existing repository. A binding retains the exact company observation, chosen
listing, market lookup evidence, provider identity, native instrument, policy version,
validation reasons, creation time and host-attached actor/run provenance.

Binding records and superseding/revocation history are immutable. Multiple instruments
per company are supported; creation does not establish a silent default. Fetching via a
binding requires an explicit binding ID and date range. The application resolves that
ID to the pinned provider/instrument and uses the existing worker, offering, cancellation
and ingestion path. Preserve binding provenance on resulting fetch/dataset references,
so old views retain their original meaning after a binding is superseded.

Agents and manual clients share lookup, bind, list/read and bound-price-fetch operations.
Models cannot choose workspace ownership, replace evidence provenance, or assert their
own validation result. Provider restart uses existing generation checks; provider identity
changes require explicit revalidation rather than automatic reassignment. Offline reads
of bindings and prior data require no running provider. No automatic retry/failover.

## Acceptance

Use real synthetic provider processes and deterministic AgentRuntime tests:
- Apple SEC observation -> AAPL/Nasdaq lookup evidence -> agent-created binding ->
  price fetch and frozen dataset, with no per-binding user confirmation.
- Same ticker with wrong issuer/exchange/type is rejected; missing evidence remains
  incomplete; multiple share classes never select the first result implicitly.
- Agent scope/provenance forgery is rejected; all inputs, pages and outputs are bounded.
- Binding history, cross-workspace denial, concurrent request deduplication, offline
  reopening, and original dataset meaning survive refresh/supersession.
- Existing provider lifecycle/cancellation and financial selection tests remain green.

This milestone adds identity evidence and routing. It does not add conversations,
background refresh, passage extraction, desktop UI, or a universal security master.
