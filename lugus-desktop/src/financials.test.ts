import {test} from 'node:test';
import assert from 'node:assert/strict';
import {financialListing,financialValue,financialStatement,financialStatements,financialPeriodGroups,filingChoices} from './financials.ts';
import type {ReportedFinancialFact} from './types.ts';

function fact(change:Partial<ReportedFinancialFact>={}):ReportedFinancialFact {
 return {company:{namespace:'sec:cik',value:'0001321655'},namespace:'us-gaap',concept:'Assets',label:'Assets',unit:'USD',value:'9007199254740993.123456789',period:{kind:'instant',date:'2025-12-31'},filed:'2026-02-15',form:'10-K',filing_id:'annual-new',fiscal_year:2025,fiscal_period:'FY',source_url:'https://www.sec.gov/example',retrieved_at:'2026-09-11T00:00:00Z',...change};
}
const facts=[
 fact({filing_id:'annual-old',filed:'2025-02-15',period:{kind:'instant',date:'2024-12-31'}}),
 fact(),
 fact({filing_id:'quarter-new',form:'10-Q',filed:'2026-08-04',period:{kind:'instant',date:'2026-06-30'}}),
 fact({filing_id:'proxy',form:'DEF 14A',filed:'2026-09-01',namespace:'ecd',concept:'Pay',label:null,value:'0'}),
 fact({concept:'Revenue',label:'Revenue',period:{kind:'duration',start:'2025-01-01',end:'2025-12-31'}}),
 fact({concept:'Assets',unit:'EUR',value:'1.25'}),
];
test('defaults to latest annual and quarterly filings while retaining every known metric',()=>{
 const listing=financialListing(facts);
 assert.deepEqual(listing.filings.map(f=>f.id).sort(),['annual-new','quarter-new']);
 assert.equal(listing.metricCount,4);
 assert.equal(listing.rows.length,5);
 const absent=listing.rows.find(r=>r.metric.concept==='Pay')!;
 assert.equal(absent.observation,null);
 assert.equal(listing.rows.filter(r=>r.metric.concept==='Assets'&&r.metric.unit==='USD').length,2);
 assert(!listing.rows.some(r=>r.observation?.filing_id==='annual-old'));
});
test('history exposes every original observation with exact decimal and optional fields',()=>{
 const listing=financialListing(facts,{filing:'all'});
 assert.equal(listing.total,facts.length);
 assert.equal(listing.rows.find(r=>r.metric.concept==='Pay')!.observation!.label,null);
 assert.equal(listing.rows.find(r=>r.observation?.filing_id==='annual-new'&&r.metric.unit==='USD')!.observation!.value,'9007199254740993.123456789');
});
test('explicit filing selection marks other metrics missing without borrowing older values',()=>{
 const selected=filingChoices(facts).find(f=>f.id==='quarter-new')!;
 const listing=financialListing(facts,{filing:selected.key});
 assert.equal(listing.rows.length,4);
 assert.equal(listing.rows.filter(r=>r.observation!==null).length,1);
 assert.equal(listing.rows.find(r=>r.metric.concept==='Revenue')!.observation,null);
 assert.equal(financialListing(facts,{filing:'unknown'}).filings.length,0);
 assert.equal(financialListing(facts,{filing:'unknown'}).rows.filter(r=>r.observation!==null).length,0);
});
test('search covers namespace, concept and labels, and pagination reaches more than 1000 rows',()=>{
 const many=Array.from({length:1103},(_,i)=>fact({concept:`Metric${String(i).padStart(4,'0')}`,label:null}));
 const last=financialListing(many,{page:22,pageSize:50});
 assert.equal(last.total,1103);assert.equal(last.rows.length,3);assert.equal(last.rows[2].metric.concept,'Metric1102');
 assert.equal(financialListing(facts,{search:'ecd pay',filing:'all'}).rows.length,1);
 assert.equal(financialListing(facts,{search:'revenue'}).rows.length,1);
 assert.equal(financialListing(facts,{search:'does not exist'}).total,0);
});
test('null and absent values are explicit while zero and exact precision survive',()=>{
 assert.equal(financialValue(null),'Missing');assert.equal(financialValue(undefined),'Missing');
 assert.equal(financialValue(''),'Missing');assert.equal(financialValue('0'),'0');
 assert.equal(financialValue('9007199254740993.123456789'),'9,007,199,254,740,993.123456789');
});
test('latest uses filing chronology, handles amendments and foreign forms without merging periods',()=>{
 const input=[fact(),fact({filing_id:'amendment',form:'10-K/A',filed:'2026-03-01'}),
 fact({filing_id:'foreign',form:'20-F',filed:'2026-04-01'}),
 fact({filing_id:'quarter',form:'10-Q',filed:'2026-08-01',period:{kind:'duration',start:'2026-01-01',end:'2026-06-30'}}),
 fact({filing_id:'quarter',form:'10-Q',filed:'2026-08-01',period:{kind:'duration',start:'2026-04-01',end:'2026-06-30'}})];
 const selected=financialListing(input);
 assert.deepEqual(selected.filings.map(f=>f.id).sort(),['amendment','annual-new','foreign','quarter']);
 assert.equal(selected.rows.filter(r=>r.observation?.filing_id==='quarter').length,2);
});
test('with no periodic forms latest uses newest source filing and empty data stays empty',()=>{
 const listing=financialListing([fact({form:'DEF 14A'})]);
 assert.equal(listing.filings.length,1);assert.equal(listing.periodic,false);
 assert.equal(financialListing([]).total,0);
});

test('groups by exact reporting period newest first before paginating, with missing metrics last',()=>{
 const input=[
  fact({concept:'Annual',period:{kind:'duration',start:'2025-01-01',end:'2025-12-31'}}),
  fact({concept:'Quarter',period:{kind:'duration',start:'2026-04-01',end:'2026-06-30'}}),
  fact({concept:'Ytd',period:{kind:'duration',start:'2026-01-01',end:'2026-06-30'}}),
  fact({concept:'Balance',period:{kind:'instant',date:'2026-06-30'}}),
  fact({concept:'Missing',filed:'2024-02-01',filing_id:'old'}),
  fact({concept:'QuarterOther',period:{kind:'duration',start:'2026-04-01',end:'2026-06-30'}}),
 ];
 const rows=financialListing(input).rows;
 const groups=financialPeriodGroups(rows);
 assert.equal(groups.length,5);
 assert.deepEqual(groups[0].rows.map(r=>r.metric.concept),['Quarter','QuarterOther']);
 assert.equal(groups[1].period?.start,'2026-01-01');
 assert.equal(groups[2].period?.kind,'instant');
 assert.equal(groups[3].period?.end,'2025-12-31');
 assert.equal(groups[4].period,undefined);
 assert.equal(groups[4].rows[0].observation,null);
 const paged=[0,1,2].flatMap(page=>financialListing(input,{page,pageSize:2}).rows);
 assert.deepEqual(paged,rows);
 assert.equal(financialPeriodGroups(financialListing(input,{search:'Quarter'}).rows).length,1);
});

test('statement categories use exact namespace/concept mappings and retain unknown metrics',()=>{
 for(const [concept,category] of [['Revenues','income'],['NetIncomeLoss','income'],['Assets','balance'],['StockholdersEquity','balance'],['NetCashProvidedByUsedInOperatingActivities','cash_flow'],['PaymentsToAcquirePropertyPlantAndEquipment','cash_flow'],['NotMapped','other']]){
  assert.equal(financialStatement({namespace:'us-gaap',concept}),category,concept);
 }
 assert.equal(financialStatement({namespace:'ecd',concept:'Assets'}),'other');
 assert.equal(financialStatement({namespace:'custom',concept:'Revenues'}),'other');
 assert.equal(financialStatement({namespace:'us-gaap',concept:'toString'}),'other');
});
test('statement sections partition every metric without changing filings, missing values or periods',()=>{
 const input=[fact({concept:'Revenues'}),fact({concept:'Assets'}),fact({concept:'PaymentsToAcquirePropertyPlantAndEquipment'}),fact({concept:'UnknownMetric'}),fact({concept:'GrossProfit',filing_id:'old',filed:'2024-02-01'})].map(f=>({...f,label:f.concept}));
 const sections=financialStatements.map(({id})=>financialListing(input,{statement:id}));
 assert.deepEqual(sections.map(s=>s.metricCount),[2,1,1,1]);
 assert.equal(sections.flatMap(s=>s.rows).length,financialListing(input).rows.length);
 assert.equal(sections[0].rows.find(r=>r.metric.concept==='GrossProfit')!.observation,null);
 assert(sections.every(s=>s.filings[0].id==='annual-new'));
 assert.equal(financialListing(input,{statement:'income',search:'Assets'}).total,0);
 assert.equal(financialListing(input,{statement:'income',filing:'all'}).rows.find(r=>r.metric.concept==='GrossProfit')!.observation!.filing_id,'old');
});
