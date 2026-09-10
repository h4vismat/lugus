import hashlib
import json
import unittest
from test_provider import Fixture, STAMP
import provider as p

URL = 'https://www.sec.gov/files/company_tickers_exchange.json'
DIRECTORY = {'fields':['ticker','exchange','name','cik'], 'data':[
    ['IBM','NYSE','International Business Machines',51143],
    ['IBM.A','NYSE','International Business Machines',51143],
    ['IBM','Other','Other IBM',123],
    ['PLTR','Nasdaq','Palantir Technologies',1321655],
]}
def identifier(value, namespace='sec:ticker'):
    return {'namespace':namespace,'value':value}
def query(value='IBM', **extra):
    return dict(query={'kind':'identifier','identifier':identifier(value),'exchange':None},page_size=1,cursor=None,**extra)

class ResolutionTests(unittest.TestCase):
    def plugin(self, data=DIRECTORY):
        fixture=Fixture({URL:json.dumps(data).encode()})
        plugin=p.Provider(transport=fixture,clock=lambda:STAMP)
        plugin.call('initialize',{'protocol_version':1,'config':{'user_agent':'Tests test@example.com'}})
        return plugin, fixture
    def test_capability_and_ticker_snapshot(self):
        plugin,fixture=self.plugin()
        result=plugin.call('initialize',{'protocol_version':1,'config':{'user_agent':'Tests test@example.com'}})
        self.assertEqual(result['capabilities']['company_resolution'],1)
        first=plugin.call('company_resolution.search',query(' ibm '))
        fixture.data[URL]=b'{}'
        second=plugin.call('company_resolution.search',dict(query(' ibm '),cursor=first['next_cursor']))
        self.assertEqual(first['snapshot'],second['snapshot'])
        self.assertIsNone(second['next_cursor'])
        item=first['items'][0]
        self.assertEqual(item['identifier'],identifier('0000051143','sec:cik'))
        self.assertEqual(len(item['listings']),2)
        self.assertEqual(item['source_checksum'],hashlib.sha256(json.dumps(DIRECTORY).encode()).hexdigest())
        self.assertEqual(item['retrieved_at'],STAMP)
        self.assertEqual(item['match_reasons'],['exact_identifier'])
        self.assertEqual(len(fixture.calls),1)
    def test_name_exact_before_substring_and_punctuation_preserved(self):
        plugin,_=self.plugin()
        result=plugin.call('company_resolution.search',dict(query(),query={'kind':'name','text':' other   ibm '}))
        self.assertEqual(result['items'][0]['match_reasons'],['exact_name'])
        result=plugin.call('company_resolution.search',query('IBM.A'))
        self.assertEqual(len(result['items']),1)
        self.assertEqual(plugin.call('company_resolution.search',query('IBM-A'))['items'],[])
    def test_exchange_qualifier_and_cursor_invalidation(self):
        plugin,fixture=self.plugin()
        first=plugin.call('company_resolution.search',query())
        altered=dict(query(),cursor=first['next_cursor'],page_size=2)
        with self.assertRaises(p.ProviderError): plugin.call('company_resolution.search',altered)
        plugin.call('company_resolution.search',query('PLTR'))
        with self.assertRaises(p.ProviderError): plugin.call('company_resolution.search',dict(query(),cursor=first['next_cursor']))
        request=query(); request['query']['exchange']=identifier('NYSE','sec:exchange')
        self.assertEqual(len(plugin.call('company_resolution.search',request)['items']),1)
    def test_lookup_and_cik_search(self):
        plugin,fixture=self.plugin()
        url='https://data.sec.gov/submissions/CIK0000051143.json'
        fixture.data[url]=json.dumps({'cik':'51143','name':'IBM','tickers':['IBM'],'exchanges':['NYSE']}).encode()
        item=plugin.call('company_resolution.lookup',{'identifier':identifier('51143','sec:cik')})
        self.assertEqual(item['identifier']['value'],'0000051143')
        self.assertEqual(item['match_reasons'],[])
        request=query(); request['query']['identifier']=identifier('51143','sec:cik')
        self.assertEqual(plugin.call('company_resolution.search',request)['items'][0]['match_reasons'],['exact_identifier'])
        fixture.data[url]=json.dumps({'cik':999,'name':'IBM','tickers':[],'exchanges':[]}).encode()
        with self.assertRaises(p.ProviderError) as error: plugin.call('company_resolution.lookup',{'identifier':identifier('51143','sec:cik')})
        self.assertEqual(error.exception.kind,'malformed_data')
    def test_request_errors_precede_io(self):
        plugin,fixture=self.plugin()
        requests=[dict(query(),page_size=v) for v in (0,101,True)]
        requests += [dict(query(),query={'kind':'name','text':s}) for s in ('','é'*129)]
        requests += [dict(query(),query={'kind':'identifier','identifier':identifier(v,'sec:cik')}) for v in ('0','-1','12345678901','１２')]
        requests += [dict(query(),cursor='x'*129),dict(query(),query={'kind':'identifier','identifier':identifier('IBM','yahoo:symbol')})]
        for request in requests:
            with self.subTest(request=request), self.assertRaises(p.ProviderError): plugin.call('company_resolution.search',request)
        self.assertEqual(fixture.calls,[])
    def test_malformed_directory_is_not_empty(self):
        malformed=[{}, {'fields':['cik','name','ticker','ticker'],'data':[]},
            {'fields':['cik','name','ticker','exchange'],'data':[[1,'Name','T']]},
            {'fields':['cik','name','ticker','exchange'],'data':[[True,'Name','T','NYSE']]},
            {'fields':['cik','name','ticker','exchange'],'data':[[1,'Name','T','NYSE'],[1,'Different','X','NYSE']]}]
        for payload in malformed:
            plugin,_=self.plugin(payload)
            with self.subTest(payload=payload), self.assertRaises(p.ProviderError) as error: plugin.call('company_resolution.search',query())
            self.assertEqual(error.exception.kind,'malformed_data')
    def test_extra_fields_and_unicode_controls_rejected_before_io(self):
        plugin,fixture=self.plugin()
        requests=[dict(query(),extra=True)]
        for section in ('query','identifier','exchange'):
            request=query()
            if section=='query': request['query']['extra']=True
            elif section=='identifier': request['query']['identifier']['extra']=True
            else: request['query']['exchange']=dict(identifier('NYSE','sec:exchange'),extra=True)
            requests.append(request)
        requests.extend(dict(query(),query={'kind':'name','text':value}) for value in ('IBM\x7f','IBM\x85','IBM\ud800'))
        for request in requests:
            with self.subTest(request=request), self.assertRaises(p.ProviderError) as error:
                plugin.call('company_resolution.search',request)
            self.assertEqual(error.exception.kind,'invalid_request')
        with self.assertRaises(p.ProviderError):
            plugin.call('company_resolution.lookup',{'identifier':identifier('1','sec:cik'),'extra':True})
        self.assertEqual(fixture.calls,[])

    def test_resolution_requires_initialize_and_reinitialization_expires_cursor(self):
        with self.assertRaises(p.ProviderError) as error:
            p.Provider(transport=Fixture({})).call('company_resolution.search',query())
        self.assertEqual(error.exception.kind,'configuration')
        plugin,fixture=self.plugin()
        page=plugin.call('company_resolution.search',query())
        plugin.call('initialize',{'protocol_version':1,'config':{'user_agent':'Tests test@example.com'}})
        with self.assertRaises(p.ProviderError) as error:
            plugin.call('company_resolution.search',dict(query(),cursor=page['next_cursor']))
        self.assertEqual(error.exception.kind,'invalid_request')
        self.assertEqual(len(fixture.calls),1)
    def test_source_errors_retain_kind_and_duplicate_listings_deduplicate(self):
        payload=json.loads(json.dumps(DIRECTORY))
        payload['data'].append(payload['data'][0])
        plugin,fixture=self.plugin(payload)
        self.assertEqual(len(plugin.call('company_resolution.search',query())['items'][0]['listings']),2)
        fixture.data[URL]=p.ProviderError('rate_limited','busy',retry_after_seconds=2)
        response=p.handle(plugin,{'jsonrpc':'2.0','id':1,'method':'company_resolution.search','params':query()})
        self.assertEqual(response['error']['data'],{'kind':'rate_limited','retry_after_seconds':2})
    def test_cik_search_not_found_is_scoped_empty_and_ticker_lookup_unsupported(self):
        plugin,fixture=self.plugin()
        fixture.data['https://data.sec.gov/submissions/CIK0000000001.json']=p.ProviderError('not_found','missing')
        request=query(); request['query']['identifier']=identifier('1','sec:cik')
        result=plugin.call('company_resolution.search',request)
        self.assertEqual(result['items'],[])
        self.assertIn('0000000001',result['coverage'])
        self.assertIsNone(result['next_cursor'])
        with self.assertRaises(p.ProviderError) as error:
            plugin.call('company_resolution.lookup',{'identifier':identifier('IBM')})
        self.assertEqual(error.exception.kind,'unsupported')

    def test_lookup_mismatched_arrays_and_not_found(self):
        plugin,fixture=self.plugin(); url='https://data.sec.gov/submissions/CIK0000000001.json'
        fixture.data[url]=json.dumps({'cik':1,'name':'Name','tickers':['X'],'exchanges':[]}).encode()
        request={'identifier':identifier('1','sec:cik')}
        with self.assertRaises(p.ProviderError) as error: plugin.call('company_resolution.lookup',request)
        self.assertEqual(error.exception.kind,'malformed_data')
        fixture.data[url]=p.ProviderError('not_found','missing')
        with self.assertRaises(p.ProviderError) as error: plugin.call('company_resolution.lookup',request)
        self.assertEqual(error.exception.kind,'not_found')

if __name__=='__main__': unittest.main()
