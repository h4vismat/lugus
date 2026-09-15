"""Synthetic history capability for lifecycle and persistence tests only."""
import json
import sys
from datetime import date, timedelta

mode = 'ok'
for line in sys.stdin:
    request = json.loads(line)
    q = request.get('params', {})
    result = None
    error = None
    if request['method'] == 'initialize':
        mode = q['config'].get('mode', 'ok')
        result = dict(protocol_version=1, plugin_id='history-fixture', plugin_version='1',
                      capabilities={} if mode == 'unsupported' else {'historical_prices': 1})
    elif mode == 'rate_limited':
        error = dict(code=-32000, message='Rate limited', data={'kind': 'rate_limited'})
    else:
        manifest = dict(instrument=q['instrument'], requested_start=q['start'], requested_end=q['end'],
            coverage_start='2025-12-31', anchor=q['anchor'], last_completed_session='2026-01-05',
            currency='USD', exchange_timezone='America/New_York', calendar='NYSE',calendar_version='5.4.0',
            normalization_version=1,source_basis='yahoo_split_adjusted_close',completeness='unverified',
            retrieved_at='2026-01-05T22:00:00Z')
        rows=[]
        for i in range(6):
            day=date(2025,12,31)+timedelta(days=i)
            session=day.isoformat() in ['2025-12-31','2026-01-02','2026-01-05']
            rows.append(dict(date=day.isoformat(),market_close=day.isoformat()+'T21:00:00Z' if session else None,
                source_close='100' if session else None,close='100' if session else None,factor_to_anchor='1',
                split=None,unsupported_action=None,source_url='https://example.com/history'))
        offset=int(q.get('cursor') or '0')
        items=rows[offset:offset+q['page_size']]
        next_offset=offset+len(items)
        result=dict(manifest=manifest,items=items,next_cursor=str(next_offset) if next_offset<len(rows) else None)
        if mode=='wrong_instrument': manifest['instrument']={'namespace':'yahoo:symbol','value':'OTHER'}
        if mode=='wrong_anchor': manifest['anchor']='2026-01-06'
        if mode=='wrong_order': items.reverse()
        if mode=='invalid_split': items[0]['split']={'numerator':0,'denominator':1}
        if mode=='changed_manifest' and offset: manifest['retrieved_at']='2026-01-05T23:00:00Z'
    reply=dict(jsonrpc='2.0',id=request['id'])
    reply['error' if error else 'result']=error if error else result
    print(json.dumps(reply),flush=True)
