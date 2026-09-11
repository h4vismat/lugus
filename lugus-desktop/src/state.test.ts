import {test} from 'node:test';
import assert from 'node:assert/strict';
import {acceptsResult, chartGeometry, exactNumber, sameCompany} from './state.ts';

test('late workspace or selection responses cannot replace the active research',()=>{
  assert.equal(acceptsResult({id:'b',generation:2},{id:'a',generation:1}),false);
  assert.equal(acceptsResult({id:'a',generation:3},{id:'a',generation:1}),false);
  assert.equal(acceptsResult({id:'a',generation:3},{id:'a',generation:3}),true);
});
test('plotting retains missing-value gaps and never invents a point',()=>{
  const plot=chartGeometry([{date:'2024-01-01',value:'100'},{date:'2024-01-02',value:null},{date:'2024-01-03',value:'110'}]);
  assert.equal(plot.segments.length,2);
  assert.equal(plot.points.length,2);
  assert.equal(plot.points[0].value,'100');
  assert.equal(plot.points[1].x,360);
  assert.equal(chartGeometry([]).points.length,0);
  assert.ok(Number.isFinite(chartGeometry([{date:'2024-01-01',value:'0'}]).points[0].y));
});
test('financial display keeps exact decimals beyond JavaScript safe precision',()=>{
  assert.equal(exactNumber('12345678901234567890.123400'),'12,345,678,901,234,567,890.123400');
  assert.equal(exactNumber('-1234.00'),'-1,234.00');
  assert.equal(exactNumber(null),'Not reported');
});
test('company joins require full explicit identifiers, never ticker/name inference',()=>{
  assert.equal(sameCompany({namespace:'sec:cik',value:'1'},{namespace:'sec:cik',value:'1'}),true);
  assert.equal(sameCompany({namespace:'sec:cik',value:'1'},{namespace:'other',value:'1'}),false);
  assert.equal(sameCompany(undefined,undefined),false);
});
