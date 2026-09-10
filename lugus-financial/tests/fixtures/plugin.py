import json
import base64
import sys
import time

for line in sys.stdin:
    req = json.loads(line)
    method = req['method']
    if method == 'initialize':
        mode = req['params']['config'].get('mode', 'ok')
        result = {'protocol_version': 1, 'plugin_id': 'fixture', 'plugin_version': '1', 'capabilities': {'filings': 1, 'fundamentals': 1}}
        if mode == 'wrong_version':
            result['protocol_version'] = 2
        if mode == 'unsupported':
            result['capabilities'] = {'filings': 1}
    else:
        if mode == 'timeout':
            time.sleep(10)
        if mode == 'malformed':
            print('not json', flush=True)
            continue
        if mode == 'oversized':
            print('x' * 4096, flush=True)
            continue
        if mode == 'stderr':
            sys.stderr.write('x' * 100000)
            sys.stderr.flush()
        if mode == 'error':
            print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'error': {'code': -32000, 'message': 'slow down', 'data': {'kind': 'rate_limited', 'retry_after_seconds': 3}}}), flush=True)
            continue
        result = {'items': [], 'next_cursor': None}
        if mode == 'populated':
            if method == 'filings.document':
                result = {'source_url': req['params']['source_url'], 'media_type': 'text/html', 'content_base64': base64.b64encode(b'<html>filing</html>').decode(), 'retrieved_at': '2026-09-09T00:00:00Z'}
            else:
                company = req['params']['company']
                common = {'company': company, 'filing_id': '0000320193-24-000001', 'form': '10-K', 'filed': '2024-02-01', 'source_url': 'https://example.com/filing', 'retrieved_at': '2026-09-09T00:00:00Z'}
                if method == 'filings.list':
                    result['items'] = [dict(common, report_date='2023-12-31', accepted_at=None, primary_document='report.htm')]
                elif not req['params'].get('cursor'):
                    result['items'] = [dict(common, namespace='us-gaap', concept='Assets', label=None, value='12345678901234567890.001', unit='USD', period={'kind':'instant','date':'2023-12-31'}, fiscal_year=2023, fiscal_period='FY')]
                    result['next_cursor'] = 'second-page'
                else:
                    result['items'] = [dict(common, namespace='custom', concept='Unknown', label=None, value='1', unit='shares', period={'kind':'instant','date':'2023-12-31'}, fiscal_year=None, fiscal_period=None)]
    print(json.dumps({'jsonrpc': '2.0', 'id': req['id'], 'result': result}), flush=True)
