"""Recovery exercises the provider boundary without live Yahoo traffic or real sleeps."""
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
from common import ProviderError
from provider import Provider, error_response
from test_provider import Q, META, row


class RecoveryTests(unittest.TestCase):
    def setUp(self):
        self.now = 0
        self.sleeps = []

    def sleep(self, delay):
        self.sleeps.append(delay)
        self.now += delay

    def recovery(self):
        from recovery import Recovery
        return Recovery(sleep=self.sleep, monotonic=lambda: self.now, jitter=lambda: 0)

    def test_history_recovers_then_paginates_without_refetch(self):
        calls = []
        def fetch(*args):
            calls.append(args)
            if len(calls) == 1:
                raise ProviderError('timeout', 'slow upstream')
            return [row(2), row(3)], META
        plugin = Provider(fetch=fetch, recovery=self.recovery())
        plugin.initialize({'protocol_version': 1, 'config': {}})
        first = plugin.daily(Q)
        second = plugin.daily(dict(Q, cursor=first['next_cursor']))
        self.assertEqual(first['items'][0]['date'], '2024-01-02')
        self.assertEqual(second['items'][0]['date'], '2024-01-03')
        self.assertEqual(len(calls), 2)
        self.assertEqual(self.sleeps, [1])

    def test_rate_limit_cooldown_shared_between_history_and_identity(self):
        calls = []
        def limited(*args):
            calls.append('history')
            raise ProviderError('rate_limited', 'busy')
        def metadata(*args):
            calls.append('identity')
            return {'symbol': 'AAPL'}
        plugin = Provider(fetch=limited, fetch_metadata=metadata, recovery=self.recovery())
        plugin.initialize({'protocol_version': 1, 'config': {}})
        with self.assertRaises(ProviderError) as caught:
            plugin.daily(Q)
        self.assertEqual(error_response(1, caught.exception)['error']['data']['retry_after_seconds'], 60)
        self.now = 20
        with self.assertRaises(ProviderError) as caught:
            plugin.lookup_instrument({'instrument': Q['instrument']})
        self.assertEqual(caught.exception.retry_after_seconds, 40)
        self.assertEqual(calls, ['history'])
        self.now = 60
        self.assertEqual(plugin.lookup_instrument({'instrument': Q['instrument']})['ticker'], 'AAPL')
        self.assertEqual(calls, ['history', 'identity'])
        self.assertEqual(self.sleeps, [])

    def test_permanent_failures_are_not_retried(self):
        for kind in ('configuration', 'invalid_request', 'not_found', 'malformed_data', 'unsupported'):
            calls = []
            def fail():
                calls.append(1)
                raise ProviderError(kind, 'permanent')
            with self.subTest(kind=kind), self.assertRaises(ProviderError) as caught:
                self.recovery().run(fail)
            self.assertEqual(caught.exception.kind, kind)
            self.assertEqual(len(calls), 1)
        self.assertEqual(self.sleeps, [])

    def test_repeated_outage_stops_and_next_call_observes_cooldown(self):
        recovery = self.recovery()
        calls = []
        def fail():
            calls.append(1)
            raise ProviderError('unavailable', 'offline')
        with self.assertRaises(ProviderError) as caught:
            recovery.run(fail)
        self.assertEqual(caught.exception.kind, 'unavailable')
        self.assertEqual(len(calls), 3)
        self.assertEqual(self.sleeps, [1, 2])
        with self.assertRaises(ProviderError):
            recovery.run(fail)
        self.assertEqual(len(calls), 3)

    def test_retry_after_is_never_shortened(self):
        recovery = self.recovery()
        def fail():
            raise ProviderError('unavailable', 'busy', retry_after_seconds=120)
        with self.assertRaises(ProviderError) as caught:
            recovery.run(fail)
        self.assertEqual(caught.exception.retry_after_seconds, 120)
        self.assertEqual(self.sleeps, [])

    def test_slow_failure_does_not_start_another_attempt_after_budget(self):
        calls = []
        def fail():
            calls.append(1)
            self.now += 40
            raise ProviderError('timeout', 'slow')
        with self.assertRaises(ProviderError):
            self.recovery().run(fail)
        self.assertEqual(len(calls), 1)
        self.assertEqual(self.sleeps, [])
