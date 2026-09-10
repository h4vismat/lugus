"""Lookup identity is source metadata, never a requested-symbol echo."""
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import types
import unittest
from contextlib import redirect_stdout, redirect_stderr
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import provider
try:
    import instruments
except ImportError:
    instruments = None

Q = {'instrument': {'namespace': 'yahoo:symbol', 'value': 'AAPL'}}
SOURCE = {'symbol': 'AAPL', 'longName': 'Apple Inc.', 'exchangeName': 'NMS', 'instrumentType': 'EQUITY'}
AT = '2026-09-10T00:00:00Z'


class InstrumentTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(instruments, 'source instrument projection must be implemented')
        self.calls = []
        self.source = dict(SOURCE)
        def fetch(symbol):
            self.calls.append(symbol)
            return self.source
        self.p = provider.Provider(fetch_metadata=fetch, clock=lambda: AT)
        self.p.initialize({'protocol_version': 1, 'config': {}})

    def test_apple_source_projection_and_canonical_evidence_checksum(self):
        result = self.p.lookup_instrument(Q)
        self.assertEqual(result, {
            'instrument': {'namespace': 'yahoo:symbol', 'value': 'AAPL'},
            'issuer_name': 'Apple Inc.', 'ticker': 'AAPL',
            'exchange': {'namespace': 'yahoo:exchange', 'value': 'NMS'},
            'kind': 'equity', 'issuer_identifiers': [],
            'source_url': 'https://finance.yahoo.com/quote/AAPL/',
            'source_checksum': hashlib.sha256(b'{"exchangeName":"NMS","instrumentType":"EQUITY","longName":"Apple Inc.","symbol":"AAPL"}').hexdigest(),
            'retrieved_at': AT,
        })
        self.assertEqual(self.calls, ['AAPL'])
        self.assertEqual(self.source, SOURCE)
        self.source = dict(reversed(list(SOURCE.items())), unrelated=object())
        self.assertEqual(self.p.lookup_instrument(Q)['source_checksum'], result['source_checksum'])

    def test_missing_or_wrong_source_symbol_cannot_be_replaced_by_request(self):
        for symbol in (None, '', 'MSFT', 'aapl', 42):
            self.source = dict(SOURCE, symbol=symbol)
            with self.subTest(symbol=symbol), self.assertRaises(provider.ProviderError) as caught:
                self.p.lookup_instrument(Q)
            self.assertEqual(caught.exception.kind, 'malformed_data')
        self.source = {k:v for k,v in SOURCE.items() if k != 'symbol'}
        with self.assertRaises(provider.ProviderError): self.p.lookup_instrument(Q)

    def test_optional_source_fields_are_unknown_not_invented(self):
        self.source = {'symbol': 'AAPL'}
        result = self.p.lookup_instrument(Q)
        for field in ('issuer_name', 'exchange', 'kind'): self.assertIsNone(result[field])
        self.assertEqual(result['issuer_identifiers'], [])
        self.source = dict(SOURCE, instrumentType='ETF')
        self.assertEqual(self.p.lookup_instrument(Q)['kind'], 'other')
        self.source = dict(SOURCE, longName=None, exchangeName=None, instrumentType=None)
        self.assertIsNone(self.p.lookup_instrument(Q)['issuer_name'])

    def test_malformed_and_oversized_source_fields_fail_without_echoing_contents(self):
        for field, bad in [('longName', []), ('longName', 'a'*1025), ('longName', 'é'*513), ('longName', 'secret\ncontrol'), ('exchangeName', 'x'*129), ('exchangeName', 1), ('instrumentType', ''), ('symbol', 'x'*129), ('symbol','A\x7fAPL')]:
            self.source = dict(SOURCE, **{field: bad})
            with self.subTest(field=field, bad=type(bad)), self.assertRaises(provider.ProviderError) as caught:
                self.p.lookup_instrument(Q)
            self.assertEqual(caught.exception.kind, 'malformed_data')
            self.assertNotIn('secret', str(caught.exception))
        for source in (None, [], 'secret-source'):
            self.source = source
            with self.assertRaises(provider.ProviderError): self.p.lookup_instrument(Q)

    def test_lookup_requires_initialization_and_validates_before_io(self):
        self.p.initialized = False
        with self.assertRaises(provider.ProviderError) as caught: self.p.lookup_instrument(Q)
        self.assertEqual(caught.exception.kind, 'configuration')
        self.p.initialize({'protocol_version':1,'config':{}})
        bad_requests = [{}, dict(Q, extra=True), {'instrument':dict(Q['instrument'], extra=True)}, {'instrument': {'namespace':'sec:cik','value':'AAPL'}}]
        for bad in ('', 'x'*129, 'é'*65, 'A\x7fAPL', ' AAPL', 1):
            bad_requests.append({'instrument': {'namespace':'yahoo:symbol','value':bad}})
        for request in bad_requests:
            with self.subTest(request=request), self.assertRaises(provider.ProviderError) as caught: self.p.lookup_instrument(request)
            self.assertEqual(caught.exception.kind,'invalid_request')
        self.assertEqual(self.calls, [])

    def test_dispatch_capability_and_stdout_isolation(self):
        def fetch(symbol):
            print('library chatter')
            return dict(SOURCE)
        p = provider.Provider(fetch_metadata=fetch, clock=lambda:AT)
        init = provider.handle(p, {'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocol_version':1,'config':{}}})
        self.assertEqual(init['result']['capabilities'], {'market_data':1, 'instrument_lookup':1})
        self.assertEqual(init['result']['plugin_version'], '0.2.0')
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            response = provider.handle(p, {'jsonrpc':'2.0','id':2,'method':'instrument_lookup.lookup','params':Q})
        self.assertEqual(response['result']['ticker'],'AAPL')
        self.assertEqual(out.getvalue(), '')
        self.assertIn('library chatter', err.getvalue())

    def test_metadata_transport_does_not_disturb_active_daily_cursor(self):
        from test_provider import Q as DAILY, META, row
        self.p.fetch = lambda *args: ([row(2),row(3)], META)
        cursor = self.p.daily(DAILY)['next_cursor']
        self.p.lookup_instrument(Q)
        self.assertEqual(self.p.daily(dict(DAILY,cursor=cursor))['items'][0]['date'], '2024-01-03')


class MetadataBoundaryTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(instruments, 'source metadata transport must be implemented')

    def fake_library(self, failure=None):
        class RateLimit(Exception): pass
        class MissingPrices(Exception): pass
        class MissingTimezone(Exception): pass
        class Metadata:
            def get(self, key): return SOURCE.get(key)
            def __iter__(self): raise AssertionError('Do not iterate lazy metadata')
        seen = []
        class Ticker:
            def __init__(self, symbol): seen.append(('symbol', symbol))
            def history(self, **options):
                seen.append(('history',options))
                if failure == 'rate_limited': raise RateLimit('secret')
                if failure == 'not_found': raise MissingPrices('secret')
                if failure == 'timeout': raise TimeoutError('secret')
                if failure == 'malformed_data': raise ValueError('secret')
                if failure == 'unavailable': raise RuntimeError('secret')
            def get_history_metadata(self): return Metadata()
            def get_info(self): raise AssertionError('get_info symbol is overwritten by yfinance')
        yf = types.SimpleNamespace(__version__='1.7.0', Ticker=Ticker, set_tz_cache_location=lambda value:None, config=types.SimpleNamespace(debug=types.SimpleNamespace(hide_exceptions=True)))
        exceptions = types.SimpleNamespace(YFRateLimitError=RateLimit,YFPricesMissingError=MissingPrices,YFTzMissingError=MissingTimezone)
        return {'yfinance':yf, 'yfinance.exceptions':exceptions}, seen

    def test_public_chart_metadata_boundary_primes_history_and_reads_only_source_fields(self):
        modules, seen = self.fake_library()
        with patch.dict(sys.modules,modules), patch.object(Path,'mkdir'):
            self.assertEqual(instruments.fetch_metadata('AAPL'), SOURCE)
        self.assertEqual(seen, [('symbol','AAPL'), ('history',dict(period='5d', interval='1d', auto_adjust=False, back_adjust=False, repair=False, actions=False, timeout=10))])
        self.assertFalse(modules['yfinance'].config.debug.hide_exceptions)

    def test_typed_safe_library_failures(self):
        for kind in ('rate_limited','not_found','timeout','malformed_data','unavailable'):
            modules, _ = self.fake_library(kind)
            with self.subTest(kind=kind), patch.dict(sys.modules,modules), patch.object(Path,'mkdir'), self.assertRaises(provider.ProviderError) as caught:
                instruments.fetch_metadata('AAPL')
            self.assertEqual(caught.exception.kind,kind)
            self.assertNotIn('secret',str(caught.exception))

    def test_mismatched_library_version_never_fetches(self):
        modules, seen = self.fake_library()
        modules['yfinance'].__version__ = '0.0.0'
        with patch.dict(sys.modules,modules), self.assertRaises(provider.ProviderError) as caught:
            instruments.fetch_metadata('AAPL')
        self.assertEqual(caught.exception.kind, 'configuration')
        self.assertEqual(seen, [])


# Optional installed-library characterization: real 1.7.0 chart transformation,
# synthetic HTTP JSON, no credentials/network. Core tests above need no library.
try:
    import yfinance as installed_yfinance
except ImportError:
    installed_yfinance = None


@unittest.skipUnless(installed_yfinance is not None, 'optional pinned yfinance environment')
class InstalledChartMetadataTests(unittest.TestCase):
    def test_chart_source_symbol_is_preserved_when_it_disagrees_with_request(self):
        from yfinance.scrapers.history import PriceHistory
        self.assertEqual(installed_yfinance.__version__, '1.7.0')
        source = dict(SOURCE, symbol='MSFT', currency='USD', exchangeTimezoneName='America/New_York', validRanges=['5d'])
        payload = {'chart': {'error': None, 'result': [{
            'meta': source, 'timestamp': [1704205800],
            'indicators': {'quote': [{'open':[10.], 'high':[12.], 'low':[9.], 'close':[11.], 'volume':[100]}]},
        }]}}
        class Transport:
            def get(self, **kwargs):
                return types.SimpleNamespace(text='', json=lambda:copy.deepcopy(payload))
        history = PriceHistory(Transport(), 'AAPL', 'America/New_York')
        previous = installed_yfinance.config.debug.hide_exceptions
        try:
            installed_yfinance.config.debug.hide_exceptions = False
            history.history(period='5d', interval='1d', auto_adjust=False, back_adjust=False, repair=False, actions=False, timeout=10)
        finally:
            installed_yfinance.config.debug.hide_exceptions = previous
        raw = history.get_history_metadata()
        self.assertEqual(raw.get('symbol'), 'MSFT')
        self.assertEqual(raw.get('longName'), 'Apple Inc.')
        with self.assertRaises(provider.ProviderError) as caught:
            instruments.project_metadata(raw, Q, AT)
        self.assertEqual(caught.exception.kind, 'malformed_data')

if __name__ == '__main__': unittest.main()
