import io
import json
from pathlib import Path
import subprocess
import sys
import unittest
from datetime import datetime
from zoneinfo import ZoneInfo

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
try:
    import provider
except ImportError:
    provider = None

Q = {'instrument': {'namespace': 'yahoo:symbol', 'value': 'AAPL'}, 'start': '2024-01-01', 'end': '2024-01-04', 'page_size': 1, 'cursor': None}
META = {'currency': 'USD', 'exchangeTimezoneName': 'America/New_York'}

def row(day=2, **overrides):
    result = {'Date': datetime(2024, 1, day, tzinfo=ZoneInfo('America/New_York')), 'Open': 10.1, 'High': 12., 'Low': 9., 'Close': 11., 'Adj Close': 10.8, 'Volume': 9007199254740993}
    result.update(overrides)
    return result

class ProviderTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(provider, 'market data provider must be implemented')
        self.calls = []
        self.rows = [row(3), row(2)]
        def fetch(symbol, options):
            self.calls.append((symbol, options))
            return self.rows, dict(META)
        self.p = provider.Provider(fetch=fetch, clock=lambda: '2026-09-09T00:00:00Z')
        self.p.initialize({'protocol_version': 1, 'config': {}})

    def test_snapshot_inclusive_options_and_precision(self):
        page = self.p.daily(Q)
        self.assertEqual(page['items'][0]['date'], '2024-01-02')
        self.assertEqual(page['items'][0]['volume'], 9007199254740993)
        self.assertEqual(page['items'][0]['open'], '10.1')
        self.assertEqual(page['items'][0]['precision'], 'binary_float_source')
        self.assertEqual(page['items'][0]['price_basis'], 'source_reported')
        self.assertEqual(page['coverage'], {'first_date':'2024-01-02','last_date':'2024-01-03','completeness':'unverified'})
        self.assertEqual(self.calls[0], ('AAPL', dict(interval='1d', start='2024-01-01',end='2024-01-05',auto_adjust=False,back_adjust=False,repair=False,rounding=False,actions=False,keepna=True,timeout=10)))
        page2 = self.p.daily(dict(Q, cursor=page['next_cursor']))
        self.assertEqual(page2['items'][0]['date'], '2024-01-03')
        self.assertEqual(page2['coverage'], page['coverage'])
        self.assertIsNone(page2['next_cursor'])
        self.assertEqual(len(self.calls), 1)

    def test_cursor_binds_all_query_fields_and_expires(self):
        cursor = self.p.daily(Q)['next_cursor']
        for change in ({'page_size':2},{'end':'2024-01-03'},{'instrument':{'namespace':'yahoo:symbol','value':'MSFT'}}):
            with self.subTest(change=change), self.assertRaises(provider.ProviderError) as caught:
                self.p.daily(dict(Q, cursor=cursor, **change))
            self.assertEqual(caught.exception.kind, 'invalid_request')
        self.p.daily(Q)
        with self.assertRaises(provider.ProviderError): self.p.daily(dict(Q,cursor=cursor))
        cursor = self.p.daily(Q)['next_cursor']
        self.p.initialize({'protocol_version':1,'config':{}})
        with self.assertRaises(provider.ProviderError): self.p.daily(dict(Q,cursor=cursor))

    def test_invalid_rows_are_never_dropped(self):
        for change in ({'Open':float('nan')},{'Close':float('inf')},{'Low':-1},{'High':8},{'Volume':1.5},{'Volume':-1},{'Volume':2**64},{'Volume':True},{'Adj Close':float('nan')},{'Date':datetime(2024,1,2)},{'Date':datetime(2024,1,5,tzinfo=ZoneInfo('America/New_York'))}):
            self.rows = [row(**change)]
            with self.subTest(change=change), self.assertRaises(provider.ProviderError) as caught:
                self.p.daily(Q)
            self.assertEqual(caught.exception.kind,'malformed_data')
        self.rows = [row(),row()]
        with self.assertRaises(provider.ProviderError): self.p.daily(Q)

    def test_empty_uncertain_source_is_not_found(self):
        self.rows = []
        with self.assertRaises(provider.ProviderError) as caught: self.p.daily(Q)
        self.assertEqual(caught.exception.kind,'not_found')

    def test_metadata_is_required_and_local_date_retained(self):
        for meta in ({}, {'currency':'USD'},dict(META,exchangeTimezoneName='Invalid/Zone')):
            with self.subTest(meta=meta), self.assertRaises(provider.ProviderError):
                provider.normalize_rows([row()],meta,Q,'2026-09-09T00:00:00Z')
        tokyo = dict(META, currency='JPY',exchangeTimezoneName='Asia/Tokyo')
        rows = [row(Date=datetime(2024,1,2,tzinfo=ZoneInfo('Asia/Tokyo')))]
        result = provider.normalize_rows(rows,tokyo,Q,'2026-09-09T00:00:00Z')
        self.assertEqual(result[0]['date'],'2024-01-02')
        self.assertEqual(result[0]['currency'],'JPY')

    def test_null_adjusted_close_and_plain_small_decimal(self):
        self.rows=[row(Open=1e-8,Low=0,Close=1e-7,High=1e-6,**{'Adj Close':None})]
        result=self.p.daily(Q)['items'][0]
        self.assertEqual(result['open'],'0.00000001')
        self.assertIsNone(result['adjusted_close'])

    def test_validates_before_transport(self):
        for change in ({'page_size':True},{'start':'2024-1-01'},{'end':'9999-12-31'},{'end':'2023-12-31'},{'instrument':{'namespace':'sec:cik','value':'AAPL'}}):
            with self.subTest(change=change), self.assertRaises(provider.ProviderError): self.p.daily(dict(Q,**change))
        self.assertEqual(self.calls,[])

    def test_dispatch_and_stdout_isolation(self):
        p=provider.Provider(fetch=lambda *args: (print('library chatter') or [row()],META))
        response=provider.handle(p,{'jsonrpc':'2.0','id':1,'method':'market_data.daily','params':Q})
        self.assertEqual(response['error']['data']['kind'],'configuration')
        provider.handle(p,{'jsonrpc':'2.0','id':2,'method':'initialize','params':{'protocol_version':1,'config':{}}})
        from contextlib import redirect_stdout, redirect_stderr
        out,err=io.StringIO(),io.StringIO()
        with redirect_stdout(out),redirect_stderr(err):
            response=provider.handle(p,{'jsonrpc':'2.0','id':3,'method':'market_data.daily','params':Q})
        self.assertIn('result',response)
        self.assertEqual(out.getvalue(),'')
        self.assertIn('library chatter',err.getvalue())
        self.assertEqual(provider.handle(p,{'jsonrpc':'2.0','id':True,'method':'initialize'})['error']['code'],-32600)

    def test_typed_transport_failures(self):
        from recovery import Recovery
        for kind in ('rate_limited','timeout','unavailable','not_found','malformed_data'):
            # Independent failure scenarios must not share a source cooldown.
            self.p.recovery = Recovery(sleep=lambda _: None, jitter=lambda: 0)
            def fetch(*args): raise provider.ProviderError(kind,'fixture failure')
            self.p.fetch=fetch
            with self.subTest(kind=kind), self.assertRaises(provider.ProviderError) as caught: self.p.daily(Q)
            self.assertEqual(caught.exception.kind,kind)

    def test_frame_tuple_conversion_preserves_volume(self):
        class Frame:
            columns = ['Open','High','Low','Close','Adj Close','Volume']
            def __len__(self): return 1
            def itertuples(self,index,name):
                assert index is True and name is None
                return iter([(row()['Date'],10.1,12.,9.,11.,10.8,9007199254740993)])
        rows=provider.frame_rows(Frame())
        self.assertEqual(rows[0]['Volume'],9007199254740993)
        self.assertIs(type(rows[0]['Volume']),int)
        self.assertEqual(provider.normalize_rows(rows,META,Q,'2026-09-09T00:00:00Z')[0]['volume'],9007199254740993)

    def test_snapshot_bound_and_consumed_cursor(self):
        from unittest.mock import patch
        with patch.object(provider,'MAX_ROWS',1), self.assertRaises(provider.ProviderError) as caught:
            self.p.daily(Q)
        self.assertEqual(caught.exception.kind,'malformed_data')
        with patch.object(provider,'MAX_SNAPSHOT_BYTES',10), self.assertRaises(provider.ProviderError): self.p.daily(Q)
        cursor=self.p.daily(Q)['next_cursor']
        self.p.daily(dict(Q,cursor=cursor))
        with self.assertRaises(provider.ProviderError): self.p.daily(dict(Q,cursor=cursor))

    def test_main_protocol_works_without_dependency(self):
        payload='not json\n'+json.dumps({'jsonrpc':'2.0','id':7,'method':'initialize','params':{'protocol_version':1,'config':{}}})+'\n'
        proc=subprocess.run([sys.executable,str(ROOT/'main.py')],input=payload,text=True,capture_output=True,check=True)
        responses=[json.loads(line) for line in proc.stdout.splitlines()]
        self.assertEqual(responses[0]['error']['code'],-32700)
        self.assertEqual(responses[1]['result']['capabilities'],{'market_data':1, 'instrument_lookup':1})

if __name__=='__main__': unittest.main()
