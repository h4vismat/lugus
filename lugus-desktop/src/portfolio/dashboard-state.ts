import type {Holding,View} from './types';
import {compareDecimal} from './format.ts';
export type HoldingSort='symbol'|'market_value'|'allocation_percent'|'basis'|'unrealized'|'unrealized_percent';
export function sortHoldings(view:View,key:HoldingSort,direction:'asc'|'desc'):Holding[]{
 const symbol=(id:string)=>view.instruments.find(i=>i.id===id)?.symbol??id;
 const value=(h:Holding)=>key==='unrealized_percent'?view.dashboard?.holdings.find(m=>m.instrument_id===h.instrument_id)?.unrealized_percent??null:key==='symbol'?null:h[key];
 return [...view.valuation.holdings].sort((a,b)=>{
  const order=key==='symbol'?symbol(a.instrument_id).localeCompare(symbol(b.instrument_id))*(direction==='asc'?1:-1):compareDecimal(value(a),value(b),direction);
  return order||symbol(a.instrument_id).localeCompare(symbol(b.instrument_id))||a.instrument_id.localeCompare(b.instrument_id);
 });
}
