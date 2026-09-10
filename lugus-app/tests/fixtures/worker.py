"""Real protocol peer. Files signal response boundaries, release files unblock them."""
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
                  'plugin_id': 'worker-fixture', 'plugin_version': config.get('version', '1'),
                  'capabilities': {'filings': 1, 'fundamentals': 1, 'company_resolution': 1, 'market_data': 1, 'instrument_lookup': 2 if mode == 'instrument_unsupported' else 1}}
    else:
        cursor = params.get('cursor')
        (root / ('second' if cursor else 'first')).touch()
        if method == 'market_data.daily': (root / 'prices-started').touch()
        if (mode == 'apple_price_blocked' and method == 'market_data.daily') or mode == 'blocked' or (mode == 'search_blocked' and method == 'company_resolution.search') or (mode in ('second_blocked', 'repeated_cursor', 'market_repeated_date') and cursor):
            while not (root / 'release').exists():
                # A killed CLI cannot run normal child cleanup. During a blocked
                # response, EOF on the request pipe means its owner is gone.
                ready, _, _ = select.select([sys.stdin], [], [], 0.005)
                if ready and not os.read(sys.stdin.fileno(), 1):
                    raise SystemExit(0)
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
                if params['query']['kind'] == 'identifier':
                    identifier = params['query']['identifier']
                    if identifier['value'].upper() in ('AAPL', '0000320193'):
                        result['items'][0]['match_reasons'] = ['exact_identifier']
                    else:
                        result['items'] = []
        elif method == 'company_resolution.lookup':
            result = {'identifier': params['identifier'], 'name': 'Fixture', 'aliases': [], 'listings': [], 'source_url': 'https://example.test/company', 'source_checksum': 'a' * 64, 'retrieved_at': '2026-09-09T00:00:00Z', 'match_reasons': []}
        elif method == 'instrument_lookup.lookup':
            result = {'instrument': {'namespace':'yahoo:symbol','value':'AAPL'},'issuer_name':'Apple Inc.','ticker':'AAPL','exchange':{'namespace':'yahoo:exchange','value':'NMS'},'kind':'equity','issuer_identifiers':[],'source_url':'https://example.test/instrument','source_checksum':'b'*64,'retrieved_at':'2026-09-09T00:00:00Z'}
            if mode == 'apple_wrong_issuer': result['issuer_name'] = 'Another Issuer Inc.'
            if mode == 'apple_wrong_exchange': result['exchange']['value'] = 'NYQ'
            if mode == 'apple_missing': result['issuer_name'] = None
        elif method == 'market_data.daily':
            result = {'items': [{'instrument': params['instrument'], 'date': '2024-01-02', 'open': '100', 'high': '102', 'low': '99', 'close': '101', 'volume': 123, 'adjusted_close': '100.5', 'currency': 'USD', 'exchange_timezone': 'America/New_York', 'price_basis': 'source_reported', 'precision': 'decimal_source', 'source_url': 'https://example.test/history', 'retrieved_at': '2026-09-09T00:00:00Z'}], 'next_cursor': None, 'coverage': {'first_date': '2024-01-02', 'last_date': '2024-01-02', 'completeness': 'unverified'}}
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
    print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': result}), flush=True)
