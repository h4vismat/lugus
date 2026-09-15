"""Historical share-basis normalization; source errors never become synthetic prices."""
from datetime import date, datetime, timedelta
from decimal import Decimal, InvalidOperation, localcontext, ROUND_HALF_EVEN
from fractions import Fraction
from contextlib import redirect_stdout
from urllib.parse import quote
from zoneinfo import ZoneInfo
import json
import re
import secrets
import sys
from common import ProviderError
from history_calendar import calendar_name, previous_session, session_days, CALENDAR_VERSION

MAX_ROWS = 100_000
MAX_BYTES = 24*1024*1024

def number(value):
    if isinstance(value, bool):
        raise ValueError('Boolean price or split')
    result = Decimal(str(value))
    if not result.is_finite() or result < 0 or result.adjusted() > 19:
        raise ValueError('Invalid or unsupported historical number')
    return result

def exact_text(value):
    with localcontext() as ctx:
        ctx.prec=80
        value=value.quantize(Decimal('0.000000000000000001'),rounding=ROUND_HALF_EVEN)
    if abs(value) >= Decimal('100000000000000000000'):
        raise ValueError('Historical number exceeds supported range')
    text=format(value,'f').rstrip('0').rstrip('.')
    return text if text and text!='-0' else '0'

def reverse_split_factor(day, splits, anchor):
    ratio=Fraction(1)
    for effective,numerator,denominator in splits:
        if numerator<=0 or denominator<=0:
            raise ValueError('Invalid split ratio')
        if day < effective <= anchor:
            ratio*=Fraction(numerator,denominator)
    with localcontext() as ctx:
        ctx.prec=80
        return Decimal(ratio.numerator)/Decimal(ratio.denominator)

def normalize_history(rows, metadata, query, retrieved_at, calendar_rows):
    try:
        if metadata['currency']!='USD' or metadata['exchangeTimezoneName']!='America/New_York':
            raise ProviderError('unsupported','Historical prices require supported USD exchange evidence')
        anchor=date.fromisoformat(query['anchor'])
        now=datetime.fromisoformat(retrieved_at)
        if not now.tzinfo or not calendar_rows or len(calendar_rows)>MAX_ROWS or len(rows)>MAX_ROWS:
            raise ValueError('Invalid calendar or source snapshot')
        by_date={}; splits=[]
        for row in rows:
            day=date.fromisoformat(row['date'])
            if day.isoformat() in by_date or day.isoformat()<calendar_rows[0]['date'] or day>anchor:
                raise ValueError('Duplicate or out-of-range historical row')
            ratio=number(row['split'])  # A missing action column is not a no-action claim.
            split=None
            if ratio:
                fraction=Fraction(ratio)
                if max(fraction.numerator,fraction.denominator)>2**64-1:
                    raise ValueError('Split ratio exceeds supported range')
                split=dict(numerator=fraction.numerator,denominator=fraction.denominator)
                splits.append((day,fraction.numerator,fraction.denominator))
            close=None if row['close'] is None else number(row['close'])
            by_date[day.isoformat()]=dict(close=close,split=split,unsupported_action=row.get('unsupported_action'))
        benchmark=query['instrument']['value']=='^SP500TR'
        if benchmark and splits:
            raise ValueError('Total-return index cannot have split actions')
        items=[]; completed=[]; previous=None
        url='https://finance.yahoo.com/quote/'+quote(query['instrument']['value'],safe='')+'/history/'
        for cal in calendar_rows:
            day=date.fromisoformat(cal['date'])
            if previous is not None and day!=previous+timedelta(days=1):
                raise ValueError('Nonconsecutive calendar rows')
            previous=day
            source=by_date.get(cal['date'],{})
            close_at=datetime.fromisoformat(cal['market_close']) if cal['market_close'] else None
            if close_at and close_at<=now: completed.append(cal['date'])
            if close_at is None and (source.get('close') is not None or source.get('split')):
                raise ValueError('Source price/action on a scheduled closure')
            raw=source.get('close') if close_at and close_at<=now else None
            factor=Decimal(1) if benchmark else reverse_split_factor(day,splits,anchor)
            with localcontext() as ctx:
                ctx.prec=80
                normalized=None if raw is None else (format(raw,'f') if benchmark else exact_text(raw*factor))
            items.append(dict(date=cal['date'],market_close=cal['market_close'],
                source_close=None if raw is None else format(raw,'f'),close=normalized,
                factor_to_anchor=exact_text(factor),split=source.get('split'),
                unsupported_action=source.get('unsupported_action'),source_url=url))
        if not completed or previous!=anchor:
            raise ValueError('Incomplete calendar coverage')
        manifest=dict(instrument=query['instrument'],requested_start=query['start'],requested_end=query['end'],
            coverage_start=calendar_rows[0]['date'],anchor=query['anchor'],last_completed_session=completed[-1],
            currency='USD',exchange_timezone='America/New_York',calendar=metadata['calendar'],
            calendar_version=CALENDAR_VERSION,normalization_version=1,
            source_basis='total_return_index' if benchmark else 'yahoo_split_adjusted_close',
            completeness='unverified',retrieved_at=retrieved_at)
        result=dict(manifest=manifest,items=items)
        if len(json.dumps(result,separators=(',',':')).encode())>MAX_BYTES:
            raise ValueError('Historical snapshot exceeds byte limit')
        return result
    except ProviderError:
        raise
    except (ValueError, KeyError, TypeError, ArithmeticError) as exc:
        raise ProviderError('malformed_data',str(exc)) from exc

def history_params(params):
    from instruments import lookup_params
    try:
        if not isinstance(params,dict) or set(params)-{'instrument','start','end','anchor','cursor','page_size'}:
            raise ValueError('Unknown historical query fields')
        lookup_params({'instrument':params['instrument']})
        dates=[]
        for key in ('start','end','anchor'):
            if not isinstance(params[key],str) or not re.fullmatch(r'\d{4}-\d{2}-\d{2}',params[key]):
                raise ValueError('Expected ISO historical dates')
            dates.append(date.fromisoformat(params[key]))
        if not dates[0]<=dates[1]<=dates[2] or (dates[2]-dates[0]).days>=MAX_ROWS:
            raise ValueError('Invalid historical range')
        size=params.get('page_size',200)
        if type(size) is not int or not 1<=size<=200:
            raise ValueError('History page_size must be 1–200')
        cursor=params.get('cursor')
        if cursor is not None and (not isinstance(cursor,str) or not cursor or len(cursor)>256):
            raise ValueError('Invalid history cursor')
        return dict(params,page_size=size,cursor=cursor)
    except (KeyError,ValueError,TypeError) as exc:
        raise ProviderError('invalid_request',str(exc),-32602) from exc

class HistoricalProvider:
    def __init__(self,fetch,clock,recovery,calendar=session_days):
        self.fetch,self.clock,self.recovery,self.calendar=fetch,clock,recovery,calendar
        self.snapshot=None; self.next_cursor=None; self.offset=0

    def daily(self,params):
        query=history_params(params); cursor=query.pop('cursor')
        if cursor is None:
            self.snapshot=None; self.next_cursor=None; self.offset=0
            at=self.clock()
            if query['anchor']!=datetime.fromisoformat(at).astimezone(ZoneInfo('America/New_York')).date().isoformat():
                raise ProviderError('invalid_request','Historical normalization anchor must be the current New York date')
            # Both supported calendars determine the earliest possible required baseline.
            start=min(previous_session(name,query['start']) for name in ('NYSE','NASDAQ'))
            options=dict(interval='1d',start=start,end=(date.fromisoformat(query['anchor'])+timedelta(days=1)).isoformat(),
                auto_adjust=False,back_adjust=False,repair=False,rounding=False,actions=True,keepna=True,timeout=10)
            with redirect_stdout(sys.stderr):
                raw,metadata=self.recovery.run(lambda:self.fetch(query['instrument']['value'],options))
            at=self.clock()
            if query['anchor']!=datetime.fromisoformat(at).astimezone(ZoneInfo('America/New_York')).date().isoformat():
                raise ProviderError('unavailable','Historical retrieval crossed the normalization date; refresh again')
            if metadata.get('symbol')!=query['instrument']['value']:
                raise ProviderError('malformed_data','Historical source symbol mismatch')
            metadata=dict(metadata,calendar=calendar_name(metadata,query['instrument']['value']))
            calendar_rows=self.calendar(metadata['calendar'],query['start'],query['anchor'],at)
            rows=[]
            for item in raw:
                try:
                    stamp=item['Date']
                    if not stamp.tzinfo or stamp.utcoffset()!=stamp.astimezone(ZoneInfo('America/New_York')).utcoffset():
                        raise ValueError('Unverified historical exchange date')
                    day=stamp.date().isoformat()
                    if day<calendar_rows[0]['date']: continue
                    close=item['Close']
                    # pandas' missing cells are handled explicitly; infinity remains invalid.
                    import pandas as pd
                    close=None if pd.isna(close) else str(close)
                    split=item['Stock Splits']
                    if pd.isna(split): raise ValueError('Missing source split value')
                    unsupported='Source capital gain action requires review' if item.get('Capital Gains',0) else None
                    rows.append(dict(date=day,close=close,split=str(split),unsupported_action=unsupported))
                except (KeyError,TypeError,ValueError) as exc:
                    raise ProviderError('malformed_data','Incomplete historical source frame') from exc
            self.snapshot=(query,normalize_history(rows,metadata,query,at,calendar_rows))
        elif self.snapshot is None or cursor!=self.next_cursor or query!=self.snapshot[0]:
            raise ProviderError('invalid_request','Expired or mismatched history cursor',-32602)
        snapshot=self.snapshot[1]
        stop=self.offset+query['page_size']; items=snapshot['items'][self.offset:stop]; self.offset=stop
        self.next_cursor=secrets.token_urlsafe(24) if stop<len(snapshot['items']) else None
        return dict(manifest=snapshot['manifest'],items=items,next_cursor=self.next_cursor)
