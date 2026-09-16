#!/usr/bin/env python3
"""Bounded line-framed JSON-RPC; stdout is exclusively protocol output."""
import json
import sys
from provider import MAX_LINE, Provider, ProviderError, error_response, handle


def reject_constant(value):
    raise ValueError('Nonfinite JSON value')


def main():
    provider = Provider()
    while True:
        line = sys.stdin.buffer.readline(-1 if provider.unlimited_research else MAX_LINE + 1)
        if not line:
            return
        if not provider.unlimited_research and len(line) > MAX_LINE:
            print(json.dumps(error_response(None,ProviderError('invalid_request','Request exceeds byte limit',-32600))),flush=True)
            return
        try:
            request = json.loads(line,parse_constant=reject_constant)
            response = handle(provider,request)
        except (ValueError,UnicodeError,RecursionError):
            response = error_response(None,ProviderError('invalid_request','Invalid JSON',-32700))
        output = json.dumps(response,separators=(',',':'),allow_nan=False)
        if not provider.unlimited_research and len(output.encode('utf-8')) > MAX_LINE:
            output = json.dumps(error_response(response['id'],ProviderError('malformed_data','Response exceeds byte limit')))
        print(output,flush=True)


if __name__ == '__main__':
    main()
