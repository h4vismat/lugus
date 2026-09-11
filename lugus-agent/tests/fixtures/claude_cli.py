#!/usr/bin/env python3
"""Deterministic Claude print protocol peer, no credentials or model calls."""
import json, os, sys, time, urllib.request, urllib.error
from pathlib import Path

if '--version' in sys.argv:
    print('2.1.268 (Claude Code)')
    sys.exit(0)

def option(name):
    return sys.argv[sys.argv.index(name) + 1]

def emit(value):
    print(json.dumps(value), flush=True)

assert '--print' in sys.argv and '--verbose' in sys.argv
assert '--no-session-persistence' in sys.argv
assert '--include-partial-messages' in sys.argv
assert '--restricted' in sys.argv and '--strict-mcp-config' in sys.argv
assert '--disable-slash-commands' in sys.argv
assert option('--permission-mode') == 'dontAsk'
assert option('--permission-prompts') == 'none'
assert option('--tools') in ('', 'WebSearch,WebFetch')
assert '--bare' not in sys.argv
assert json.loads(option('--settings'))['disableAllHooks'] is True
assert option('--output-format') == 'stream-json'
Path('pid').write_text(str(os.getpid()))
config = json.loads(option('--mcp-config'))['mcpServers']['lugus']
Path('endpoint').write_text(config['url'])
Path('mcp_authorization.tmp').write_text(config['headers']['Authorization'])
Path('mcp_authorization.tmp').replace('mcp_authorization')
body = json.loads(sys.stdin.read())
assert body['context'] == 'fixture context'
scenario = body['prompt']

def rpc(method, params=None, headers=None, ident=1, raw=None):
    data = raw if raw is not None else json.dumps({'jsonrpc': '2.0', 'id': ident, 'method': method, 'params': params or {}}).encode()
    hdrs = {'Content-Type': 'application/json', 'Accept': 'application/json, text/event-stream', **config['headers']}
    if headers: hdrs.update(headers)
    req = urllib.request.Request(config['url'], data=data, headers=hdrs)
    try:
        with urllib.request.urlopen(req, timeout=5) as response:
            return response.status, json.loads(response.read())
    except urllib.error.HTTPError as error:
        return error.code, None

if scenario == 'progress':
    emit({'type': 'tool_progress', 'tool_use_id': 'native-id', 'tool_name': 'mcp__lugus__lookup', 'elapsed_time_seconds': 1})
    emit({'type': 'tool_use_summary', 'summary': 'Tool completed', 'preceding_tool_use_ids': ['native-id']})
elif scenario == 'malformed':
    print('not json', flush=True)
    time.sleep(30)
elif scenario == 'oversized_frame':
    print('x' * (8 * 1024 * 1024 + 1), flush=True)
    time.sleep(30)
elif scenario == 'auth':
    emit({'type': 'assistant', 'error': 'authentication_failed', 'message': {'content': []}})
    emit({'type': 'result', 'subtype': 'error_during_execution', 'is_error': True, 'errors': ['secret credential do not propagate']})
    sys.exit(1)
elif scenario == 'hang':
    time.sleep(30)
elif scenario == 'eof':
    sys.exit(0)
elif scenario.startswith('tool') or scenario == 'security':
    status, response = rpc('initialize', {'protocolVersion': '2025-11-25', 'capabilities': {}, 'clientInfo': {'name': 'fixture', 'version': '1'}})
    assert status == 200 and response['result']['protocolVersion'] == '2025-11-25'
    if scenario == 'security':
        assert rpc('tools/list', headers={'Authorization': 'Bearer wrong'})[0] == 401
        assert rpc('tools/list', headers={'Origin': 'http://evil.example'})[0] == 403
        assert rpc('tools/list', raw=b'x' * (1024 * 1024 + 1))[0] == 413
        status, response = rpc('tools/call', {'name': 'unregistered', 'arguments': {}})
        assert status == 200 and 'error' in response
    else:
        status, response = rpc('tools/list')
        assert [x['name'] for x in response['result']['tools']] == ['lookup']
        count = 2 if scenario == 'tool_limit' else 1
        for index in range(count):
            status, response = rpc('tools/call', {'name': 'lookup', 'arguments': {'query': 'synthetic'}}, ident=index + 2)
            assert status == 200 and response['id'] == index + 2
            result = response['result']
            if scenario == 'tool_zero' or (scenario == 'tool_limit' and index == 1):
                assert result['isError']
            elif scenario == 'tool_large':
                assert result['isError'] and len(result['content'][0]['text']) <= 100
            else:
                assert not result['isError'] and result['content'][0]['text'] == 'fixture value'

emit({'type': 'stream_event', 'event': {'type': 'content_block_delta', 'delta': {'type': 'text_delta', 'text': 'Hello '}}})
emit({'type': 'stream_event', 'event': {'type': 'content_block_delta', 'delta': {'type': 'text_delta', 'text': 'world'}}})
emit({'type': 'assistant', 'message': {'content': [{'type': 'text', 'text': 'Hello world'}]}})
emit({'type': 'result', 'subtype': 'success', 'is_error': False, 'result': 'Hello world', 'usage': {'input_tokens': 7, 'output_tokens': 3}})
