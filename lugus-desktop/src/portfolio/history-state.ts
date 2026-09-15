export type Period='1M'|'3M'|'YTD'|'1Y'|'All';
export type ChartMode='return'|'value';
export interface HistorySelection {portfolioId:string;accountId:string|null;revision:string;generation:number;period:Period;resultId:string|null}
export function acceptsHistory(current:HistorySelection,origin:HistorySelection):boolean{return current.portfolioId===origin.portfolioId&&current.accountId===origin.accountId&&current.revision===origin.revision&&current.generation===origin.generation&&current.period===origin.period&&current.resultId===origin.resultId;}
export function historyRange(period:Period,end:string):{start:string|null;end:string}{
 if(period==='All')return{start:null,end};const [year,month,day]=end.split('-').map(Number);
 if(period==='YTD')return{start:`${year}-01-01`,end};
 const months=period==='1M'?1:period==='3M'?3:12;const start=new Date(Date.UTC(year,month-1-months,1));const last=new Date(Date.UTC(start.getUTCFullYear(),start.getUTCMonth()+1,0)).getUTCDate();start.setUTCDate(Math.min(day,last));return{start:start.toISOString().slice(0,10),end};
}
