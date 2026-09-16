"""Pure projection of source chart metadata and an injected public-library boundary."""
from collections.abc import Mapping
import hashlib
import json
from pathlib import Path
import unicodedata
from urllib.parse import quote

from common import ProviderError, YFINANCE_VERSION

SOURCE_FIELDS = {'symbol': 128, 'longName': 1024, 'exchangeName': 128, 'instrumentType': 128}


def valid_text(value, max_bytes):
    return (isinstance(value, str) and bool(value.strip())
            and len(value.encode('utf-8')) <= max_bytes
            and not any(unicodedata.category(char) == 'Cc' for char in value))


def lookup_params(params):
    try:
        if not isinstance(params, dict) or set(params) != {'instrument'}:
            raise ValueError()
        instrument = params['instrument']
        if not isinstance(instrument, dict) or set(instrument) != {'namespace', 'value'}:
            raise ValueError()
        symbol = instrument['value']
        if (instrument['namespace'] != 'yahoo:symbol' or not valid_text(symbol, 128)
                or symbol != symbol.strip()):
            raise ValueError()
        return {'instrument': dict(instrument)}
    except (ValueError, TypeError, KeyError, UnicodeError) as exc:
        raise ProviderError('invalid_request', 'Expected a bounded yahoo:symbol instrument lookup', -32602) from exc


def project_metadata(source, query, retrieved_at):
    """Hash only the bounded source fields used; requests never fill identity gaps."""
    query = lookup_params(query)
    try:
        if not isinstance(source, Mapping):
            raise ValueError()
        fields = {}
        for key, max_bytes in SOURCE_FIELDS.items():
            value = source.get(key)
            if value is not None and not valid_text(value, max_bytes):
                raise ValueError()
            fields[key] = value
        symbol = fields['symbol']
        if symbol is None or symbol != query['instrument']['value']:
            raise ValueError()
        canonical = json.dumps(fields, sort_keys=True, ensure_ascii=False, separators=(',', ':'), allow_nan=False).encode('utf-8')
        exchange, kind = fields['exchangeName'], fields['instrumentType']
        return dict(
            instrument={'namespace': 'yahoo:symbol', 'value': symbol},
            issuer_name=fields['longName'], ticker=symbol,
            exchange=None if exchange is None else {'namespace': 'yahoo:exchange', 'value': exchange},
            kind=None if kind is None else ('equity' if kind == 'EQUITY' else 'other'),
            issuer_identifiers=[],
            source_url='https://finance.yahoo.com/quote/' + quote(symbol, safe='') + '/',
            source_checksum=hashlib.sha256(canonical).hexdigest(), retrieved_at=retrieved_at,
        )
    except (ValueError, TypeError, KeyError, AttributeError, UnicodeError) as exc:
        raise ProviderError('malformed_data', 'Invalid source instrument metadata or symbol mismatch') from exc


def fetch_metadata(symbol, unlimited_research=False):
    """Prime bounded daily history, then read actual chart.meta fields.

    yfinance 1.7.0 get_info overwrites symbol with the request; it cannot be
    identity evidence. HistoryMetadata preserves chart.meta's source symbol.
    """
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
        from transport import ticker_for_research
        ticker = ticker_for_research(yf, symbol, unlimited_research)
        # Prime explicitly: get_history_metadata alone swallows initial errors.
        ticker.history(period='5d', interval='1d', auto_adjust=False,
                       back_adjust=False, repair=False, actions=False, timeout=None if unlimited_research else 10)
        metadata = ticker.get_history_metadata()
        # Do not iterate its lazy mapping: tradingPeriods could trigger extra IO.
        return {key: metadata.get(key) for key in SOURCE_FIELDS}
    except ProviderError:
        raise
    except YFRateLimitError as exc:
        raise ProviderError('rate_limited', 'Yahoo Finance rate limit') from exc
    except (YFPricesMissingError, YFTzMissingError) as exc:
        raise ProviderError('not_found', 'Source has no instrument history metadata') from exc
    except TimeoutError as exc:
        raise ProviderError('timeout', 'Instrument metadata request timed out') from exc
    except (ValueError, KeyError, TypeError, AttributeError) as exc:
        raise ProviderError('malformed_data', 'Invalid instrument metadata source response') from exc
    except Exception as exc:
        if type(exc).__name__ in ('Timeout', 'ReadTimeout', 'ConnectTimeout'):
            raise ProviderError('timeout', 'Instrument metadata request timed out') from exc
        raise ProviderError('unavailable', 'Instrument metadata source request failed') from exc
