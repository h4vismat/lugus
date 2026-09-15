# Yfinance market data and instrument lookup plugin 0.3.0

An independently installed Python adapter for Lugus protocol v1, `market_data: 1` and
`instrument_lookup: 1`.
It retrieves daily historical bars for explicit `yahoo:symbol` identifiers. No API key,
company binding decision, or fallback provider is used.

From the repository root, install Python 3.10+ and the pinned upstream library:

```sh
python3 -m venv plugins/yfinance/.venv
plugins/yfinance/.venv/bin/python -m pip install -r plugins/yfinance/requirements.txt
python3 -m unittest discover -s plugins/yfinance/tests -v
```

The manifest uses `.venv/bin/python`, relative to the plugin directory. To run directly:

```sh
cd plugins/yfinance
.venv/bin/python main.py
```

Send one JSON object per line; initialize does not import yfinance or access the network:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocol_version":1,"config":{}}}
{"jsonrpc":"2.0","id":2,"method":"market_data.daily","params":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"},"start":"2024-01-01","end":"2024-01-31","cursor":null,"page_size":100}}
```

Dates are inclusive. The adapter adds one day to `end` for the library's exclusive
boundary. It requests daily history with `auto_adjust=False`, `back_adjust=False`,
`repair=False`, `rounding=False`, `actions=False`, `keepna=True`, and `timeout=10`.
Library exceptions propagate; stdout is redirected to stderr during library calls.
Timezone/cookie caches reside in this plugin's ignored `.cache/` directory.

Prices are `source_reported`: Yahoo/yfinance native OHLC, **not a guarantee of raw
split-unadjusted as-traded prices**. Adjusted close is retained separately when supplied.
Plain decimal strings reflect the binary float representation returned by yfinance;
`binary_float_source` explicitly records that upstream precision limitation. Dataframe
rows use typed tuples to preserve integer volume, including integers above 2^53.
The pinned library itself performs transformations, including replacing missing source
volume with zero before returning its dataframe. Validation detects invalid values
present in that dataframe; it cannot recover original Yahoo values changed upstream.

Currency and exchange timezone must be supplied by history metadata. No USD or timezone
default is guessed. The adapter preserves the exchange-local trading date, rejects
missing/nonfinite/negative numeric data, contradictory OHLC, fractional or overflowing
volume, duplicate dates, and dates outside the query. Missing adjusted-close columns
or explicit nulls produce null; nonfinite adjusted close is rejected.

Pagination reuses one bounded downloaded snapshot (100,000 rows / 24 MiB normalized
records). Its coverage endpoints describe the whole snapshot, and completeness always
remains `unverified`; no trading-calendar completeness is claimed. An empty source
result becomes `not_found`, since an empty download cannot establish a no-trading range.
Only one next cursor is retained, bound to the entire query including page size.
Cursors are single-use and expire on initialization or a new root query. Restart at
page one after process exit or failure. Input/output lines are limited to 32 MiB.
The provider makes no retry loop; yfinance may perform its own cookie/request handling,
and the host's process deadline remains the outer bound.

Errors distinguish configuration, malformed data, rate limiting, missing prices,
timeouts, unavailable source and invalid protocol/query requests. The fixture suite
runs without the upstream dependency or network.

Upstream references checked September 9, 2026:
[yfinance 1.7.0](https://pypi.org/project/yfinance/1.7.0/),
[tagged history implementation](https://github.com/ranaroussi/yfinance/blob/1.7.0/yfinance/scrapers/history.py).
yfinance is unofficial, unaffiliated with Yahoo, and intended for personal/research use;
Yahoo's terms govern use of its data.

## Source instrument lookup

Version 0.2.0 adds `instrument_lookup.lookup` for explicit native instruments:

```json
{"jsonrpc":"2.0","id":3,"method":"instrument_lookup.lookup","params":{"instrument":{"namespace":"yahoo:symbol","value":"AAPL"}}}
```

Lookup uses public `Ticker.history()` over five daily bars to prime chart metadata,
then reads `symbol`, `longName`, `exchangeName`, and `instrumentType` through public
`Ticker.get_history_metadata()`. The request symbol never fills missing source identity.
`Ticker.get_info()` is excluded because yfinance 1.7.0 overwrites its symbol with the
request. Optional fields remain null; no CIK or security identifiers are invented.
The result includes a SHA-256 checksum of the bounded canonical four-field projection,
a source URL and retrieval timestamp. Missing/wrong source symbol fails validation.
The existing daily pagination snapshot is unaffected by lookup.

See [instrument lookup v1](../../docs/protocol/instrument-lookup-v1.md) for field bounds,
checksum canonicalization, exact source/library limitations and immutable storage rules.
The suite includes optional offline characterization of the pinned library using synthetic
chart JSON; it performs no live requests. Run it with the plugin virtual environment to
include that test; dependency-free execution skips only that characterization.
## Fetch recovery

Fresh history and instrument metadata reads share bounded transient retries and
a per-process cooldown. Rate limiting returns a delay of at least 60 seconds;
exhausted timeouts/outages return at least five seconds. Subsequent fresh requests
respect that delay without hitting Yahoo. Snapshot pagination does not refetch.
Missing or malformed data and configuration failures are not retried. See the
[recovery report](../../../docs/data-fetch-recovery.md) for guarantees and limits.


## Historical prices

Version 0.3.0 adds `historical_prices.daily`. Its immutable paged snapshot includes
all calendar dates from the preceding trading session through the current New York
normalization date. It uses pinned `pandas_market_calendars==5.4.0` exchange schedules.
Missing session prices remain null. Stock splits after the requested chart range
still participate in reversing Yahoo Close into trading-date share prices.
`^SP500TR` is preserved as an unmodified total-return index level.

See [historical prices v1](../../docs/protocol/historical-prices-v1.md).
Run `python live_history_check.py --output /tmp/lugus-history-live.json` in the plugin
virtual environment for a read-only source check of Apple's 2020 split and the
S&P 500 total-return index. Source completeness remains explicitly unverified.
