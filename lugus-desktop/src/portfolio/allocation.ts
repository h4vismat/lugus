import type {View} from './types';
import {node,button} from './forms';
import {formatMoney,formatPercent,compareDecimal} from './format';
export function renderAllocation(root:HTMLElement,view:View){
 const wrap=node('div',undefined,'portfolio-below');const allocation=node('section',undefined,'portfolio-card');const head=node('div',undefined,'portfolio-cardhead');head.append(node('h2','Allocation'));const modes=node('div',undefined,'portfolio-segmented');head.append(modes);allocation.append(head);const rows=node('div');allocation.append(rows);
 const holdings=view.valuation.holdings.map(h=>({key:h.instrument_id,label:view.instruments.find(i=>i.id===h.instrument_id)?.symbol??h.instrument_id,value:h.market_value,percent:h.allocation_percent})).sort((a,b)=>compareDecimal(a.percent,b.percent,'desc'));
 holdings.push({key:'cash',label:'Cash',value:view.valuation.cash,percent:view.valuation.cash_allocation_percent});
 const draw=(kind:'holdings'|'types')=>{
  for(const b of modes.querySelectorAll('button'))b.setAttribute('aria-pressed',String(b.dataset.mode===kind));rows.replaceChildren();
  if(!view.valuation.complete){rows.append(node('p','Allocation is unavailable until every holding has a compatible price.','portfolio-muted'));return;}
  const entries=kind==='holdings'?holdings:view.dashboard?.by_asset_type??[];
  if(!entries.length||view.valuation.total_value==='0'){rows.append(node('p','Fund an account to see your allocation.','portfolio-muted'));return;}
  for(const [i,a]of entries.entries()){
   const line=node('div',undefined,'portfolio-barrow');const label=node('span',a.label);label.title=formatMoney(a.value);const track=node('div',undefined,'portfolio-track');const fill=node('div',undefined,'portfolio-fill');fill.style.width=`${Math.min(100,Math.max(0,Number(a.percent??0)))}%`;fill.style.opacity=String(Math.max(.3,1-i*.11));track.append(fill);line.append(label,track,node('span',formatPercent(a.percent)));rows.append(line);
  }
 };
 for(const [mode,label]of [['holdings','Holdings'],['types','Asset type']]as const){const b=button(label,()=>draw(mode));b.dataset.mode=mode;modes.append(b);}draw('holdings');
 const concentration=node('section',undefined,'portfolio-card');concentration.append(node('h2','Concentration & cash'));const metric=view.dashboard;
 if(metric?.largest_weight!==null&&metric?.largest_weight!==undefined){const instrument=view.instruments.find(i=>i.id===metric.largest_instrument_id);concentration.append(node('p',`${formatPercent(metric.largest_weight)} in your largest holding`,'portfolio-concentration'),node('p',instrument?.name??'','portfolio-muted'),node('p',`Your two largest positions make up ${formatPercent(metric.top_two_weight)} of the portfolio.`,'portfolio-insight'));}
 else concentration.append(node('p',view.valuation.complete?'No open security positions.':'Concentration is unavailable while holdings are unpriced.','portfolio-muted'));
 concentration.append(node('p',`${formatMoney(view.valuation.cash)} cash · ${formatPercent(view.valuation.cash_allocation_percent)}`,'portfolio-insight'),node('p','Cash is included in portfolio performance.','portfolio-muted'));wrap.append(allocation,concentration);root.append(wrap);
}
