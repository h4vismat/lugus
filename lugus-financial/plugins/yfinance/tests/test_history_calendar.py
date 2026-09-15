import unittest
from history_calendar import session_days, calendar_name
from common import ProviderError

class CalendarTests(unittest.TestCase):
    def test_thanksgiving_and_early_close_are_not_guessed_from_prices(self):
        rows=session_days('NYSE','2025-11-27','2025-11-28','2025-11-28T23:00:00Z')
        self.assertEqual(rows[0]['date'],'2025-11-26')
        self.assertIsNone(rows[1]['market_close'])
        self.assertEqual(rows[2]['market_close'],'2025-11-28T18:00:00Z')

    def test_unknown_exchange_is_explicit(self):
        with self.assertRaises(ProviderError):
            calendar_name(dict(exchangeName='LSE',exchangeTimezoneName='Europe/London'),'TEST')

    def test_dst_changes_utc_session_close(self):
        rows=session_days('NASDAQ','2026-03-06','2026-03-09','2026-03-09T23:00:00Z')
        by_date={r['date']:r['market_close'] for r in rows}
        self.assertEqual(by_date['2026-03-06'],'2026-03-06T21:00:00Z')
        self.assertEqual(by_date['2026-03-09'],'2026-03-09T20:00:00Z')
