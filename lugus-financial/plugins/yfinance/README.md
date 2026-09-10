# Yfinance daily market data plugin

An independently installed Python adapter for Lugus protocol v1 and `market_data: 1`.
It retrieves daily historical bars for explicit `yahoo:symbol` identifiers. No API key,
company identity inference, or fallback provider is used.

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
