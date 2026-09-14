import type {LotRow,View} from './types';
import {node} from './forms';
import {formatUsd} from './format';
export function renderHoldings(root:HTMLElement,view:View,lots:LotRow[]){
 if(!view.valuation.holdings.length){root.append(node('p','No open positions. Add a purchase or opening lots to see holdings.'));return;}
 for(const h of view.valuation.holdings){const instrument=view.instruments.find(i=>i.id===h.instrument_id);const card=node('details',undefined,'portfolio-holding');
  card.append(node('summary',`${instrument?.symbol??h.instrument_id} · ${h.quantity} shares · ${formatUsd(h.market_value)}`));
  const list=node('dl');for(const [label,value]of [['Cost basis',formatUsd(h.basis)],['Price',h.price?`${formatUsd(h.price.close)} on ${h.price.date}`:'Unpriced'],['Unrealized P&L',formatUsd(h.unrealized)]]){list.append(node('dt',label),node('dd',value));}card.append(list);
  if(h.unpriced_reason)card.append(node('p',h.unpriced_reason,'portfolio-notice'));if(h.simplified)card.append(node('p','Includes simplified opening purchase history.'));
  const table=node('table');const header=node('tr');for(const label of ['Account','Acquired','Remaining shares','Remaining basis'])header.append(node('th',label));table.append(header);
  for(const r of lots.filter(r=>r.lot.instrument_id===h.instrument_id)){const row=node('tr');for(const v of [r.account_name,r.lot.acquired,r.lot.quantity,formatUsd(r.lot.basis)])row.append(node('td',v));table.append(row);}card.append(table);root.append(card);
 }
}
