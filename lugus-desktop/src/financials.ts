import {statementMappings} from './statement-mappings.ts';
import {exactNumber} from './state.ts';
import type {FinancialPeriod,ReportedFinancialFact} from './types';

export type FilingChoice={key:string;id:string;form:string;filed:string};
export type FinancialMetric={key:string;namespace:string;concept:string;label:string|null;unit:string};
export type FinancialListingRow={metric:FinancialMetric;observation:ReportedFinancialFact|null};
export type FinancialStatement='income'|'balance'|'cash_flow'|'other';
export const financialStatements:ReadonlyArray<{id:FinancialStatement;label:string}>=[{id:'income',label:'Income Statement'},{id:'balance',label:'Balance Sheet'},{id:'cash_flow',label:'Cash Flow'},{id:'other',label:'Other Metrics'}];
/** Concept categorization only; Company Facts does not contain filing presentation roles. */
export function financialStatement(metric:Pick<FinancialMetric,'namespace'|'concept'>):FinancialStatement {
 const mapped=metric.namespace==='us-gaap'&&Object.hasOwn(statementMappings,metric.concept)?statementMappings[metric.concept]:undefined;
 return mapped==='income'||mapped==='balance'||mapped==='cash_flow'?mapped:'other';
}
export type FinancialFilter={statement?:FinancialStatement;filing?:string;search?:string;page?:number;pageSize?:number};
const compare=(a:string,b:string)=>a<b?-1:a>b?1:0;
const metricKey=(f:ReportedFinancialFact)=>JSON.stringify([f.namespace,f.concept,f.unit]);
const filingKey=(f:ReportedFinancialFact)=>JSON.stringify([f.filing_id,f.form,f.filed]);
const periodKey=(period:FinancialPeriod|undefined)=>JSON.stringify(period?[period.kind,period.date??null,period.start??null,period.end??null]:null);
const periodEnd=(period:FinancialPeriod|undefined)=>period?.date??period?.end??'';
export function financialPeriodGroups(rows:FinancialListingRow[]) {
 const groups=new Map<string,{key:string;period:FinancialPeriod|undefined;rows:FinancialListingRow[]}>();
 for(const row of rows){
  const period=row.observation?.period,key=periodKey(period);
  if(!groups.has(key))groups.set(key,{key,period,rows:[]});
  groups.get(key)!.rows.push(row);
 }
 return [...groups.values()].sort((a,b)=>compare(periodEnd(b.period),periodEnd(a.period))||compare(b.period?.start??'',a.period?.start??'')||compare(a.key,b.key));
}
const family=(form:string)=>form.replace(/\/A$/,'');
const periodicForms=new Set(['10-K','10-Q','20-F','40-F']);

export function financialValue(value:string|null|undefined):string {
 return value===null||value===undefined||value===''?'Missing':exactNumber(value);
}

/** Distinct source filings, ordered by filing date. Never infer reporting dates. */
export function filingChoices(facts:ReportedFinancialFact[]):FilingChoice[] {
 const filings=new Map<string,FilingChoice>();
 for(const fact of facts){const key=filingKey(fact);filings.set(key,{key,id:fact.filing_id,form:fact.form,filed:fact.filed});}
 return [...filings.values()].sort((a,b)=>compare(b.filed,a.filed)||compare(b.id,a.id)||compare(a.form,b.form));
}

/** Pure presentation over one saved snapshot; no selection changes source values. */
export function financialListing(facts:ReportedFinancialFact[],filter:FinancialFilter={}) {
 const choices=filingChoices(facts);
 const periodic=choices.some(f=>periodicForms.has(family(f.form)));
 let filings:FilingChoice[];
 if((filter.filing??'latest')==='latest'){
  const latest=new Map<string,string>();
  const originals=new Map<string,string>();
  for(const f of choices)if(periodicForms.has(f.form)&&!originals.has(f.form))originals.set(f.form,f.filed);
  filings=choices.filter(f=>{
   if(periodic&&!periodicForms.has(family(f.form)))return false;
   const key=periodic?family(f.form):'any';
   const original=originals.get(key);
   if(original)return f.form.endsWith('/A')?f.filed>=original:f.filed===original;
   if(!latest.has(key))latest.set(key,f.filed);
   // Retain same-day accessions rather than arbitrarily choosing among them.
   return latest.get(key)===f.filed;
  });
 }else filings=filter.filing==='all'?choices:choices.filter(f=>f.key===filter.filing);
 const selected=new Set(filings.map(f=>f.key));
 const metrics=new Map<string,FinancialMetric>();
 const observations=new Map<string,ReportedFinancialFact[]>();
 for(const fact of facts){
  const key=metricKey(fact);
  if(!metrics.has(key))metrics.set(key,{key,namespace:fact.namespace,concept:fact.concept,label:fact.label,unit:fact.unit});
  if(selected.has(filingKey(fact))){const rows=observations.get(key)??[];rows.push(fact);observations.set(key,rows);}
 }
 const terms=(filter.search??'').trim().toLowerCase().split(/\s+/).filter(Boolean);
 const matching=[...metrics.values()].filter(m=>!filter.statement||financialStatement(m)===filter.statement).filter(m=>terms.every(t=>`${m.namespace} ${m.concept} ${m.label??''} ${m.unit}`.toLowerCase().includes(t)))
  .sort((a,b)=>compare(a.namespace,b.namespace)||compare(a.concept,b.concept)||compare(a.unit,b.unit));
 const rows:FinancialListingRow[]=[];
 for(const metric of matching){
  const reported=observations.get(metric.key)??[];
  if(!reported.length)rows.push({metric,observation:null});
  else for(const observation of [...reported].sort((a,b)=>compare(b.filed,a.filed)||compare(b.period.date??b.period.end??'',a.period.date??a.period.end??'')||compare(b.period.start??'',a.period.start??'')||compare(a.filing_id,b.filing_id)))rows.push({metric,observation});
 }
 const groupedRows=financialPeriodGroups(rows).flatMap(group=>group.rows);
 const pageSize=Number.isSafeInteger(filter.pageSize)&&filter.pageSize!>0?Math.min(filter.pageSize!,100):50;
 const requested=Number.isSafeInteger(filter.page)&&filter.page!>=0?filter.page!:0;
 const page=Math.min(requested,Math.max(0,Math.ceil(rows.length/pageSize)-1));
 return {rows:groupedRows.slice(page*pageSize,(page+1)*pageSize),total:rows.length,metricCount:matching.length,
  missingCount:rows.filter(r=>r.observation===null).length,page,pageSize,filings,periodic};
}
