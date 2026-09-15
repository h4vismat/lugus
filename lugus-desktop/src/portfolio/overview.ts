import type {View,PerformanceSummary} from './types';
import {node} from './forms';
import {formatUsd,formatMoney,formatPercent,overviewLabels} from './format';
import {renderAllocation} from './allocation';
import {renderHoldings} from './holdings';
export function renderOverview(root:HTMLElement,view:View,onOpen:(id:string,trigger:HTMLElement)=>void){
 const labels=overviewLabels(view);const grid=node('section',undefined,'portfolio-summary');grid.setAttribute('aria-label','Portfolio summary');
 const value=node('article');value.append(node('h3',labels.valueLabel),node('p',labels.value,'portfolio-number'),node('p',`${formatMoney(view.dashboard?.invested_market_value??null)} invested · ${formatUsd(view.valuation.cash)} cash`,'portfolio-muted'));grid.append(value);
 const cards=[['Portfolio return','Cash flows accounted for'],['S&P 500','Total return · dividends reinvested'],['Difference','Percentage points vs. benchmark']].map(([label,note])=>{const card=node('article');const title=node('h3',label),number=node('p','—','portfolio-number');card.append(title,number,node('p',note,'portfolio-muted'));grid.append(card);return{title,number,label};});root.append(grid);
 if(!view.valuation.complete)root.append(node('p','Some holdings are unpriced. The priced subtotal excludes them; total value and portfolio allocation are unavailable.','portfolio-notice'));
 const performanceRoot=node('section',undefined,'portfolio-card portfolio-performance');root.append(performanceRoot);renderAllocation(root,view);renderHoldings(root,view,onOpen);
 const details=node('details',undefined,'portfolio-accounting');details.append(node('summary','Accounting & price details'));const accounting=node('div',undefined,'portfolio-metrics');
 for(const [label,value]of [['Realized P&L',formatUsd(view.realized)],['Unrealized P&L',formatMoney(view.valuation.unrealized)],['Dividend income',formatUsd(view.dividends)],['Standalone fees',formatUsd(view.standalone_fees)],['Trade fees (included in P&L)',formatUsd(view.trade_fees)],['Deposits',formatUsd(view.deposits)],['Withdrawals',formatUsd(view.withdrawals)]]){const card=node('article');card.append(node('h3',label),node('p',value));accounting.append(card);}details.append(accounting,node('p',`Accounting as of ${view.as_of}. Sales use FIFO within each account.`));
 for(const a of view.accounts)details.append(node('p',`${a.name}: recorded activity starts ${a.start}.${a.simplified?' Opening lots include simplified purchase history.':''}`));
 for(const raw of view.price_status){let text=raw;for(const i of view.instruments)text=text.replaceAll(i.id,i.symbol);details.append(node('p',text));}root.append(details);
 return{performanceRoot,updateSummary(summary:PerformanceSummary|null,period:string,stale:boolean){const values=[summary?.portfolio_return_percent??null,summary?.benchmark_return_percent??null,summary?.difference_pp??null];cards.forEach((c,i)=>{c.title.textContent=`${c.label}${i<2?` · ${period}`:''}${stale?' · saved':''}`;c.number.textContent=i===2?formatPercent(values[i],true).replace('%',' pp'):formatPercent(values[i],true);c.number.className=`portfolio-number ${values[i]?.startsWith('-')?'portfolio-negative':i===1?'':'portfolio-positive'}`;});}};
}
