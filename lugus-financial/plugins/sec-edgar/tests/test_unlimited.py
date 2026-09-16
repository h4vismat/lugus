import io
import sys
import unittest
from email.message import Message
from pathlib import Path
from unittest.mock import patch
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import provider as p
from resolution import Resolution

URL = 'https://www.sec.gov/Archives/edgar/data/320193/report.htm'
CONFIG = {'user_agent': 'Lugus test@example.com', 'unlimited_research': True}

class UnlimitedTests(unittest.TestCase):
    def test_large_document_limit_and_uncapped_resolution(self):
        calls=[]
        class Transport:
            def get(self,url,maximum,archive=False):
                calls.append(maximum)
                return b'{}','text/plain'
        provider=p.Provider(transport=Transport())
        provider.call('initialize',{'protocol_version':1,'config':CONFIG})
        provider.call('filings.document',{'source_url':URL,'max_bytes':9223372036854775807})
        provider.call('filings.document',{'source_url':URL})
        with patch('resolution.MAX_SOURCE',1):
            provider.resolution.fetch('https://data.sec.gov/test')
        self.assertEqual(calls,[9223372036854775807,None,None])
        provider.call('initialize',{'protocol_version':1,'config':dict(CONFIG,unlimited_research=False)})
        with self.assertRaises(p.ProviderError):
            provider.call('filings.document',{'source_url':URL,'max_bytes':9223372036854775807})

    def test_slow_source_has_no_deadline_and_unbounded_reads_use_chunks(self):
        now=[0]; timeouts=[]; reads=[]
        class Response(io.BytesIO):
            headers=Message()
            def geturl(self): return URL
            def read(self,size):
                reads.append(size); now[0]+=100
                return super().read(size)
        class Opener:
            def open(self,request,timeout):
                timeouts.append(timeout)
                return Response(b'content')
        transport=p.HttpTransport(CONFIG['user_agent'],opener_factory=lambda _:Opener(),
            monotonic=lambda:now[0],sleep=lambda _:None,unlimited_research=True)
        self.assertEqual(transport.get(URL,None)[0],b'content')
        self.assertEqual(timeouts,[None])
        self.assertEqual(reads,[65536,65536])

    def test_unlimited_line_framing_is_instance_scoped(self):
        import main
        init={'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocol_version':1,'config':CONFIG}}
        import json
        second={'jsonrpc':'2.0','id':2,'method':'unknown','params':{'padding':'x'*1000}}
        stream=io.BytesIO((json.dumps(init)+'\n'+json.dumps(second)+'\n').encode())
        from types import SimpleNamespace
        out=io.StringIO()
        with patch.object(main.sys,'stdin',SimpleNamespace(buffer=stream)), patch.object(main.sys,'stdout',out), patch.object(main,'MAX_LINE',300):
            main.main()
        responses=[json.loads(line) for line in out.getvalue().splitlines()]
        self.assertIn('result',responses[0])
        self.assertEqual(responses[1]['id'],2)
        self.assertEqual(responses[1]['error']['data']['kind'],'unsupported')
