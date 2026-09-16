import sys
import unittest
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import provider
from test_provider import row, Q, META
from recovery import Recovery
from common import ProviderError

class UnlimitedTests(unittest.TestCase):
    def test_snapshot_caps_timeout_and_default_reset(self):
        calls=[]
        def fetch(symbol,options):
            calls.append(options)
            return [row(2),row(3)],META
        p=provider.Provider(fetch=fetch)
        p.initialize({'protocol_version':1,'config':{'unlimited_research':True}})
        with patch.object(provider,'MAX_ROWS',1),patch.object(provider,'MAX_SNAPSHOT_BYTES',1):
            first=p.daily(Q)
            self.assertIsNotNone(first['next_cursor'])
            self.assertEqual(p.daily(dict(Q,cursor=first['next_cursor']))['items'][0]['date'],'2024-01-03')
            self.assertIsNone(calls[0]['timeout'])
            p.initialize({'protocol_version':1,'config':{}})
            with self.assertRaises(ProviderError): p.daily(Q)
        self.assertEqual(calls[-1]['timeout'],10)
        self.assertFalse(p.recovery.unlimited_research)

    def test_recovery_does_not_expire_total_budget(self):
        now=[0]; attempts=[]
        def operation():
            attempts.append(1); now[0]+=100
            if len(attempts)<3: raise ProviderError('unavailable','fixture')
            return 'ok'
        r=Recovery(sleep=lambda delay:None,monotonic=lambda:now[0],jitter=lambda:0,unlimited_research=True)
        self.assertEqual(r.run(operation),'ok')
        self.assertEqual(len(attempts),3)

    def test_history_range_and_normalization_caps_are_opt_in(self):
        import history
        query=dict(instrument=dict(namespace='yahoo:symbol',value='TEST'),start='2026-01-02',end='2026-01-02',anchor='2026-01-02')
        calendar=[dict(date='2026-01-02',market_close='2026-01-02T21:00:00Z')]
        rows=[dict(date='2026-01-02',close='100',split='0')]
        meta=dict(currency='USD',exchangeTimezoneName='America/New_York',calendar='NYSE')
        with patch.object(history,'MAX_ROWS',0),patch.object(history,'MAX_BYTES',1):
            with self.assertRaises(ProviderError): history.history_params(query)
            history.history_params(query,unlimited_research=True)
            result=history.normalize_history(rows,meta,query,'2026-01-02T22:00:00Z',calendar,unlimited_research=True)
            self.assertEqual(len(result['items']),1)
            with self.assertRaises(ProviderError): history.normalize_history(rows,meta,query,'2026-01-02T22:00:00Z',calendar)

    def test_dependency_session_overrides_internal_timeouts_without_network(self):
        import transport
        from transport import ticker_for_research
        from curl_cffi.requests import Session
        from types import SimpleNamespace
        captured=[]
        yf=SimpleNamespace(Ticker=lambda symbol,session:session)
        session=ticker_for_research(yf,'TEST',True)
        try:
            with patch.object(Session,'request',lambda self,method,url,**kwargs:captured.append(kwargs)):
                session.get('https://example.invalid',timeout=10)
                session.post('https://example.invalid',timeout=30)
                bounded=ticker_for_research(yf,'TEST',False)
                bounded.get('https://example.invalid',timeout=10)
            self.assertEqual([entry['timeout'] for entry in captured],[None,None,10])
        finally:
            session.close()
            transport._research_session=None
