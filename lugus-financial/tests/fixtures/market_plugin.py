import json
import sys

for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'initialize':
        mode = request['params']['config'].get('mode', 'ok')
        result = {'protocol_version': 1, 'plugin_id': 'market-fixture', 'plugin_version': '1', 'capabilities': {} if mode == 'unsupported' else {'market_data': 1}}
    else:
        query = request['params']
        offset = int(query.get('cursor') or 0)
        if mode == 'rate_limited':
            print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'error': {'code': -32000, 'message': 'quota', 'data': {'kind': 'rate_limited'}}}), flush=True)
            continue
        bars = []
        for date in ('2024-01-02', '2024-01-03'):
            bars.append({'instrument': query['instrument'], 'date': date, 'open': '100', 'high': '102', 'low': '99', 'close': '101', 'volume': 123, 'adjusted_close': '100.5', 'currency': 'USD', 'exchange_timezone': 'America/New_York', 'price_basis': 'source_reported', 'precision': 'binary_float_source', 'source_url': 'https://example.com/history', 'retrieved_at': '2026-09-09T00:00:00Z'})
        end = offset + query['page_size']
        result = {'items': bars[offset:end], 'next_cursor': str(end) if end < len(bars) else None, 'coverage': {'first_date': '2024-01-02', 'last_date': '2024-01-03', 'completeness': 'unverified'}}
        if mode == 'wrong_instrument':
            result['items'][0]['instrument'] = {'namespace': 'yahoo:symbol', 'value': 'WRONG'}
        elif mode == 'wrong_ohlc':
            result['items'][0]['high'] = '1'
        elif mode == 'wrong_order':
            result['items'] = list(reversed(result['items']))
    print(json.dumps({'jsonrpc': '2.0', 'id': request['id'], 'result': result}), flush=True)
