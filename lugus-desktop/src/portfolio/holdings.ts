import type {View} from './types';
import {node,button} from './forms';
import {formatMoney,formatPercent} from './format';
import {sortHoldings,type HoldingSort} from './dashboard-state';
export function renderHoldings(root:HTMLElement,view:View,onOpen:(id:string,trigger:HTMLElement)=>void){
 const card=node('section',undefined,'portfolio-card');card.append(node('h2','Holdings'),node('p',`${view.valuation.holdings.length} positions · Select a holding to explore its details`,'portfolio-muted'));root.append(card);
 if(!view.valuation.holdings.length){card.append(node('p','No open positions. Add a purchase or opening lots to see holdings.'));return;}
 let key:HoldingSort='market_value';let direction:'asc'|'desc'='desc';const wrap=node('div',undefined,'portfolio-tablewrap');const table=node('table',undefined,'portfolio-holdings-table');wrap.append(table);card.append(wrap);
 const draw=()=>{
  table.replaceChildren();const thead=node('thead'),tr=node('tr');
  for(const [field,label]of [['symbol','Holding'],['market_value','Market value'],['allocation_percent','Weight'],['basis','Cost basis'],['unrealized','Gain / loss'],['unrealized_percent','Return']]as const){const th=node('th');th.scope='col';if(field===key)th.setAttribute('aria-sort',direction==='asc'?'ascending':'descending');th.append(button(`${label}${field===key?(direction==='asc'?' ↑':' ↓'):''}`,()=>{direction=field===key&&direction==='desc'?'asc':'desc';key=field;draw();}));tr.append(th);}thead.append(tr);table.append(thead);
  const body=node('tbody');for(const h of sortHoldings(view,key,direction)){
   const instrument=view.instruments.find(i=>i.id===h.instrument_id);const row=node('tr');const name=node('td');const open=button(instrument?.symbol??h.instrument_id,()=>onOpen(h.instrument_id,open));name.append(open,node('small',instrument?.name??''));row.append(name);
   const gain=view.dashboard?.holdings.find(m=>m.instrument_id===h.instrument_id)?.unrealized_percent??null;
   for(const [i,v]of [formatMoney(h.market_value),formatPercent(h.allocation_percent),formatMoney(h.basis),formatMoney(h.unrealized),formatPercent(gain,true)].entries()){const td=node('td',v);if(i>=3&&h.unrealized!==null)td.className=h.unrealized.startsWith('-')?'portfolio-negative':'portfolio-positive';row.append(td);}body.append(row);
  }table.append(body);
 };draw();card.append(node('p',`${view.valuation.complete?'All holdings priced':'Some holdings are unpriced'} · USD · Prices and lot details are available on each holding.`,'portfolio-muted'));
}
