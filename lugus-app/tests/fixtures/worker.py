"""Real protocol peer. Files signal response boundaries, release files unblock them."""
import datetime
from decimal import Decimal
import base64
import json
import os
import pathlib
import select
import sys

FILING_HTML = b'''<!doctype html><html><head><meta charset="utf-8"><title>Hidden title</title><style>.x{display:none}</style></head><body><h1>Risk factors</h1><p>Revenue  &amp;
 cash <b>grew</b>.</p><ix:hidden><ix:nonFraction>999999</ix:nonFraction></ix:hidden><p>S\xc3\xa3o Paulo: <ix:nonFraction>(1,234.50)</ix:nonFraction> USD</p><table><tr><th>Year</th><th>Revenue</th></tr><tr><td>2025</td><td>1,234.50</td></tr></table><script>bad()</script><template>secret</template><p hidden>hidden</p><p style="display: none">invisible</p></body></html>'''
FILING_HTML_REVISED = FILING_HTML.replace(b"Revenue  &amp;\n cash <b>grew</b>.", b"Revenue  &amp;\n cash <b>fell sharply</b>.")

for line in sys.stdin:
    req = json.loads(line)
    method, params = req['method'], req['params']
    if method == 'initialize':
        config = params['config']
        mode = config.get('mode', 'ok')
        root = pathlib.Path(config['barrier'])
        root.mkdir(exist_ok=True)
        (root / 'pid').write_text(str(os.getpid()))
        result = {'protocol_version': 2 if mode == 'startup_failure' else 1,
                  'plugin_id': config.get('plugin_id', 'worker-fixture'), 'plugin_version': config.get('version', '1'),
                  'capabilities': {'historical_prices': 1, 'filings': 1, 'fundamentals': 1, 'company_resolution': 1, 'market_data': 1, 'instrument_lookup': 2 if mode == 'instrument_unsupported' else 1}}
    else:
        cursor = params.get('cursor')
        if mode.startswith('apple_comparison') and method == 'fundamentals.facts':
            (root / 'comparison-facts').touch()
            if mode == 'apple_comparison_blocked':
                while not (root / 'release').exists():
                    ready, _, _ = select.select([sys.stdin], [], [], 0.005)
                    if ready and not os.read(sys.stdin.fileno(), 1): raise SystemExit(0)

        (root / ('second' if cursor else 'first')).touch()
        if method == 'market_data.daily': (root / 'prices-started').touch()
        if (mode == 'history_blocked' and method == 'historical_prices.daily') or (mode == 'apple_price_blocked' and method == 'market_data.daily') or mode == 'blocked' or (mode == 'search_blocked' and method == 'company_resolution.search') or (mode in ('second_blocked', 'repeated_cursor', 'market_repeated_date') and cursor):
            while not (root / 'release').exists():
                # A killed CLI cannot run normal child cleanup. During a blocked
                # response, EOF on the request pipe means its owner is gone.
                ready, _, _ = select.select([sys.stdin], [], [], 0.005)
                if ready and not os.read(sys.stdin.fileno(), 1):
                    raise SystemExit(0)
        if mode in ('source_timeout_once', 'source_unavailable_once'):
            kind = 'timeout' if mode == 'source_timeout_once' else 'unavailable'
            mode = 'ok'
            print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -32000, 'message': 'temporary source failure', 'data': {'kind': kind}}}), flush=True)
            continue
        if mode == 'protocol':
            print('invalid-json', flush=True)
            continue
        if (mode == 'apple_price_error' and method == 'market_data.daily') or mode == 'source_error' or (mode == 'search_rate_limited' and method == 'company_resolution.search'):
            print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -32000, 'message': 'source secret', 'data': {'kind': 'rate_limited', 'retry_after_seconds': 3}}}), flush=True)
            continue
        result = {'items': [], 'next_cursor': None}
        if method == 'company_resolution.search':
            result.update(snapshot='fixture-snapshot', coverage='fixture universe')
            if mode.startswith('apple'):
                result['items'] = [{'identifier': {'namespace':'sec:cik','value':'0000320193'}, 'name':'Apple Inc.', 'aliases':[], 'listings':[{'ticker':{'namespace':'sec:ticker','value':'AAPL'},'exchange':{'namespace':'sec:exchange','value':'NASDAQ'}}], 'source_url':'https://example.test/apple','source_checksum':'a'*64,'retrieved_at':'2026-09-09T00:00:00Z','match_reasons':['name_substring']}]
                if mode == 'apple_escaped': result['items'][0]['source_url'] += '\\' * 1024
                if mode == 'apple_multiple':
                    result['items'][0]['listings'].append({'ticker':{'namespace':'sec:ticker','value':'APPLB'},'exchange':{'namespace':'sec:exchange','value':'NYSE'}})
                source_input = params['query'].get('text', params['query'].get('identifier', {}).get('value', '')).upper()
                if (mode == 'apple_compare' or mode.startswith('apple_comparison')) and (source_input in ('MSFT', '0000789019') or 'MICROSOFT' in source_input):
                    result['items'][0].update(identifier={'namespace':'sec:cik','value':'0000789019'},name='Microsoft Corp.',listings=[{'ticker':{'namespace':'sec:ticker','value':'MSFT'},'exchange':{'namespace':'sec:exchange','value':'NASDAQ'}}])
                if params['query']['kind'] == 'name' and params['query']['text'].casefold() == 'apple inc.':
                    result['items'][0]['match_reasons'] = ['exact_name']
                if params['query']['kind'] == 'identifier':
                    identifier = params['query']['identifier']
                    expected = [result['items'][0]['identifier']['value']] + [l['ticker']['value'] for l in result['items'][0]['listings']]
                    if identifier['value'].upper() in expected:
                        result['items'][0]['match_reasons'] = ['exact_identifier']
                    else:
                        result['items'] = []
        elif method == 'company_resolution.lookup':
            result = {'identifier': params['identifier'], 'name': 'Fixture', 'aliases': [], 'listings': [], 'source_url': 'https://example.test/company', 'source_checksum': 'a' * 64, 'retrieved_at': '2026-09-09T00:00:00Z', 'match_reasons': []}
        elif method == 'instrument_lookup.lookup':
            result = {'instrument': {'namespace':'yahoo:symbol','value':'AAPL'},'issuer_name':'Apple Inc.','ticker':'AAPL','exchange':{'namespace':'yahoo:exchange','value':'NMS'},'kind':'equity','issuer_identifiers':[],'source_url':'https://example.test/instrument','source_checksum':'b'*64,'retrieved_at':'2026-09-09T00:00:00Z'}
            if (mode == 'apple_compare' or mode.startswith('apple_comparison')) and params['instrument']['value'] == 'MSFT':
                result.update(instrument={'namespace':'yahoo:symbol','value':'MSFT'},issuer_name='Microsoft Corp.',ticker='MSFT')
            if mode == 'apple_wrong_issuer': result['issuer_name'] = 'Another Issuer Inc.'
            if mode == 'apple_wrong_exchange': result['exchange']['value'] = 'NYQ'
            if mode == 'apple_missing': result['issuer_name'] = None
        elif method == 'historical_prices.daily':
            # Synthetic exchange schedule: no live market data is used by this peer.
            start=datetime.date.fromisoformat(params['start'])-datetime.timedelta(days=1)
            while start.weekday()>=5 or start.isoformat()=='2026-01-01': start-=datetime.timedelta(days=1)
            anchor=datetime.date.fromisoformat(params['anchor'])
            if not cursor: history_at=datetime.datetime.now(datetime.timezone.utc)
            at=history_at
            rows=[]
            for i in range((anchor-start).days+1):
                day=start+datetime.timedelta(days=i)
                session=day.weekday()<5 and day.isoformat()!='2026-01-01'
                close_at=datetime.datetime.combine(day,datetime.time(21),datetime.timezone.utc) if session else None
                completed=close_at is not None and close_at<=at
                rows.append(dict(date=day.isoformat(),market_close=close_at.isoformat() if close_at else None,
                    source_close='100' if completed else None,close='100' if completed else None,factor_to_anchor='1',
                    split=None,unsupported_action=None,source_url='https://example.com/history'))
            if mode=='portfolio_dashboard':
                for row in rows:
                    if row['close'] is not None:
                        elapsed=(datetime.date.fromisoformat(row['date'])-datetime.date(2026,1,1)).days
                        value=str(Decimal(100)+Decimal(elapsed)/Decimal(20 if params['instrument']['value']=='^SP500TR' else 10))
                        row['close']=row['source_close']=value
                        if (root/'missing-history').exists() and params['instrument']['value']=='TEST' and row['date']=='2026-05-05': row['close']=row['source_close']=None
            last=next(r['date'] for r in reversed(rows) if r['close'] is not None)
            manifest = dict(instrument=params['instrument'], requested_start=params['start'], requested_end=params['end'],
                coverage_start=start.isoformat(), anchor=params['anchor'], last_completed_session=last,
                currency='USD', exchange_timezone='America/New_York', calendar='NYSE',calendar_version='5.4.0',
                normalization_version=1,source_basis='total_return_index' if params['instrument']['value']=='^SP500TR' else 'yahoo_split_adjusted_close',completeness='unverified',
                retrieved_at=at.isoformat())
            if mode=='history_changed_manifest' and cursor: manifest['retrieved_at']=(at+datetime.timedelta(seconds=1)).isoformat()
            offset=int(params.get('cursor') or '0')
            items=rows[offset:offset+params['page_size']]
            next_offset=offset+len(items)
            result=dict(manifest=manifest,items=items,next_cursor=str(next_offset) if next_offset<len(rows) else None)
        elif method == 'market_data.daily':
            result = {'items': [{'instrument': params['instrument'], 'date': '2024-01-02', 'open': '100', 'high': '102', 'low': '99', 'close': '101', 'volume': 123, 'adjusted_close': '100.5', 'currency': 'USD', 'exchange_timezone': 'America/New_York', 'price_basis': 'source_reported', 'precision': 'decimal_source', 'source_url': 'https://example.test/history', 'retrieved_at': '2026-09-09T00:00:00Z'}], 'next_cursor': None, 'coverage': {'first_date': '2024-01-02', 'last_date': '2024-01-02', 'completeness': 'unverified'}}
            if mode in ('portfolio_current','portfolio_dashboard'):
                day = params['end']
                result['items'][0].update(date=day, retrieved_at=datetime.datetime.now(datetime.timezone.utc).isoformat())
                result['coverage'].update(first_date=day, last_date=day)
                if mode=='portfolio_dashboard':
                    value=str(Decimal(100)+Decimal((datetime.date.fromisoformat(day)-datetime.date(2026,1,1)).days)/10)
                    result['items'][0].update(open=value,high=value,low=value,close=value,adjusted_close=value)
            if mode == 'market_repeated_date' and not cursor:
                result['next_cursor'] = 'page-two'
        elif method == 'filings.document':
            content = b'fixture document'
            media_type = 'text/plain'
            if mode == 'filing_html':
                content, media_type = FILING_HTML, 'text/html; charset=utf-8'
            elif mode == 'filing_html_revised':
                content, media_type = FILING_HTML_REVISED, 'text/html; charset=utf-8'
            result = {'source_url': params['source_url'], 'media_type': media_type, 'content_base64': base64.b64encode(content).decode(), 'retrieved_at': '2026-09-09T00:00:00Z'}
        elif method in ('filings.list', 'fundamentals.facts'):
            common = {'company': params['company'], 'filing_id': 'first' if not cursor else 'second', 'form': '10-K', 'filed': '2024-02-01', 'source_url': 'https://example.test/filing', 'retrieved_at': '2026-09-09T00:00:00Z'}
            if method == 'filings.list':
                result['items'] = [dict(common, report_date='2023-12-31', accepted_at=None, primary_document='report.htm')]
            else:
                result['items'] = [dict(common, namespace='us-gaap', concept='Assets', label=None, value='12345678901234567890.001', unit='USD', period={'kind':'instant','date':'2023-12-31'}, fiscal_year=2023, fiscal_period='FY')]
            if mode in ('second_blocked', 'two_pages') and not cursor:
                result['next_cursor'] = 'page-two'
            if mode == 'repeated_cursor':
                result['next_cursor'] = 'page-two'
            if mode == 'many_items':
                result['items'] *= 3
            if mode == 'large_bytes':
                result['items'][0]['source_url'] += 'x' * 6000
    if method == 'fundamentals.facts' and mode.startswith('apple_comparison'):
        values=[]
        qa_root=pathlib.Path(config['barrier'])
        for year, amount in [(2021,80),(2022,90),(2023,100),(2024,120)]:
            if year == 2024 and (qa_root / 'changed').exists(): amount = 150
            for concept,value in [('RevenueFromContractWithCustomerExcludingAssessedTax',amount),('NetIncomeLoss',10)]:
                values.append(dict(company=params['company'],namespace='us-gaap',concept=concept,label=None,value=str(value),unit='USD',period=dict(kind='duration',start=f'{year}-01-01',end=f'{year}-12-31'),filing_id=f'filing-{year}',form='10-K',filed='2025-02-01',fiscal_year=2024,fiscal_period='FY',source_url='https://fixture.test/comparison',retrieved_at='2026-09-30T00:00:00Z'))
        if (mode=='apple_comparison_partial' or (qa_root / 'partial').exists()) and params['company']['value']=='0000789019': values=[]
        offset=int(params.get('cursor') or '0')
        result={'items':values[offset:offset+2],'next_cursor':str(offset+2) if offset+2<len(values) else None}
    print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': result}), flush=True)
