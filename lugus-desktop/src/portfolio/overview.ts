import type {View} from './types';
import {node} from './forms';
import {formatUsd,overviewLabels} from './format';
export function renderOverview(root:HTMLElement,view:View){
 const labels=overviewLabels(view);const grid=node('div',undefined,'portfolio-metrics');
 const metrics:[[string,string],...[string,string][]]=[[labels.valueLabel,labels.value],['Cash',formatUsd(view.valuation.cash)],['Realized P&L',formatUsd(view.realized)],['Unrealized P&L',formatUsd(view.valuation.unrealized)],['Dividend income',formatUsd(view.dividends)],['Standalone fees',formatUsd(view.standalone_fees)],['Trade fees (already included in P&L)',formatUsd(view.trade_fees)],['Deposits',formatUsd(view.deposits)],['Withdrawals',formatUsd(view.withdrawals)]];
 for(const [label,value]of metrics){const card=node('article');card.append(node('h3',label),node('p',value));grid.append(card);}root.append(grid);
 root.append(node('p',`Accounting as of ${view.as_of}. Long-only stocks, ETFs and cash in USD. Sales use FIFO within each account.`));
 if(!view.valuation.complete)root.append(node('p','Some holdings are unpriced. The priced subtotal excludes them; total value, total unrealized P&L and portfolio allocation are unavailable.','portfolio-notice'));
 for(const account of view.accounts)root.append(node('p',`${account.name}: recorded activity starts ${account.start}.${account.simplified?' Opening lots include simplified purchase history; realized FIFO results may differ from your broker.':''}`));
 if(labels.allocationAvailable&&view.valuation.total_value!=='0'){
  const list=node('ul');for(const h of view.valuation.holdings){const instrument=view.instruments.find(i=>i.id===h.instrument_id);list.append(node('li',`${instrument?.symbol??h.instrument_id}: ${h.allocation_percent??'—'}%`));}list.append(node('li',`Cash: ${view.valuation.cash_allocation_percent??'—'}%`));root.append(node('h3','Allocation'),list);
 }
 if(view.price_status.length){const details=node('details');details.append(node('summary','Price refresh details'));for(const raw of view.price_status){let text=raw;for(const i of view.instruments)text=text.replace(i.id,i.symbol);details.append(node('p',text));}root.append(details);}
}
