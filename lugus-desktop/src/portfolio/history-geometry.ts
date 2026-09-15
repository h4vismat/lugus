import type {PerformancePoint} from './types.ts';
import type {ChartMode} from './history-state.ts';
import {compareDecimal} from './format.ts';
const fields=['value','portfolio_return_percent','segment_return_percent','benchmark_return_percent','hypothetical_value']as const;
export function hasFlow(row:PerformancePoint){return [row.deposits,row.withdrawals,row.opening_contribution].some(v=>v!==null&&compareDecimal(v,'0')!==0);}
export function sampleHistory(rows:PerformancePoint[],budget:number):PerformancePoint[]{
 if(rows.length<=budget)return rows;const keep=new Set<number>([0,rows.length-1]);
 rows.forEach((row,i)=>{if(hasFlow(row))keep.add(i);if(i>0&&(row.segment!==rows[i-1].segment||fields.some(f=>(row[f]===null)!==(rows[i-1][f]===null)))){keep.add(i);keep.add(i-1);if(i+1<rows.length)keep.add(i+1);}});
 const buckets=Math.max(1,Math.floor(budget/10));const width=Math.ceil(rows.length/buckets);
 for(let start=0;start<rows.length;start+=width){const end=Math.min(rows.length,start+width);for(const field of fields){let min=-1,max=-1;for(let i=start;i<end;i++){if(rows[i][field]===null)continue;if(min<0||compareDecimal(rows[i][field],rows[min][field])<0)min=i;if(max<0||compareDecimal(rows[i][field],rows[max][field])>0)max=i;}if(min>=0)keep.add(min);if(max>=0)keep.add(max);}}
 return [...keep].sort((a,b)=>a-b).map(i=>rows[i]);
}
export function portfolioPlotValue(row:PerformancePoint,mode:ChartMode){return mode==='value'?row.value:row.segment>0?row.segment_return_percent:row.portfolio_return_percent;}
export function historyGeometry(rows:PerformancePoint[],mode:ChartMode){
 const value=(r:PerformancePoint,benchmark=false)=>benchmark?(mode==='return'?r.benchmark_return_percent:r.hypothetical_value):portfolioPlotValue(r,mode);
 const numbers=rows.flatMap(r=>[value(r),value(r,true)]).filter((v):v is string=>v!==null).map(Number).filter(Number.isFinite);
 let min=numbers.length?numbers.reduce((a,b)=>Math.min(a,b),Infinity):0,max=numbers.length?numbers.reduce((a,b)=>Math.max(a,b),-Infinity):1;if(min===max){const pad=Math.max(Math.abs(min)*.05,1);min-=pad;max+=pad;}
 const begin=rows.length?Date.parse(rows[0].date):0,end=rows.length?Date.parse(rows.at(-1)!.date):1;
 const points=rows.map(r=>{const y=(v:string|null)=>v===null?null:230-(Number(v)-min)/(max-min)*200;return{date:r.date,x:70+(Date.parse(r.date)-begin)/Math.max(1,end-begin)*900,portfolioY:y(value(r)),benchmarkY:y(value(r,true))};});
 const paths=(benchmark:boolean)=>{const out:string[]=[];let current='';points.forEach((p,i)=>{const y=benchmark?p.benchmarkY:p.portfolioY;const reset=!benchmark&&mode==='return'&&i>0&&rows[i].segment!==rows[i-1].segment;if(y===null||reset){if(current)out.push(current);current='';}if(y!==null)current+=`${current?' L':'M'}${p.x.toFixed(2)},${y.toFixed(2)}`;});if(current)out.push(current);return out;};
 return{portfolio:paths(false),benchmark:paths(true),points,min,max};
}
