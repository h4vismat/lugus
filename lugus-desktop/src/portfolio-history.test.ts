import assert from 'node:assert/strict';
import {test} from 'node:test';
import {historyRange,acceptsHistory} from './portfolio/history-state.ts';
import {sampleHistory,historyGeometry} from './portfolio/history-geometry.ts';
import type {PerformancePoint} from './portfolio/types.ts';
const point=(day:number,value:string|null):PerformancePoint=>({date:`2026-01-${String(day).padStart(2,'0')}`,value,deposits:'0',withdrawals:'0',opening_contribution:'0',portfolio_growth:null,portfolio_return_percent:value,segment_return_percent:value,segment:0,benchmark_return_percent:'0',hypothetical_value:'100',issues:[]});
test('period ranges clamp month ends and stale selections are rejected',()=>{
 assert.deepEqual(historyRange('1M','2026-03-31'),{start:'2026-02-28',end:'2026-03-31'});
 assert.deepEqual(historyRange('YTD','2026-09-15'),{start:'2026-01-01',end:'2026-09-15'});
 const selected={portfolioId:'p',accountId:null,revision:'1',generation:1,period:'YTD' as const,resultId:null};
 assert.equal(acceptsHistory(selected,{...selected,generation:0}),false);
 assert.equal(acceptsHistory(selected,{...selected,accountId:'a'}),false);
});
test('sampling retains gaps, extreme values, flow dates and recapitalization boundaries',()=>{
 const rows=Array.from({length:25},(_,i)=>point(i+1,String(i)));
 rows[5].portfolio_return_percent=null;rows[5].segment_return_percent=null;rows[5].value=null;rows[12].deposits='10';rows[15].portfolio_return_percent='900';rows[20].segment=1;
 const sampled=sampleHistory(rows,6);for(const index of [0,4,5,6,12,15,19,20,24])assert.ok(sampled.includes(rows[index]));
 const geometry=historyGeometry(rows,'return');assert.ok(geometry.portfolio.length>=3);assert.equal(geometry.points[5].portfolioY,null);
});
