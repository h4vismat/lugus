import test from 'node:test';import assert from 'node:assert/strict';
import {ComparisonController} from './controller.ts';
import type {Rpc} from '../portfolio/api.ts';
const input={subjects:[{text:'AAPL',exchange:null},{text:'MSFT',exchange:null}],period_end:'2024-12-31',years:3,revenue_basis:'contract_revenue_excluding_tax',question:null,facts_instance:null,resolution_instance:null,previous_id:null} as const;
const record=(id:string)=>({id,package_id:'p',request:{...input,request_id:'original'},companies:[],row_count:0,source_count:0,state:'complete',created_at:'2026-09-30T00:00:00Z',issues:[],previous_id:null});
const empty={items:[],next_offset:null};
test('submission retry preserves identity; refresh has new identity and retains saved result on failure',async()=>{
 const sent:Record<string,unknown>[]=[];let fail=true;
 const rpc:Rpc=async<T>(raw:object)=>{const c=(raw as {command:Record<string,unknown>}).command;if(c.kind==='providers')return {facts:[],resolution:[]} as T;if(c.kind==='read')return record(String(c.id)) as T;if(c.kind==='start'){sent.push(c.request as Record<string,unknown>);if(fail){fail=false;throw Error('Connection lost');}return {id:'job',state:'failed',comparison_id:null,error:{message:'Source unavailable'}} as T;}return empty as T;};
 const c=new ComparisonController(rpc);await c.open('conversation','AAPL');await c.select('saved');await c.create(input);assert.match(c.getSnapshot().error,/Connection lost/);await c.create(input);assert.equal(sent[0].request_id,sent[1].request_id);assert.equal(c.getSnapshot().selected?.id,'saved');await c.refresh(c.getSnapshot().selected!);assert.notEqual(sent[2].request_id,sent[1].request_id);assert.equal(sent[2].previous_id,'saved');c.dispose();
});
test('opening saved research only reads and stale responses cannot replace another company',async()=>{
 let release!:(value:unknown)=>void;const calls:string[]=[];const rpc:Rpc=async<T>(raw:object)=>{const c=(raw as {command:Record<string,unknown>}).command;calls.push(String(c.kind));if(c.kind==='providers')return {facts:[],resolution:[]} as T;if(c.kind==='read'&&c.id==='old')return await new Promise<unknown>(resolve=>release=resolve) as T;if(c.kind==='read')return record('new') as T;return empty as T;};
 const c=new ComparisonController(rpc);await c.open('a','AAPL');const old=c.select('old');await c.open('b','MSFT');await c.select('new');release(record('old'));await old;assert.equal(c.getSnapshot().selected?.id,'new');assert(!calls.includes('start'));c.dispose();
});
