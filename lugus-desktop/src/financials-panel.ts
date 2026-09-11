import {element} from './dom';
import {financialListing,financialValue,financialStatements,type FinancialStatement,financialPeriodGroups,filingChoices,type FinancialListingRow} from './financials';
import type {FinancialPeriod,ResearchView} from './types';

const missing=(value:unknown)=>value===null||value===undefined||value===''?'Missing':String(value);
const periodText=(period:FinancialPeriod|undefined)=>!period?'Missing':period.kind==='instant'?missing(period.date):`${missing(period.start)} – ${missing(period.end)}`;
const humanize=(name:string)=>name.replace(/([a-z])([A-Z])/g,'$1 $2');

function observationDetails(row:FinancialListingRow){
 const detail=element('details',undefined,'financial-details');detail.append(element('summary','Source details'));
 const fact=row.observation;
 const fields:[string,unknown][]=[['SEC concept',`${row.metric.namespace}:${row.metric.concept}`],['Source label',fact?.label],
  ['Value',fact?financialValue(fact.value):null],['Unit',row.metric.unit],['Period type',fact?.period.kind],
  ['Reporting period',fact?periodText(fact.period):null],['Form',fact?.form],['Filed',fact?.filed],
  ['Fiscal year',fact?.fiscal_year],['Fiscal period',fact?.fiscal_period],['Accession',fact?.filing_id],
  ['Retrieved',fact?.retrieved_at],['Source',fact?.source_url]];
 const list=element('dl');for(const [label,value] of fields)list.append(element('dt',label),element('dd',missing(value)));
 detail.append(list);return detail;
}

/** Render one full saved snapshot, keeping filter state local to that snapshot. */
export function financialsPanel(item:ResearchView|undefined):HTMLElement {
 const section=element('section',undefined,'financial-section full-financials');
 section.append(element('h3','Reported financials'));
 if(!item){section.append(element('p','No saved financial data is available for this company. Ask Lugus to research its financials.','evidence-note'));return section;}
 const facts=item.data.rows.flatMap(row=>row.kind==='reported_fact'?[row.evidence.value]:[]);
 const complete=item.data.header.policy==='all-reported-facts-v1';
 if(!complete){
  section.append(element('p','This saved view contains selected metrics only. Start new financial research to open every available SEC metric.','evidence-note'));
  for(const row of item.data.rows){
   if(row.kind!=='fact')continue;
   const fact=row.group.candidates[0]?.value;
   const metric=element('div',undefined,'metric');metric.append(element('span',fact?.label||humanize(fact?.concept??String(item.data.header.query.concept??'Reported metric'))),element('strong',financialValue(row.group.value)));
   metric.append(element('small',`${periodText(row.group.period)} · Filed ${missing(row.group.filed)} · ${missing(fact?.unit)}`));
   if(!fact?.label)metric.append(element('small','Source label: Missing'));
   if(row.group.conflict)metric.append(element('small',`Conflicting reports: ${row.group.conflict}`,'conflict'));
   section.append(metric);
  }
  if(!item.data.rows.length)section.append(element('p',`${humanize(String(item.data.header.query.concept??'Financial values'))}: Missing`,'evidence-note'));
  if(item.data.header.error)section.append(element('p',item.data.header.error.message,'evidence-note'));
  return section;
 }
 if(item.data.header.error)section.append(element('p',`Incomplete source retrieval: ${item.data.header.error.message}`,'evidence-note'));
 if(facts.length!==item.data.header.row_count){section.append(element('p','The financial listing is incomplete. Reopen this research view to retry loading its saved data.','evidence-note'));return section;}
 if(!facts.length){section.append(element('p','Financial values: Missing. No observations were returned for this company and filing-date range.','evidence-note'));return section;}
 let statement:FinancialStatement='income';
 const navigation=element('div',undefined,'financial-statement-nav');navigation.setAttribute('role','group');navigation.setAttribute('aria-label','Financial statement');
 const statementButtons=financialStatements.map(choice=>{
  const button=element('button',choice.label);button.type='button';button.setAttribute('aria-pressed',String(choice.id===statement));
  button.onclick=()=>{statement=choice.id;page=0;draw();container.scrollTop=0;};navigation.append(button);return button;
 });
 section.append(navigation);
 const controls=element('div',undefined,'financial-controls');
 const filingLabel=element('label','Filings');const select=element('select');select.setAttribute('aria-label','Financial filings');
 for(const [value,text] of [['latest','Latest annual & quarterly'],['all','All filings']]){const option=element('option',text);option.value=value;select.append(option);}
 for(const filing of filingChoices(facts)){const option=element('option',`${filing.form} · ${filing.filed} · ${filing.id}`);option.value=filing.key;select.append(option);}
 filingLabel.append(select);
 const searchLabel=element('label','Find a metric');const search=element('input');search.type='search';search.placeholder='Name, SEC concept or unit';search.setAttribute('aria-label','Find a financial metric');searchLabel.append(search);
 controls.append(filingLabel,searchLabel);section.append(controls);
 const scope=element('p',undefined,'financial-scope');const count=element('p',undefined,'financial-count');count.setAttribute('role','status');
 const container=element('div',undefined,'financial-table-wrap');container.tabIndex=0;container.setAttribute('role','region');container.setAttribute('aria-label','Financial observations');
 const pagination=element('div',undefined,'financial-pagination');
 const statementTitle=element('h4');section.append(scope,statementTitle,count,container,pagination);
 const query=item.data.header.query;
 section.append(element('p',`Saved filing dates: ${missing(query.filed_from)} – ${missing(query.filed_to)}. ${facts.length.toLocaleString()} source observations. Retrieved ${missing(facts[0]?.retrieved_at)} · ${item.data.header.provider.instance_id}`,'source-note'));
 section.append(element('p','Metrics are categorized by standard SEC concepts, not the filing’s original statement layout. Unmapped metrics remain in Other Metrics. Each row retains its original period and filing. Missing means no observation in the selected filings, or an absent source field; it is never a zero. Annual, quarterly and year-to-date durations remain separate.','evidence-note'));
 let page=0;
 function draw(){
  const listing=financialListing(facts,{statement,filing:select.value,search:search.value,page});page=listing.page;
  statementTitle.textContent=financialStatements.find(choice=>choice.id===statement)!.label;
  statementButtons.forEach((button,index)=>button.setAttribute('aria-pressed',String(financialStatements[index].id===statement)));
  if(select.value==='latest')scope.textContent=`${listing.periodic?'Latest saved annual and quarterly filings, including subsequent amendments':'No annual or quarterly filings are saved; showing the newest available filing'}: ${listing.filings.map(f=>`${f.form} (${f.filed})`).join(', ')}.`;
  else scope.textContent=select.value==='all'?'All filings in this saved date range.':`Selected filing: ${listing.filings.map(f=>`${f.form} (${f.filed})`).join(', ')}.`;
  count.textContent=`${listing.metricCount.toLocaleString()} metrics · ${listing.total.toLocaleString()} rows${listing.missingCount?` · ${listing.missingCount.toLocaleString()} not reported in selected filings`:''}`;
  const table=element('table',undefined,'financial-table');const head=element('thead');const headings=element('tr');
  for(const name of ['Metric / source','Value','Unit','Reporting period','Filing']){const th=element('th',name);th.scope='col';headings.append(th);}head.append(headings);table.append(head);
  for(const group of financialPeriodGroups(listing.rows)){
  const body=element('tbody');const heading=element('tr',undefined,'financial-period-heading');
  const title=element('th',group.period?`${group.period.kind==='instant'?'As of':'Period'} ${periodText(group.period)}`:'Not reported in selected filings');title.colSpan=5;title.scope='rowgroup';heading.append(title);body.append(heading);
  for(const row of group.rows){
   const tr=element('tr');const fact=row.observation;const name=element('td');
   name.append(element('strong',fact?.label||row.metric.label||humanize(row.metric.concept)),element('span',`${row.metric.namespace}:${row.metric.concept}`,'financial-concept'));
   if(fact&&!fact.label)name.append(element('span','Source label: Missing','missing-field'));
   name.append(observationDetails(row));
   const value=element('td',financialValue(fact?.value),fact?.value==null?'missing-field':'financial-value');
   if(!fact)value.append(element('small','Not reported in selected filings'));
   tr.append(name,value,element('td',missing(row.metric.unit)),element('td',periodText(fact?.period)),element('td',fact?`${fact.form} · ${fact.filed}`:'Missing'));body.append(tr);
  }
  table.append(body);
  }
  container.replaceChildren(table);
  if(!listing.total)container.replaceChildren(element('p',search.value.trim()?'No metrics in this section match this search.':'No mapped metrics are available in this section. Check Other Metrics for additional SEC disclosures.','evidence-note'));
  const previous=element('button','Previous');previous.type='button';previous.disabled=page===0;
  previous.onclick=()=>{page--;draw();container.scrollTop=0;};
  const next=element('button','Next');next.type='button';next.disabled=(page+1)*listing.pageSize>=listing.total;
  next.onclick=()=>{page++;draw();container.scrollTop=0;};
  pagination.replaceChildren(previous,element('span',`Page ${page+1} of ${Math.max(1,Math.ceil(listing.total/listing.pageSize))}`),next);
 }
 select.onchange=()=>{page=0;draw();};search.oninput=()=>{page=0;draw();};draw();return section;
}
