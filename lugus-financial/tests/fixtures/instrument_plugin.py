import json
import sys
for line in sys.stdin:
    request = json.loads(line)
    if request['method'] == 'initialize':
        mode = request['params']['config']['mode']
        capabilities = {} if mode == 'unsupported' else {'instrument_lookup': 2 if mode == 'v2' else 1}
        result = dict(protocol_version=1, plugin_id='instrument-fixture', plugin_version='1', capabilities=capabilities)
    else:
        assert mode not in ('unsupported', 'v2'), 'unsupported capability dispatched'
        assert request['method'] == 'instrument_lookup.lookup'
        assert request['params'] == {'instrument': {'namespace': 'yahoo:symbol', 'value': 'AAPL'}}
        result = dict(instrument={'namespace':'yahoo:symbol','value':'AAPL'}, issuer_name='Apple Inc.', ticker='AAPL', exchange={'namespace':'yahoo:exchange','value':'NMS'}, kind='equity', issuer_identifiers=[], source_url='https://finance.yahoo.com/quote/AAPL/', source_checksum='a'*64, retrieved_at='2026-09-10T00:00:00Z')
        if mode == 'unknown': result['untrusted'] = True
        if mode == 'instrument_unknown': result['instrument']['untrusted'] = True
        if mode == 'exchange_unknown': result['exchange']['untrusted'] = True
        if mode == 'issuer_unknown': result['issuer_identifiers'] = [{'namespace':'lei','value':'id','untrusted':True}]
        if mode == 'mismatch': result['instrument']['value'] = 'MSFT'
        if mode == 'bad_name': result['issuer_name'] = 'secret\ncontrol'
    print(json.dumps(dict(jsonrpc='2.0', id=request['id'], result=result)), flush=True)
