import io
import json
import sys
import unittest
from pathlib import Path
sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
try:
    import provider as p
except ModuleNotFoundError:
    p = None

QUERY = dict(company={'namespace':'sec:cik','value':'0000320193'},filed_from='2023-01-01',filed_to='2024-12-31',forms=[],cursor=None,page_size=1)
STAMP = '2026-09-09T12:00:00Z'
FACTS = b'''{"cik":320193,"facts":{"us-gaap":{"Assets":{"label":"Assets","units":{"USD":[{"val":12345678901234567890.123456789,"end":"2023-09-30","accn":"0000320193-23-000106","form":"10-K","filed":"2023-11-03","fy":2023,"fp":"FY"},{"val":1e-7,"end":"2023-09-30","accn":"0000320193-24-000106","form":"10-K","filed":"2024-11-03"}]}}},"ifrs-full":{"Unmapped":{"units":{"EUR":[{"val":2,"start":"2023-01-01","end":"2023-12-31","accn":"0000320193-24-000107","form":"20-F","filed":"2024-02-01"}]}}}}}'''
def rows(day='2023-11-03', accession='0000320193-23-000106'):
    return dict(accessionNumber=[accession],filingDate=[day],form=['10-K'],reportDate=['2023-09-30'],acceptanceDateTime=['2023-11-03T12:00:00'],primaryDocument=['aapl.htm'])
class Fixture:
    def __init__(self, data): self.data=data; self.calls=[]
    def get(self,url,max_bytes,archive=False):
        self.calls.append(url)
        value=self.data[url]
        if isinstance(value, Exception): raise value
        if len(value)>max_bytes: raise p.ProviderError('malformed_data','too large')
        return value,'text/html'
class ProviderTests(unittest.TestCase):
    def setUp(self): self.assertIsNotNone(p, 'SEC provider is implemented')
    def plugin(self,fixture):
        plugin=p.Provider(transport=fixture,clock=lambda:STAMP)
        plugin.call('initialize',{'protocol_version':1,'config':{'user_agent':'Lugus Test test@example.com'}})
        return plugin
    def test_primary_document_allows_relative_subdirectories_without_traversal(self):
        payload=rows()
        payload['primaryDocument']=['xslF345X06/form4.xml']
        filing=p.decode_filings(payload,QUERY,STAMP)[0]
        self.assertEqual(filing['primary_document'],'xslF345X06/form4.xml')
        self.assertTrue(filing['source_url'].endswith('/xslF345X06/form4.xml'))
        for path in ('../report.xml','/report.xml','x/../report.xml','x/%2e%2e/report.xml','x\\report.xml','x//report.xml'):
            with self.subTest(path=path):
                payload['primaryDocument']=[path]
                with self.assertRaises(p.ProviderError): p.decode_filings(payload,QUERY,STAMP)

    def test_exact_decimals_unmapped_ifrs_and_repeat_disclosures(self):
        facts=p.decode_facts(p.parse_json(FACTS),QUERY,'https://data.sec.gov/facts',STAMP)
        self.assertEqual([f['value'] for f in facts],['12345678901234567890.123456789','0.0000001','2'])
        self.assertEqual(facts[2]['namespace'],'ifrs-full')
        self.assertEqual(facts[2]['period'],{'kind':'duration','start':'2023-01-01','end':'2023-12-31'})
        self.assertIsNone(facts[2]['label'])
    def test_filters_and_empty_facts(self):
        query=dict(QUERY,forms=['20-F'])
        self.assertEqual(len(p.decode_facts(p.parse_json(FACTS),query,'url',STAMP)),1)
        self.assertEqual(p.decode_facts({'cik':320193,'facts':{}},QUERY,'url',STAMP),[])
    def test_optional_labels_preserve_facts_when_empty_null_or_missing(self):
        for label in ('', None, 'Reported assets'):
            with self.subTest(label=label):
                payload=p.parse_json(FACTS)
                payload['facts']['us-gaap']['Assets']['label']=label
                facts=p.decode_facts(payload,QUERY,'url',STAMP)
                self.assertEqual(len(facts),3)
                self.assertEqual(facts[0]['label'],None if label=='' else label)
                self.assertEqual(facts[0]['value'],'12345678901234567890.123456789')
                self.assertIsNone(facts[2]['label'])
    def test_optional_labels_reject_invalid_types(self):
        for label in (0, False, [], {}):
            with self.subTest(label=label):
                payload=p.parse_json(FACTS)
                payload['facts']['us-gaap']['Assets']['label']=label
                with self.assertRaises(p.ProviderError) as caught:
                    p.decode_facts(payload,QUERY,'url',STAMP)
                self.assertEqual(caught.exception.kind,'malformed_data')
    def test_malformed_source_does_not_become_empty(self):
        for data in ({}, {'facts':[]}, {'facts':{'us-gaap':{'Assets':{'units':{'USD':[{'val':None}]}}}}}):
            with self.assertRaises(p.ProviderError) as err: p.decode_facts(data,QUERY,'url',STAMP)
            self.assertEqual(err.exception.kind,'malformed_data')
        with self.assertRaises(p.ProviderError): p.parse_json(b'{"val":NaN}')
    def test_old_submission_pages_and_unknown_acceptance_zone(self):
        root={'cik':'0000320193','filings':{'recent':rows('2024-11-03'),'files':[{'name':'CIK0000320193-submissions-001.json','filingFrom':'2023-01-01','filingTo':'2023-12-31'}]}}
        fixture=Fixture({'https://data.sec.gov/submissions/CIK0000320193.json':json.dumps(root).encode(),'https://data.sec.gov/submissions/CIK0000320193-submissions-001.json':json.dumps(rows()).encode()})
        plugin=self.plugin(fixture)
        first=plugin.call('filings.list',QUERY)
        second=plugin.call('filings.list',dict(QUERY,cursor=first['next_cursor']))
        self.assertEqual(len(first['items'])+len(second['items']),2)
        self.assertIsNone(second['items'][0]['accepted_at'])
        self.assertEqual(second['items'][0]['source_url'],'https://www.sec.gov/Archives/edgar/data/320193/000032019323000106/aapl.htm')
        self.assertEqual(len(fixture.calls),2)
    def test_reversed_historical_range_is_malformed_before_overlap_filter(self):
        root={'cik':'0000320193','filings':{'recent':rows('2025-01-01'),'files':[{'name':'CIK0000320193-submissions-001.json','filingFrom':'2025-01-01','filingTo':'2020-01-01'}]}}
        fixture=Fixture({'https://data.sec.gov/submissions/CIK0000320193.json':json.dumps(root).encode()})
        plugin=self.plugin(fixture)
        with self.assertRaises(p.ProviderError) as error:
            plugin.call('filings.list',QUERY)
        self.assertEqual(error.exception.kind,'malformed_data')

    def test_cursor_bound_to_query_method_and_session(self):
        fixture=Fixture({'https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json':FACTS})
        plugin=self.plugin(fixture)
        first=plugin.call('fundamentals.facts',QUERY)
        for method,query in [('filings.list',QUERY),('fundamentals.facts',dict(QUERY,forms=['20-F']))]:
            with self.assertRaises(p.ProviderError): plugin.call(method,dict(query,cursor=first['next_cursor']))
        with self.assertRaises(p.ProviderError): self.plugin(fixture).call('fundamentals.facts',dict(QUERY,cursor=first['next_cursor']))
        self.assertEqual(plugin.call('fundamentals.facts',dict(QUERY,cursor=first['next_cursor']))['items'][0]['value'],'0.0000001')
        self.assertEqual(len(fixture.calls),1)
    def test_request_validation_before_network(self):
        fixture=Fixture({}); plugin=self.plugin(fixture)
        for query in [dict(QUERY,page_size=0),dict(QUERY,page_size=True),dict(QUERY,filed_from='2025-01-01'),dict(QUERY,company={'namespace':'sec:cik','value':'../x'})]:
            with self.assertRaises(p.ProviderError): plugin.call('filings.list',query)
        self.assertEqual(fixture.calls,[])
        with self.assertRaises(p.ProviderError): p.Provider().call('initialize',{'protocol_version':1,'config':{'user_agent':'anonymous'}})
    def test_document_bounds_and_archive_validation(self):
        url='https://www.sec.gov/Archives/edgar/data/320193/000032019323000106/aapl.htm'
        plugin=self.plugin(Fixture({url:b'<html/>'}))
        self.assertEqual(plugin.call('filings.document',{'source_url':url,'max_bytes':100})['content_base64'],'PGh0bWwvPg==')
        with self.assertRaises(p.ProviderError): plugin.call('filings.document',{'source_url':url,'max_bytes':2})
        for url in ['https://evil.com/Archives/edgar/data/x','http://www.sec.gov/Archives/edgar/data/x','https://www.sec.gov/Archives/edgar/data/../x','https://www.sec.gov@evil.com/Archives/edgar/data/x']:
            with self.assertRaises(p.ProviderError): p.validate_url(url,True)
    def test_source_failure_is_typed(self):
        fixture=Fixture({'https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json':p.ProviderError('rate_limited','busy',retry_after_seconds=2)})
        response=p.handle(self.plugin(fixture),{'jsonrpc':'2.0','id':4,'method':'fundamentals.facts','params':QUERY})
        self.assertEqual(response['error']['data'],{'kind':'rate_limited','retry_after_seconds':2})
        self.assertEqual(response['id'],4)
    def test_rpc_validation(self):
        plugin=self.plugin(Fixture({}))
        for request in [[],{'jsonrpc':'2.0','method':'filings.list'},{'jsonrpc':'2.0','id':True,'method':'x'}]:
            self.assertEqual(p.handle(plugin,request)['error']['code'],-32600)
        self.assertEqual(p.handle(plugin,{'jsonrpc':'2.0','id':1,'method':'x','params':{}})['error']['code'],-32601)


class TransportTests(unittest.TestCase):
    def test_forbidden_requires_configuration_attention_without_retry(self):
        from urllib.error import HTTPError
        from email.message import Message
        class Opener:
            def open(self, *args, **kwargs):
                raise HTTPError('https://data.sec.gov/test', 403, 'denied', Message(), None)
        transport = p.HttpTransport('Test test@example.com', opener_factory=lambda _: Opener(),
                                    sleep=lambda _: self.fail('access denial must not retry'))
        with self.assertRaises(p.ProviderError) as caught:
            transport.get('https://data.sec.gov/test', 100)
        self.assertEqual(caught.exception.kind, 'configuration')

    def test_long_rate_limit_blocks_later_requests_until_cooldown_expires(self):
        from urllib.error import HTTPError
        from email.message import Message
        headers = Message(); headers['Retry-After'] = '60'
        now = [0]; calls = []
        class Opener:
            def open(self, *args, **kwargs):
                calls.append(1)
                raise HTTPError('https://data.sec.gov/test', 429, 'busy', headers, None)
        transport = p.HttpTransport('Test test@example.com', opener_factory=lambda _: Opener(),
                                    monotonic=lambda: now[0], sleep=lambda _: self.fail('no early retry'))
        with self.assertRaises(p.ProviderError): transport.get('https://data.sec.gov/test', 100)
        now[0] = 20
        with self.assertRaises(p.ProviderError) as caught: transport.get('https://data.sec.gov/test', 100)
        self.assertEqual(caught.exception.retry_after_seconds, 40)
        self.assertEqual(len(calls), 1)
        now[0] = 60
        with self.assertRaises(p.ProviderError): transport.get('https://data.sec.gov/test', 100)
        self.assertEqual(len(calls), 2)

    def test_expired_deadline_after_limiter_does_not_start_network_request(self):
        now = [0]
        class Opener:
            def open(inner, *args, **kwargs): self.fail('deadline expired before IO')
        def sleep(delay): now[0] += 41
        transport = p.HttpTransport('Test test@example.com', opener_factory=lambda _: Opener(),
                                    monotonic=lambda: now[0], sleep=sleep)
        transport.next_request = 1
        with self.assertRaises(p.ProviderError) as caught:
            transport.get('https://data.sec.gov/test', 100)
        self.assertEqual(caught.exception.kind, 'timeout')

    def test_redirect_passes_shared_request_limiter(self):
        from urllib.request import Request
        calls=[]
        handler=p.SafeRedirect(True)
        handler.before_request=lambda:calls.append('limited')
        redirected=handler.redirect_request(Request('https://www.sec.gov/Archives/edgar/data/a'),None,302,'',{},'https://www.sec.gov/Archives/edgar/data/b')
        self.assertEqual(redirected.full_url,'https://www.sec.gov/Archives/edgar/data/b')
        self.assertEqual(calls,['limited'])
    def test_timeout_wrapped_in_urlerror_stays_timeout(self):
        from urllib.error import URLError
        import socket
        class Opener:
            def open(self,*args,**kwargs): raise URLError(socket.timeout('timed out'))
        transport=p.HttpTransport('Test test@example.com',opener_factory=lambda archive:Opener(),sleep=lambda n:None)
        with self.assertRaises(p.ProviderError) as error: transport.get('https://data.sec.gov/test',100)
        self.assertEqual(error.exception.kind,'timeout')
    def test_retries_rate_limit_honors_delay_and_succeeds(self):
        from urllib.error import HTTPError
        from email.message import Message
        headers=Message(); headers['Retry-After']='2'; headers['Content-Type']='application/json'
        class Response(io.BytesIO):
            def __init__(self): super().__init__(b'{}'); self.headers=headers
            def geturl(self): return 'https://data.sec.gov/test'
        class Opener:
            calls=0
            def open(self,*args,**kwargs):
                self.calls+=1
                if self.calls<3: raise HTTPError('https://data.sec.gov/test',429,'busy',headers,None)
                return Response()
        opener=Opener(); sleeps=[]; now=[0]
        def sleep(n): sleeps.append(n); now[0]+=n
        transport=p.HttpTransport('Test test@example.com',opener_factory=lambda archive:opener,sleep=sleep,monotonic=lambda:now[0])
        self.assertEqual(transport.get('https://data.sec.gov/test',100),(b'{}','application/json'))
        self.assertEqual(sleeps,[2,2])
    def test_long_retry_guidance_returned_without_early_retry(self):
        from urllib.error import HTTPError
        from email.message import Message
        headers=Message(); headers['Retry-After']='60'
        class Opener:
            def open(self,*args,**kwargs): raise HTTPError('https://data.sec.gov/test',429,'busy',headers,None)
        transport=p.HttpTransport('Test test@example.com',opener_factory=lambda archive:Opener(),sleep=lambda n:self.fail('must not retry early'))
        with self.assertRaises(p.ProviderError) as error: transport.get('https://data.sec.gov/test',100)
        self.assertEqual(error.exception.retry_after_seconds,60)
    def test_oversized_stream_and_offsite_redirect_rejected(self):
        from email.message import Message
        from urllib.request import Request
        class Response(io.BytesIO):
            headers=Message()
            def geturl(self): return 'https://data.sec.gov/test'
        class Opener:
            def open(self,*args,**kwargs): return Response(b'abcdef')
        transport=p.HttpTransport('Test test@example.com',opener_factory=lambda archive:Opener())
        with self.assertRaises(p.ProviderError): transport.get('https://data.sec.gov/test',3)
        with self.assertRaises(p.ProviderError): p.SafeRedirect(True).redirect_request(Request('https://www.sec.gov/Archives/edgar/data/x'),None,302,'',{},'https://evil.com/x')
    def test_process_recovers_after_parse_error(self):
        import subprocess
        directory=Path(__file__).resolve().parents[1]
        messages=['not-json',json.dumps({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocol_version':1,'config':{'user_agent':'Test test@example.com'}}})]
        result=subprocess.run([sys.executable,str(directory/'main.py')],input='\n'.join(messages)+'\n',text=True,capture_output=True,timeout=5,check=True)
        responses=[json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual(responses[0]['error']['code'],-32700)
        self.assertEqual(responses[1]['result']['capabilities'],{'filings':1,'fundamentals':1,'company_resolution':1})
        self.assertEqual(result.stderr,'')

class ConfigurationTests(unittest.TestCase):
    def test_company_identity_requires_zero_padded_cik(self):
        with self.assertRaises(p.ProviderError):
            p.query_params(dict(QUERY,company={'namespace':'sec:cik','value':'320193'}))
    def test_reinitialize_replaces_identity_and_expires_cursors(self):
        plugin=p.Provider()
        for email in ['first@example.com','second@example.com']:
            plugin.call('initialize',{'protocol_version':1,'config':{'user_agent':'Lugus '+email}})
        self.assertEqual(plugin.transport.user_agent,'Lugus second@example.com')
    def test_transport_does_not_accept_truncated_document(self):
        from email.message import Message
        class Response(io.BytesIO):
            headers=Message(); headers['Content-Length']='10'
            def geturl(self): return 'https://www.sec.gov/Archives/edgar/data/1/a.htm'
        class Opener:
            def open(self,*args,**kwargs): return Response(b'abc')
        transport=p.HttpTransport('Test test@example.com',opener_factory=lambda archive:Opener())
        with self.assertRaises(p.ProviderError) as error: transport.get('https://www.sec.gov/Archives/edgar/data/1/a.htm',100,archive=True)
        self.assertEqual(error.exception.kind,'malformed_data')

if __name__=='__main__': unittest.main()
