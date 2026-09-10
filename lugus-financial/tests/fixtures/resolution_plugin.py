import json
import sys
for line in sys.stdin:
    request=json.loads(line)
    if request['method']=='initialize':
        mode=request['params']['config'].get('mode','ok')
        result={'protocol_version':1,'plugin_id':'resolution-fixture','plugin_version':'1','capabilities':{'company_resolution':1} if mode!='unsupported' else {}}
    else:
        candidate={'identifier':{'namespace':'sec:cik','value':'0000051143'},'name':'IBM','aliases':[],'listings':[{'ticker':{'namespace':'sec:ticker','value':'IBM'},'exchange':{'namespace':'sec:exchange','value':'NYSE'}}],'source_url':'https://example.invalid/directory','source_checksum':'a'*64,'retrieved_at':'2026-09-10T00:00:00Z','match_reasons':[]}
        if mode=='wrong_entity': candidate['identifier']['value']='0000000001'
        if request['method']=='company_resolution.search':
            candidate['match_reasons']=['exact_identifier']
            if mode=='wrong_match': candidate['listings'][0]['ticker']['value']='PLTR'
            result={'items':[candidate],'next_cursor':None,'snapshot':'one','coverage':'fixture'}
        else: result=candidate
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}),flush=True)
