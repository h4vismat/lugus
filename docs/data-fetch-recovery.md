# Agent data-fetch failure investigation and recovery

Date: 2026-09-10

Application-owned retrieval and exact-match cache reuse have since been added; see
[research preparation](application-research-preparation.md). The remaining-limit
notes below describe the original recovery patch.

## Findings and implemented corrections

The important failure was a lifecycle error, rather than a missing data provider.
A plugin could finish a valid JSON-RPC exchange with an upstream `timeout` or
`unavailable` error, and the financial host would close its otherwise healthy
process. The application worker also treated every source timeout as a cancelled
exchange and removed the provider from the available catalog. Subsequent requests
could therefore fail until application/provider restart.

The host now distinguishes a completed source error from a lost or invalid
protocol exchange. Source timeouts and outages remain failed fetches, with their
original run receipts, while the same worker can accept a later new request.
Actual host deadlines, cancellation, EOF, invalid framing and protocol violations
still invalidate the connection. A new request does not resume an old cursor.

| Failure | Prior behavior / risk | Recovery implemented |
| --- | --- | --- |
| Completed source timeout or outage | Plugin could be killed and unavailable afterward | Keep the synchronized connection; retain failed run and allow a later fetch |
| Yahoo transient timeout or outage | One attempt | Up to three fresh source reads with exponential delay and jitter |
| Yahoo rate limit | Every new tool call could hit the source again | Return a retry delay and share cooldown across history and instrument lookup |
| Repeated Yahoo outage | Agent could repeatedly hammer a failing endpoint | Five-second minimum cooldown after the attempt budget is exhausted |
| SEC long Retry-After | Returned guidance did not suppress a subsequent request | Enforce remaining cooldown in the transport shared by SEC capabilities |
| SEC HTTP 401/403 | Exposed as a retryable outage | Return a nonretryable configuration/access error requiring attention |
| SEC deadline exhausted during rate limiting | Could still start another request | Recheck the deadline immediately before opening the request |
| Missing, malformed, unsupported or misconfigured source | Repeating unchanged requests cannot solve the cause | Do not automatically retry these failures; provide actionable, bounded application messages |
| Agent recovery decisions | Generic errors provided little guidance | Explain cooldowns, failed receipts, identity ambiguity, existing evidence and honest coverage reporting in the research instructions |

## Architecture and guarantees

The existing functional core remains intact: identifiers, queries, normalization,
validation and retry decisions are independent of network I/O. The new Yahoo
`recovery.py` module owns the injected clock, sleep, jitter and minimal cooldown
state. Both Yahoo capabilities share this edge. Rust continues to own process
lifecycle, bounds, authorization and durable evidence.

Retries happen only around idempotent source reads, before snapshot creation.
They do not replay JSON-RPC cursor advancement, persistence, dataset creation,
view requests, or entire ingestion jobs. Host and provider retries therefore do
not multiply. Existing tool-call idempotency still returns the original receipt;
a later fetch uses a new call identity and receives a new run association.

Yahoo retries `timeout` and `unavailable` at most twice after the initial attempt,
waiting one then two seconds plus up to half a second of jitter. A 35-second
retry admission budget reserves ten seconds for the next source request. The
external library can perform several internal requests, so this budget is not a
hard wall-clock limit; the Rust operation deadline and process cleanup remain the
hard bound. An explicit retry delay is never shortened. Long delays return
immediately rather than sleeping through an interactive turn.

Yahoo rate limiting does not trigger immediate retries. Its cooldown is at least
60 seconds; exhausted other transient failures use at least five seconds. These
are Lugus policies, not promised upstream recovery times. Cooldown responses
preserve the error kind and return rounded-up remaining seconds. They do not
perform network I/O. Reinitializing the same provider does not bypass cooldown.

SEC retains its existing three-attempt transient HTTP retry policy and shared
two-starts-per-second limiter. Terminal rate-limit responses impose a cooldown
of at least 60 seconds, or longer if requested by the server. Other terminal
transient errors with Retry-After also retain the requested delay. Valid cached
snapshot pages need no network call and remain usable.

Application errors retain their existing schema, classification, retryability,
retry-delay field and bounded serialization. Messages do not expose arbitrary
provider exception text, configuration contents or credentials. No database
migration or new vendor dependency is introduced.

## User and agent experience

1. For a brief transient Yahoo failure, the plugin attempts recovery within the
   current fetch. Only validated source data becomes a successful dataset.
2. If the source remains unavailable, the fetch returns a typed failure and
   durable receipt. The agent explains the next useful action rather than
   interpreting failure as an empty successful dataset.
3. For rate limits, the agent receives a remaining delay. Requests made during
   cooldown fail quickly without further upstream traffic.
4. Relevant, already-owned saved evidence can still be used, with its provider,
   retrieval date, coverage limits and failed-refresh status disclosed. Existing
   offline reads continue to avoid network access.
5. Missing data calls for checking the exact listing, identifier, dates and
   source coverage. The agent must ask when identity is ambiguous. It must not
   silently switch listings, currencies, providers or price bases, or invent
   data from a failed request.
6. Invalid responses and setup problems call for correction, not retry loops.
   Cancellation and hard process failures still need a fresh provider session.

The instructions guide model behavior; they are not a deterministic guarantee of
an agent's final prose. Attempt counts, cooldowns, data validation, process cleanup
and evidence preservation are enforced in code.

## Plugin assessment

Both installed source plugins benefit from these changes, and the process fix
applies to any plugin implementing the existing protocol. A new provider plugin
would not correct the lifecycle bug. No new provider was needed for this patch.

SEC submissions and Company Facts use documented JSON APIs. SEC requests must
respect an aggregate rate limit across machines and processes; rotating identities
or processes is not a recovery strategy. See the official [fair-access guidance](https://www.sec.gov/about/developer-resources).

Missing SEC facts may be a coverage limitation rather than a network failure:
Company Facts aggregates standard-taxonomy, entity-wide data. Custom and
dimensional facts require a different extraction path. SEC also publishes bulk
archives for large workloads; repeatedly fetching per-company APIs is not the
only scaling option. See the [official API documentation](https://www.sec.gov/search-filings/edgar-application-programming-interfaces).

A future alternate market plugin should use the existing capability port and its
own namespaced instrument identity. Before offering fallback, verify listing,
exchange, currency, timezone, date coverage, price adjustment basis and evidence
provenance. Store alternate-provider results separately. No automatic merging of
bars or inference that a ticker uniquely identifies a company is acceptable.

## Remaining limits and next extensions

- Cooldowns are per plugin process, not global across applications, machines or
  repeated process restarts. Aggregate scheduling belongs in a shared host/source
  coordinator if multiple instances become common.
- Hard process failures still deactivate the worker. Automatic restart needs
  generation-aware lifecycle handling, new offerings, and explicit replay policy;
  blindly restarting and replaying a cursor is unsafe.
- Cached evidence use relies on existing owned references. This patch does not
  add automatic discovery or selection of a prior compatible dataset, nor a
  configurable freshness policy. It never silently labels old data as refreshed.
- Retry attempts are internal to plugins. The UI sees the existing running job
  and terminal receipt, not a live attempt countdown. Rich progress events would
  require an explicit protocol extension rather than extra stdout messages.
- A narrower SEC filing-date query may reduce historical submissions requests,
  but Company Facts still downloads the full company payload. Reducing page size
  cannot solve an oversized source snapshot.
- Live source availability, user access configuration and reasoning quality need
  separate live acceptance checks. Deterministic tests deliberately inject
  transport failures instead of relying on real outages or consuming rate limits.

## Verification

Regression tests cover a real JSON-RPC child recovering after a source error,
worker/catalog reuse with distinct failed and completed durable runs, hard
timeout/cancellation/EOF cleanup, Yahoo transient success followed by cursor-only
pagination, retry exhaustion, cooldown across capabilities, preserved Retry-After,
permanent failure classification, SEC access denial and deadline admission.

Commands:

```sh
cargo test --workspace --all-targets --offline
cargo clippy --workspace --all-targets --offline -- -D warnings
cargo fmt --all -- --check
python3 -m unittest discover -s lugus-financial/plugins/sec-edgar/tests -v
lugus-financial/plugins/yfinance/.venv/bin/python -m unittest discover -s lugus-financial/plugins/yfinance/tests -v
```

The system-Python Yahoo suite can run without yfinance; its optional installed
library characterization test is skipped in that environment. The plugin virtual
environment runs that test against the pinned dependency without live requests.

Verification completed: 413 workspace Rust tests, 34 SEC Python tests, and all 29
Yahoo Python tests in the plugin virtual environment passed. Strict workspace
Clippy, formatting and `git diff --check` passed. An independent read-only review
reported no concrete correctness or regression findings. No live source requests
were used to validate these changes.
