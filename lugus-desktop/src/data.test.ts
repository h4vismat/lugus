import {test} from 'node:test';
import assert from 'node:assert/strict';
import {readSavedWindow,readSavedFacts,selectedResearch} from './data.ts';
import type {Dataset,Price,ResearchView} from './types.ts';

const prices:Price[]=['10.0001',null,'12.50','13',null,'15.12345678901234567890','16'].map((value,index)=>({kind:'price',value,evidence:{value:{date:`2026-01-0${index+1}`,currency:'USD',close:value??'',retrieved_at:'2026-01-08T00:00:00Z',source_url:'https://example.test/prices',instrument:{namespace:'test',value:'stock'}}}}));
const header:Dataset['header']={id:'saved-prices',kind:'prices',binding_id:null,row_count:prices.length,query:{},created_at:'2026-01-08T00:00:00Z',provider:{instance_id:'test',plugin_id:'fixture',plugin_version:'1'},limitations:[],conflicts:[],error:null,coverage:{completeness:'complete'}};
function page(offset:number):Dataset {const end=Math.min(offset+2,prices.length);return {header,rows:prices.slice(offset,end),next_offset:end<prices.length?end:null};}

test('reads remaining pages without changing exact values or null gaps',async()=>{
 const offsets:number[]=[];const result=await readSavedWindow(page(0),async offset=>{offsets.push(offset);return page(offset);});
 assert.deepEqual(offsets,[2,4,6]);assert.equal(result.header,header);assert.deepEqual(result.rows,prices);assert.equal(result.next_offset,null);
});
test('bounds a large saved dataset to its newest suffix including its final price',async()=>{
 const offsets:number[]=[];const result=await readSavedWindow(page(0),async offset=>{offsets.push(offset);return page(offset);},4);
 assert.deepEqual(offsets,[3,5]);assert.equal(result.header.row_count,7);assert.equal(result.header,header);
 assert.deepEqual(result.rows.map(row=>(row as Price).value),['13',null,'15.12345678901234567890','16']);assert.equal(result.next_offset,null);
});
test('a complete first page needs no additional request',async()=>{
 const first={header,rows:prices,next_offset:null};const result=await readSavedWindow(first,async()=>{throw Error('unexpected read');});assert.deepEqual(result,first);
});
test('rejects nonadvancing, skipped, truncated, or mixed-dataset pages',async()=>{
 for(const bad of [
  {...page(2),next_offset:2},
  {...page(2),next_offset:5},
  {...page(2),next_offset:null},
  {...page(2),header:{...header,id:'different'}},
  {...page(2),header:{...header,row_count:8}},
  {...page(2),header:{...header,error:{message:'source failed'}}},
 ])await assert.rejects(readSavedWindow(page(0),async()=>bad));
});
test('rejects invalid bounds and propagates source errors',async()=>{
 for(const maximum of [0,-1,1.5,Infinity])await assert.rejects(readSavedWindow(page(0),async offset=>page(offset),maximum));
 await assert.rejects(readSavedWindow({...page(0),header:{...header,error:{message:'source failed'}}},async offset=>page(offset)),/source failed/);
 await assert.rejects(readSavedWindow(page(0),async()=>{throw Error('read failed');}),/read failed/);
});
test('missing explicit research selection never falls back to unrelated evidence',()=>{
 const item:ResearchView={view:{id:'chart',dataset_id:header.id,kind:'price_chart',descriptor_revision:1},data:page(0)};
 assert.equal(selectedResearch([item],'chart'),item);assert.equal(selectedResearch([item],'document'),undefined);assert.equal(selectedResearch([item],null),item);assert.equal(selectedResearch([],null),undefined);
});

test('complete fact loading retains older rows beyond the price window',async()=>{
 const count=1203;
 const factHeader={...header,kind:'facts',row_count:count};
 const rows=Array.from({length:count},(_,i)=>({...prices[0],value:String(i)}));
 const read=(offset:number):Dataset=>({header:factHeader,rows:rows.slice(offset,offset+100),next_offset:offset+100<count?offset+100:null});
 const loaded=await readSavedFacts(read(0),async offset=>read(offset));
 assert.equal(loaded.rows.length,count);assert.equal((loaded.rows[0] as Price).value,'0');
 assert.equal((loaded.rows.at(-1) as Price).value,'1202');
});
test('partial failed fact snapshots remain visible with their error and exact rows',async()=>{
 const first={header:{...header,kind:'facts',row_count:1,error:{message:'Source timed out'}},rows:[prices[0]],next_offset:null};
 const loaded=await readSavedFacts(first,async()=>{throw Error('unexpected');});
 assert.equal(loaded.header.error?.message,'Source timed out');assert.equal(loaded.rows.length,1);
});


test('default price and fact readers preserve all rows beyond former desktop caps',async()=>{
 const count=100_001;
 const largeHeader={...header,row_count:count};
 const read=(offset:number):Dataset=>{
  const end=Math.min(offset+200,count);
  return {header:largeHeader,rows:Array.from({length:end-offset},()=>prices[0]),next_offset:end<count?end:null};
 };
 for(const loader of [readSavedWindow,readSavedFacts]){
  const loaded=await loader(read(0),async offset=>read(offset));
  assert.equal(loaded.rows.length,count);
  assert.equal(loaded.next_offset,null);
 }
});
