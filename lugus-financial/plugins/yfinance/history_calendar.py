"""Explicit, version-pinned US equity session calendars."""
from datetime import date, timedelta
from common import ProviderError

CALENDAR_VERSION = '5.4.0'

def calendar_name(metadata, symbol):
    if metadata.get('exchangeTimezoneName') != 'America/New_York':
        raise ProviderError('unsupported', 'Historical timezone is not supported')
    if symbol == '^SP500TR':
        return 'NYSE'
    code = metadata.get('exchangeName')
    if code in ('NMS', 'NGM', 'NCM'):
        return 'NASDAQ'
    if code in ('NYQ', 'PCX', 'ASE'):
        return 'NYSE'
    raise ProviderError('unsupported', 'Historical exchange calendar is not supported')

def get_calendar(name):
    try:
        import pandas_market_calendars as mcal
        from importlib.metadata import version
        if version('pandas_market_calendars') != CALENDAR_VERSION:
            raise ProviderError('configuration', 'Expected pandas_market_calendars==' + CALENDAR_VERSION)
        if name not in ('NYSE', 'NASDAQ'):
            raise ProviderError('unsupported', 'Unknown historical calendar')
        return mcal.get_calendar(name)
    except ImportError as exc:
        raise ProviderError('configuration', 'Install historical calendar requirements') from exc

def previous_session(name, start):
    start = date.fromisoformat(start)
    if start <= date(1900, 1, 1):
        raise ProviderError('unsupported', 'Historical calendars begin after 1900-01-01')
    # The calendar, not missing prices or a weekday guess, determines the baseline.
    days = get_calendar(name).valid_days(start_date=date(1900, 1, 1), end_date=start-timedelta(days=1))
    if not len(days):
        raise ProviderError('not_found', 'No preceding calendar session')
    return days[-1].date().isoformat()

def session_days(name, start, anchor, retrieved_at):
    first = date.fromisoformat(previous_session(name, start))
    end = date.fromisoformat(anchor)
    if (end-first).days >= 100_000:
        raise ProviderError('invalid_request', 'Historical calendar exceeds row limit')
    schedule = get_calendar(name).schedule(start_date=first, end_date=end)
    sessions = {stamp.date().isoformat(): row['market_close'].isoformat().replace('+00:00','Z')
                for stamp, row in schedule.iterrows()}
    return [dict(date=(first+timedelta(days=i)).isoformat(), market_close=sessions.get((first+timedelta(days=i)).isoformat()))
            for i in range((end-first).days+1)]
