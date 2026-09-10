"""Daily source-reported bars: pure normalization, injected transport, bounded session."""
from contextlib import redirect_stdout
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation
import json
from numbers import Integral, Real
from pathlib import Path
import re
import secrets
import sys
from urllib.parse import quote
from zoneinfo import ZoneInfo, ZoneInfoNotFoundError

MAX_LINE = 32 * 1024 * 1024
MAX_ROWS = 100_000
MAX_SNAPSHOT_BYTES = 24 * 1024 * 1024
YFINANCE_VERSION = '1.7.0'


class ProviderError(Exception):
    def __init__(self, kind, message, code=-32000):
        super().__init__(message)
        self.kind = kind
        self.code = code


def invalid(message):
    return ProviderError('invalid_request', message, -32602)


def utc_now():
    return datetime.now(timezone.utc).isoformat(timespec='seconds').replace('+00:00', 'Z')


def query_params(params):
    try:
        if not isinstance(params, dict) or set(params) - {'instrument','start','end','page_size','cursor'}:
            raise ValueError('Invalid query fields')
        instrument = params['instrument']
        if not isinstance(instrument, dict) or set(instrument) != {'namespace','value'}:
            raise ValueError('Invalid instrument')
        if instrument['namespace'] != 'yahoo:symbol':
            raise ValueError('Expected yahoo:symbol instrument')
        symbol = instrument['value']
        if not isinstance(symbol, str) or not symbol.strip() or symbol != symbol.strip() or len(symbol) > 128 or any(ord(c) < 32 for c in symbol):
            raise ValueError('Invalid symbol')
        for key in ('start','end'):
            if not isinstance(params[key], str) or not re.fullmatch(r'[0-9]{4}-[0-9]{2}-[0-9]{2}', params[key]):
                raise ValueError('Expected ISO date')
        start, end = date.fromisoformat(params['start']), date.fromisoformat(params['end'])
        if start > end or end == date.max:
            raise ValueError('Invalid inclusive date range')
        size = params.get('page_size', 100)
        if type(size) is not int or not 1 <= size <= 1000:
            raise ValueError('Invalid page size')
        cursor = params.get('cursor')
        if cursor is not None and (not isinstance(cursor, str) or not cursor or len(cursor) > 128):
            raise ValueError('Invalid cursor')
        return dict(instrument=dict(instrument), start=start.isoformat(), end=end.isoformat(), page_size=size, cursor=cursor)
    except (KeyError, TypeError, ValueError) as exc:
        raise invalid(str(exc)) from exc


def number(value):
    if isinstance(value, bool) or not isinstance(value, (Real, Decimal)):
        raise ValueError('Expected numeric price')
    result = Decimal(str(value))
    if not result.is_finite() or result < 0 or abs(result.adjusted()) > 1000:
        raise ValueError('Invalid price')
    return result


def normalize_rows(rows, metadata, query, retrieved_at):
    """Normalize typed row mappings without dataframe-wide float coercion."""
    try:
        currency, zone = metadata['currency'], metadata['exchangeTimezoneName']
        if any(not isinstance(v, str) or not v.strip() or len(v) > 128 for v in (currency,zone)):
            raise ValueError('Missing source currency/timezone')
        tz = ZoneInfo(zone)
        items, seen, size = [], set(), 0
        url = 'https://finance.yahoo.com/quote/' + quote(query['instrument']['value'], safe='') + '/history/'
        for row in rows:
            if len(items) >= MAX_ROWS:
                raise ValueError('Snapshot exceeds row limit')
            timestamp = row['Date']
            if not isinstance(timestamp, datetime) or timestamp.tzinfo is None or timestamp.utcoffset() is None:
                raise ValueError('Expected exchange-local timestamp')
            # Never move a bar to a different day by silently interpreting a UTC date.
            local = timestamp.astimezone(tz)
            if local.date() != timestamp.date() or local.utcoffset() != timestamp.utcoffset():
                raise ValueError('Timestamp disagrees with exchange timezone')
            day = timestamp.date().isoformat()
            if not query['start'] <= day <= query['end'] or day in seen:
                raise ValueError('Duplicate or out-of-query trading date')
            seen.add(day)
            prices = {key.lower(): number(row[key]) for key in ('Open','High','Low','Close')}
            if not prices['low'] <= min(prices['open'],prices['close']) <= max(prices['open'],prices['close']) <= prices['high']:
                raise ValueError('Contradictory OHLC range')
            volume = row['Volume']
            if isinstance(volume, bool) or not isinstance(volume, (Real, Decimal)):
                raise ValueError('Invalid volume')
            volume_decimal = Decimal(int(volume)) if isinstance(volume, Integral) else Decimal(str(volume))
            if not volume_decimal.is_finite() or volume_decimal < 0 or volume_decimal > 2**64-1 or volume_decimal != volume_decimal.to_integral_value():
                raise ValueError('Volume must be a nonnegative u64 integer')
            adjusted = row.get('Adj Close')
            item = dict(instrument=dict(query['instrument']), date=day,
                        **{key: format(value, 'f') for key,value in prices.items()},
                        volume=int(volume_decimal), adjusted_close=None if adjusted is None else format(number(adjusted),'f'),
                        currency=currency, exchange_timezone=zone, price_basis='source_reported',
                        precision='binary_float_source', source_url=url, retrieved_at=retrieved_at)
            size += len(json.dumps(item, separators=(',',':')).encode('utf-8'))
            if size > MAX_SNAPSHOT_BYTES:
                raise ValueError('Snapshot exceeds byte limit')
            items.append(item)
        if not items:
            raise ProviderError('not_found', 'Source returned no prices; trading-calendar coverage is unknown')
        return sorted(items, key=lambda item: item['date'])
    except (KeyError, TypeError, ValueError, AttributeError, ArithmeticError, ZoneInfoNotFoundError) as exc:
        raise ProviderError('malformed_data', str(exc)) from exc


def frame_rows(frame):
    """itertuples preserves an integer Volume column beyond binary float precision."""
    try:
        columns = list(frame.columns)
        if len(set(columns)) != len(columns) or not {'Open','High','Low','Close','Volume'} <= set(columns):
            raise ValueError('Missing or duplicate source columns')
        if len(frame) > MAX_ROWS:
            raise ValueError('Snapshot exceeds row limit')
        return [dict(zip(['Date'] + columns, values)) for values in frame.itertuples(index=True, name=None)]
    except (TypeError, ValueError, AttributeError) as exc:
        raise ProviderError('malformed_data', 'Invalid history dataframe') from exc


def fetch_history(symbol, options):
    """Pinned-library boundary; imported only for the first actual data request."""
    try:
        import yfinance as yf
        from yfinance.exceptions import YFRateLimitError, YFPricesMissingError, YFTzMissingError
    except ImportError as exc:
        raise ProviderError('configuration', 'Install plugins/yfinance/requirements.txt in the plugin environment') from exc
    if yf.__version__ != YFINANCE_VERSION:
        raise ProviderError('configuration', 'Expected yfinance==' + YFINANCE_VERSION)
    try:
        cache = Path(__file__).resolve().parent / '.cache'
        cache.mkdir(exist_ok=True)
        yf.set_tz_cache_location(str(cache))
        yf.config.debug.hide_exceptions = False
        ticker = yf.Ticker(symbol)
        frame = ticker.history(**options)
        if frame.empty:
            raise ProviderError('not_found', 'Source returned no prices; calendar coverage is unknown')
        # In 1.7.0 history populates this cache. Read only the required public
        # metadata keys; iterating the lazy metadata mapping can fetch tradingPeriods.
        metadata = ticker.get_history_metadata()
        source_metadata = {key: metadata.get(key) for key in ('currency','exchangeTimezoneName')}
        return frame_rows(frame), source_metadata
    except ProviderError:
        raise
    except YFRateLimitError as exc:
        raise ProviderError('rate_limited', 'Yahoo Finance rate limit') from exc
    except (YFPricesMissingError, YFTzMissingError) as exc:
        raise ProviderError('not_found', 'Source has no prices or exchange timezone for this query') from exc
    except TimeoutError as exc:
        raise ProviderError('timeout', 'History request timed out') from exc
    except (ValueError, KeyError, TypeError, AttributeError) as exc:
        raise ProviderError('malformed_data', 'Invalid history source response') from exc
    except Exception as exc:
        # curl_cffi timeout class is optional until the dependency is loaded.
        if type(exc).__name__ in ('Timeout','ReadTimeout','ConnectTimeout'):
            raise ProviderError('timeout', 'History request timed out') from exc
        raise ProviderError('unavailable', 'History source request failed') from exc


class Provider:
    def __init__(self, fetch=fetch_history, clock=utc_now):
        self.fetch, self.clock = fetch, clock
        self.initialized = False
        self.snapshot = None
        self.next_cursor = None
        self.offset = 0

    def initialize(self, params):
        self.initialized = False
        self.snapshot, self.next_cursor = None, None
        if not isinstance(params, dict) or type(params.get('protocol_version')) is not int or params['protocol_version'] != 1:
            raise ProviderError('unsupported', 'Expected protocol version 1')
        if params.get('config') != {}:
            raise ProviderError('configuration', 'Yfinance configuration must be an empty object')
        self.initialized = True
        return dict(protocol_version=1,plugin_id='yfinance',plugin_version='0.1.0',capabilities={'market_data':1})

    def daily(self, params):
        if not self.initialized:
            raise ProviderError('configuration', 'Initialize the provider first')
        query = query_params(params)
        cursor = query.pop('cursor')
        if cursor is None:
            self.snapshot, self.next_cursor, self.offset = None, None, 0
            options = dict(interval='1d',start=query['start'],end=(date.fromisoformat(query['end'])+timedelta(days=1)).isoformat(),
                           auto_adjust=False,back_adjust=False,repair=False,rounding=False,actions=False,keepna=True,timeout=10)
            with redirect_stdout(sys.stderr):
                rows, metadata = self.fetch(query['instrument']['value'], options)
            items = normalize_rows(rows, metadata, query, self.clock())
            self.snapshot = (query, items)
        elif self.snapshot is None or cursor != self.next_cursor or query != self.snapshot[0]:
            raise invalid('Expired cursor or cursor does not match the complete query')
        items = self.snapshot[1]
        stop = self.offset + query['page_size']
        page = items[self.offset:stop]
        self.offset = stop
        self.next_cursor = secrets.token_urlsafe(24) if stop < len(items) else None
        return dict(items=page,next_cursor=self.next_cursor,coverage=dict(first_date=items[0]['date'],last_date=items[-1]['date'],completeness='unverified'))


def error_response(request_id, error):
    return dict(jsonrpc='2.0',id=request_id,error=dict(code=error.code,message=str(error),data={'kind':error.kind}))


def handle(provider, request):
    request_id = None
    try:
        if not isinstance(request, dict) or request.get('jsonrpc') != '2.0' or type(request.get('id')) is not int or not isinstance(request.get('method'),str):
            raise ProviderError('invalid_request','Expected a JSON-RPC request with integer ID',-32600)
        request_id = request['id']
        params = request.get('params',{})
        if request['method'] == 'initialize':
            result = provider.initialize(params)
        elif request['method'] == 'market_data.daily':
            result = provider.daily(params)
        else:
            raise ProviderError('unsupported','Unknown method',-32601)
        return dict(jsonrpc='2.0',id=request_id,result=result)
    except ProviderError as exc:
        return error_response(request_id,exc)
    except Exception:
        return error_response(request_id,ProviderError('unavailable','Unexpected provider failure'))
