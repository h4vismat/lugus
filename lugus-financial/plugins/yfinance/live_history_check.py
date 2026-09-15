"""Explicit public-source compatibility check; writes only the requested local artifact."""
import argparse
import json
from datetime import datetime, timedelta
from pathlib import Path
from zoneinfo import ZoneInfo
from decimal import Decimal
from provider import Provider

def main():
    parser=argparse.ArgumentParser();parser.add_argument('--output',required=True);args=parser.parse_args()
    anchor=datetime.now(ZoneInfo('America/New_York')).date()
    p=Provider();p.initialize({'protocol_version':1,'config':{}})
    checks=[]
    for symbol,start,end in [('AAPL','2020-08-28','2020-09-02'),('^SP500TR',(anchor-timedelta(days=40)).isoformat(),(anchor-timedelta(days=1)).isoformat())]:
        query=dict(instrument=dict(namespace='yahoo:symbol',value=symbol),start=start,end=end,anchor=anchor.isoformat(),page_size=200,cursor=None)
        try:
            rows=[];manifest=None
            while True:
                page=p.historical_daily(query);manifest=page['manifest'];rows.extend(page['items'])
                query['cursor']=page['next_cursor']
                if query['cursor'] is None:break
            selected=[r for r in rows if start<=r['date']<=end]
            assert any(r['close'] is not None for r in selected),'No actual historical prices'
            if symbol=='AAPL':
                before=next(r for r in rows if r['date']=='2020-08-28')
                split=next(r for r in rows if r['date']=='2020-08-31')
                assert split['split']==dict(numerator=4,denominator=1)
                assert Decimal(before['close'])/Decimal(before['source_close'])==Decimal(before['factor_to_anchor'])
                assert Decimal(before['factor_to_anchor'])/Decimal(split['factor_to_anchor'])==4
            else:
                assert manifest['source_basis']=='total_return_index','Incorrect index basis'
                assert all(r['source_close']==r['close'] for r in selected),'Index level changed during normalization'
                assert all(Decimal(r['close'])>0 for r in selected if r['close'] is not None),'Nonpositive index level'
            checks.append(dict(symbol=symbol,passed=True,manifest=manifest,observations=selected))
        except Exception as exc:
            checks.append(dict(symbol=symbol,passed=False,error=str(exc)))
    Path(args.output).write_text(json.dumps(checks,indent=2))
    print(json.dumps([dict(symbol=c['symbol'],passed=c['passed'],error=c.get('error')) for c in checks]))
    return 0 if all(c['passed'] for c in checks) else 1

if __name__=='__main__':raise SystemExit(main())
