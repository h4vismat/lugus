"""Exercise the production EdgarTools boundary with offline HTTP responses."""
import base64
from contextlib import contextmanager, redirect_stdout
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import provider as p
from test_provider import FACTS, QUERY, STAMP

HAS_EDGAR = importlib.util.find_spec('edgar') is not None
URL = 'https://data.sec.gov/api/xbrl/companyfacts/CIK0000320193.json'
DOCUMENT = 'https://www.sec.gov/Archives/edgar/data/320193/report.htm'


@unittest.skipUnless(HAS_EDGAR, 'Install the SEC plugin requirements for adapter tests')
class EdgarToolsTests(unittest.TestCase):
    def setUp(self):
        self.assertIsNotNone(importlib.util.find_spec('edgar_transport'),
                             'EdgarTools retrieval adapter must be implemented')
        import httpx
        self.httpx = httpx
        self.now = 0
        self.requests = []

    def sleep(self, seconds):
        self.now += seconds

    def transport(self, handler):
        from edgar_transport import EdgarOpener

        def record(request):
            self.requests.append(request)
            return handler(request)

        @contextmanager
        def client_factory(identity):
            with self.httpx.Client(transport=self.httpx.MockTransport(record)) as client:
                yield client

        transport = p.HttpTransport('Lugus test@example.com', sleep=self.sleep,
                                    monotonic=lambda: self.now)
        transport.opener_factory = lambda archive: EdgarOpener(
            archive, transport.wait_turn, lambda: self.now, client_factory)
        return transport

    def response(self, status=200, body=b'{}', headers=None):
        return self.httpx.Response(status, headers=headers,
                                   stream=self.httpx.ByteStream(body))

    def test_default_transport_uses_edgartools_and_keeps_exact_facts(self):
        # Patch only the socket boundary: EdgarTools creates the real HTTP client.
        def respond(inner, request):
            self.requests.append(request)
            return self.response(body=FACTS)

        plugin = p.Provider(clock=lambda: STAMP)
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus test@example.com'}})
        with patch.object(self.httpx.HTTPTransport, 'handle_request', respond):
            output = io.StringIO()
            with redirect_stdout(output):
                result = plugin.call('fundamentals.facts', QUERY)
                second = plugin.call('fundamentals.facts', dict(QUERY, cursor=result['next_cursor']))
            self.assertEqual(len(self.requests), 1)
            plugin.call('initialize', {'protocol_version': 1, 'config': {
                'user_agent': 'Lugus changed@example.com'}})
            plugin.call('fundamentals.facts', QUERY)
        self.assertEqual(result['items'][0]['value'], '12345678901234567890.123456789')
        self.assertEqual(second['items'][0]['value'], '0.0000001')
        self.assertEqual(output.getvalue(), '')
        self.assertEqual(len(self.requests), 2)
        self.assertEqual(self.requests[0].headers['user-agent'], 'Lugus test@example.com')
        self.assertEqual(self.requests[1].headers['user-agent'], 'Lugus changed@example.com')

    def test_timeout_retries_then_same_provider_recovers(self):
        def respond(request):
            if len(self.requests) <= 3:
                raise self.httpx.ReadTimeout('private detail', request=request)
            return self.response(body=FACTS)
        plugin = p.Provider(transport=self.transport(respond), clock=lambda: STAMP)
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus test@example.com'}})
        failed = p.handle(plugin, {'jsonrpc': '2.0', 'id': 1,
                                  'method': 'fundamentals.facts', 'params': QUERY})
        self.assertEqual(failed['error']['data']['kind'], 'timeout')
        self.assertNotIn('private detail', json.dumps(failed))
        self.assertEqual(len(self.requests), 3)
        self.assertEqual(plugin.call('fundamentals.facts', QUERY)['items'][0]['value'],
                         '12345678901234567890.123456789')

    def test_rate_limit_cooldown_applies_across_operations_and_reinitialize(self):
        transport = self.transport(lambda _: self.response(429, headers={'Retry-After': '120'}))
        plugin = p.Provider(transport=transport)
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus test@example.com'}})
        with self.assertRaises(p.ProviderError) as caught:
            transport.get(URL, 100)
        self.assertEqual(caught.exception.retry_after_seconds, 120)
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus changed@example.com'}})
        with self.assertRaises(p.ProviderError) as caught:
            transport.get(DOCUMENT, 100, archive=True)
        self.assertEqual(caught.exception.kind, 'rate_limited')
        self.assertEqual(len(self.requests), 1)

    def test_stream_timeout_discards_partial_bytes_before_retry(self):
        httpx = self.httpx

        class BrokenStream(httpx.SyncByteStream):
            def __iter__(self):
                yield b'x' * 65536
                raise httpx.ReadTimeout('private stream error')

        def respond(request):
            if len(self.requests) == 1:
                return httpx.Response(200, stream=BrokenStream())
            return self.response(body=b'complete')

        transport = self.transport(respond)
        self.assertEqual(transport.get(DOCUMENT, 100000, archive=True)[0], b'complete')
        self.assertEqual(len(self.requests), 2)

    def test_slow_trickle_cannot_hide_the_overall_read_deadline(self):
        httpx = self.httpx
        test = self

        class SlowStream(httpx.SyncByteStream):
            def __iter__(self):
                for _ in range(100):
                    test.now += 5
                    yield b'x'

        transport = self.transport(lambda _: httpx.Response(200, stream=SlowStream()))
        with self.assertRaises(p.ProviderError) as caught:
            transport.get(DOCUMENT, 100000, archive=True)
        self.assertEqual(caught.exception.kind, 'timeout')
        self.assertLessEqual(self.now, 45)

    def test_redirect_delay_cannot_start_request_after_deadline(self):
        transport = self.transport(lambda _: self.response(302, headers={'Location': 'next.htm'}))
        def expired():
            self.now += 50
        from edgar_transport import EdgarOpener
        original = transport.opener_factory
        def opener(archive):
            result = original(archive)
            result.before_request = expired
            return result
        transport.opener_factory = opener
        with self.assertRaises(p.ProviderError) as caught:
            transport.get(DOCUMENT, 100, archive=True)
        self.assertEqual(caught.exception.kind, 'timeout')
        self.assertEqual(len(self.requests), 1)

    def test_certificate_failure_requires_configuration_correction(self):
        import ssl
        def respond(request):
            try:
                raise ssl.SSLCertVerificationError('private certificate detail')
            except ssl.SSLCertVerificationError as exc:
                raise self.httpx.ConnectError('private TLS error', request=request) from exc
        transport = self.transport(respond)
        with self.assertRaises(p.ProviderError) as caught:
            transport.get(URL, 100)
        self.assertEqual(caught.exception.kind, 'configuration')
        self.assertEqual(len(self.requests), 1)
        self.assertNotIn('private', str(caught.exception))

    def test_incompatible_http_client_is_configuration_error_before_io(self):
        from edgar_transport import EdgarOpener
        from urllib.request import Request

        @contextmanager
        def incompatible(identity):
            yield object()

        opener = EdgarOpener(False, lambda: None, client_factory=incompatible)
        with self.assertRaises(p.ProviderError) as caught:
            with opener.open(Request(URL), timeout=10):
                self.fail('An incompatible client cannot fetch data')
        self.assertEqual(caught.exception.kind, 'configuration')

    def test_missing_or_unsupported_dependency_is_actionable(self):
        from importlib.metadata import PackageNotFoundError
        from edgar_transport import client_manager
        for options in ({'return_value': '0.0.0'},
                        {'side_effect': PackageNotFoundError('edgartools')}):
            with self.subTest(options=options), patch('edgar_transport.version', **options):
                with self.assertRaises(p.ProviderError) as caught:
                    client_manager.__wrapped__()
                self.assertEqual(caught.exception.kind, 'configuration')
                self.assertIn('requirements', str(caught.exception))

    def test_jsonrpc_process_recovers_with_real_edgartools_client(self):
        import subprocess
        import textwrap
        directory = Path(__file__).resolve().parents[1]
        script = textwrap.dedent('''
            import sys
            sys.path.insert(0, 'tests')
            import httpx
            from unittest.mock import patch
            from test_provider import FACTS
            from main import main
            count = 0
            def respond(self, request):
                global count
                count += 1
                if count <= 3:
                    raise httpx.ReadTimeout('private detail', request=request)
                return httpx.Response(200, stream=httpx.ByteStream(FACTS))
            with patch.object(httpx.HTTPTransport, 'handle_request', respond):
                main()
        ''')
        messages = [
            {'jsonrpc': '2.0', 'id': 1, 'method': 'initialize', 'params': {
                'protocol_version': 1, 'config': {'user_agent': 'Lugus test@example.com'}}},
            *[{'jsonrpc': '2.0', 'id': i, 'method': 'fundamentals.facts', 'params': QUERY}
              for i in (2, 3)],
        ]
        result = subprocess.run([sys.executable, '-c', script], cwd=directory,
                                input=''.join(json.dumps(m) + '\n' for m in messages),
                                text=True, capture_output=True, timeout=30, check=True)
        responses = [json.loads(line) for line in result.stdout.splitlines()]
        self.assertEqual([r['id'] for r in responses], [1, 2, 3])
        self.assertEqual(responses[1]['error']['data']['kind'], 'timeout')
        self.assertEqual(responses[2]['result']['items'][0]['value'],
                         '12345678901234567890.123456789')
        self.assertNotIn('private detail', result.stdout + result.stderr)
        self.assertEqual(result.stderr, '')

    def test_permanent_errors_do_not_retry_or_become_empty(self):
        for status, kind in ((403, 'configuration'), (404, 'not_found')):
            with self.subTest(status=status):
                self.requests.clear()
                transport = self.transport(lambda _: self.response(status))
                with self.assertRaises(p.ProviderError) as caught:
                    transport.get(URL, 100)
                self.assertEqual(caught.exception.kind, kind)
                self.assertEqual(len(self.requests), 1)

    def test_redirect_is_validated_before_contact_and_is_bounded(self):
        for location in ('https://evil.example/file', 'http://www.sec.gov/file', DOCUMENT):
            with self.subTest(location=location):
                self.requests.clear()
                transport = self.transport(lambda _: self.response(302, headers={'Location': location}))
                with self.assertRaises(p.ProviderError):
                    transport.get(DOCUMENT, 100, archive=True)
                self.assertLessEqual(len(self.requests), 6)
                self.assertTrue(all(str(r.url) == DOCUMENT for r in self.requests))

    def test_relative_redirect_is_limited_and_preserves_identity(self):
        def respond(request):
            if len(self.requests) == 1:
                return self.response(302, headers={'Location': 'next.htm'})
            return self.response(body=b'<p>exact</p>', headers={'Content-Type': 'text/html'})
        transport = self.transport(respond)
        self.assertEqual(transport.get(DOCUMENT, 100, archive=True), (b'<p>exact</p>', 'text/html'))
        self.assertEqual(self.now, 0.5)
        self.assertEqual(str(self.requests[1].url), DOCUMENT.replace('report', 'next'))
        self.assertEqual(self.requests[1].headers['user-agent'], 'Lugus test@example.com')

    def test_oversized_truncated_and_encoded_sources_are_rejected(self):
        for body, headers in ((b'abcdef', {}), (b'abc', {'Content-Length': '4'}),
                              (b'abc', {'Content-Encoding': 'gzip'})):
            with self.subTest(headers=headers):
                transport = self.transport(lambda _: self.response(body=body, headers=headers))
                with self.assertRaises(p.ProviderError) as caught:
                    transport.get(DOCUMENT, 5, archive=True)
                self.assertEqual(caught.exception.kind, 'malformed_data')

    def test_documents_and_resolution_keep_source_bytes(self):
        body = b'\xff\x00\r\n'
        plugin = p.Provider(transport=self.transport(lambda _: self.response(body=body)),
                            clock=lambda: STAMP)
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus test@example.com'}})
        document = plugin.call('filings.document', {'source_url': DOCUMENT})
        self.assertEqual(base64.b64decode(document['content_base64']), body)
        directory = b'{"fields":["cik","name","ticker","exchange"],"data":[[320193,"Apple Inc.","AAPL","Nasdaq"]]}'
        plugin.transport = self.transport(lambda _: self.response(body=directory))
        plugin.call('initialize', {'protocol_version': 1, 'config': {
            'user_agent': 'Lugus test@example.com'}})
        result = plugin.call('company_resolution.search', {'query': {'kind': 'name', 'text': 'Apple'}})
        self.assertEqual(result['items'][0]['source_checksum'], hashlib.sha256(directory).hexdigest())


if __name__ == '__main__':
    unittest.main()
