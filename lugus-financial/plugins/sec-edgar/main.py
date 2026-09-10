#!/usr/bin/env python3
"""Line-framed JSON-RPC entry point; stdout is exclusively protocol output."""
import json
import sys
from provider import MAX_LINE, Provider, ProviderError, handle, parse_json

def main():
    provider=Provider()
    while True:
        line=sys.stdin.buffer.readline(MAX_LINE+1)
        if not line: return
        if len(line)>MAX_LINE:
            response={'jsonrpc':'2.0','id':None,'error':{'code':-32600,'message':'Request exceeds byte limit','data':{'kind':'invalid_request'}}}
            print(json.dumps(response),flush=True)
            return
        try:
            request=parse_json(line)
            response=handle(provider,request)
        except ProviderError:
            response={'jsonrpc':'2.0','id':None,'error':{'code':-32700,'message':'Invalid JSON','data':{'kind':'invalid_request'}}}
        except Exception:
            response={'jsonrpc':'2.0','id':request.get('id') if isinstance(request,dict) else None,'error':{'code':-32000,'message':'Unexpected provider failure','data':{'kind':'unavailable'}}}
        output=json.dumps(response,separators=(',',':'),allow_nan=False)
        if len(output.encode('utf-8'))>MAX_LINE:
            output=json.dumps({'jsonrpc':'2.0','id':response['id'],'error':{'code':-32000,'message':'Response exceeds byte limit','data':{'kind':'malformed_data'}}})
        print(output,flush=True)

if __name__=='__main__': main()
