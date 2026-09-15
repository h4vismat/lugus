import unittest
from datetime import date
from decimal import Decimal
from history import reverse_split_factor, normalize_history
from common import ProviderError

class HistoricalNormalizationTests(unittest.TestCase):
    def test_later_split_is_reversed_and_same_day_is_excluded(self):
        actions=[(date(2020,8,31),4,1)]
        factor=reverse_split_factor(date(2020,8,28),actions,date(2026,9,15))
        self.assertEqual(factor,Decimal(4))
        self.assertEqual(Decimal('124.8075')*factor,Decimal('499.2300'))
        self.assertEqual(reverse_split_factor(date(2020,8,31),actions,date(2026,9,15)),Decimal(1))
        self.assertEqual(reverse_split_factor(date(2020,8,28),[(date(2020,8,31),1,10)],date(2026,9,15)),Decimal('0.1'))

    def test_normalized_history_preserves_holidays_and_missing_sessions(self):
        query=dict(instrument=dict(namespace='yahoo:symbol',value='TEST'),start='2026-01-02',end='2026-01-02',anchor='2026-01-05')
        calendar=[dict(date=d,market_close=d+'T21:00:00Z' if opened else None) for d,opened in
            [('2025-12-31',True),('2026-01-01',False),('2026-01-02',True),('2026-01-03',False),('2026-01-04',False),('2026-01-05',True)]]
        rows=[dict(date='2025-12-31',close='50',split='0'),dict(date='2026-01-05',close='50',split='2')]
        result=normalize_history(rows,dict(currency='USD',exchangeTimezoneName='America/New_York',calendar='NYSE'),query,'2026-01-05T22:00:00Z',calendar)
        self.assertEqual(result['items'][0]['close'],'100')
        self.assertEqual(result['items'][0]['source_close'],'50')
        self.assertIsNone(result['items'][1]['market_close'])
        self.assertIsNone(result['items'][2]['close'])
        self.assertIsNotNone(result['items'][2]['market_close'])
        self.assertEqual(result['items'][-1]['split'],dict(numerator=2,denominator=1))
        self.assertEqual(result['manifest']['completeness'],'unverified')

    def test_missing_action_column_is_not_assumed_to_be_no_actions(self):
        with self.assertRaises(ProviderError):
            normalize_history([dict(date='2026-01-02',close='50')],{}, {},'',[])

    def test_total_return_index_preserves_source_decimal_representation(self):
        query=dict(instrument=dict(namespace='yahoo:symbol',value='^SP500TR'),start='2026-01-02',end='2026-01-02',anchor='2026-01-02')
        calendar=[dict(date='2025-12-31',market_close='2025-12-31T21:00:00Z'),dict(date='2026-01-01',market_close=None),dict(date='2026-01-02',market_close='2026-01-02T21:00:00Z')]
        rows=[dict(date='2025-12-31',close='100.0',split='0'),dict(date='2026-01-02',close='101.00',split='0')]
        result=normalize_history(rows,dict(currency='USD',exchangeTimezoneName='America/New_York',calendar='NYSE'),query,'2026-01-02T22:00:00Z',calendar)
        for item in result['items']:
            self.assertEqual(item['source_close'],item['close'])
            self.assertEqual(item['factor_to_anchor'],'1')
