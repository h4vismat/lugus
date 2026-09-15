import assert from 'node:assert/strict';
import {test} from 'node:test';
import {compareDecimal,formatPercent} from './portfolio/format.ts';
test('decimal ordering is exact and null is always last',()=>{
 assert.equal(compareDecimal('9007199254740993.01','9007199254740993.02','asc'),-1);
 assert.equal(compareDecimal(null,'1','desc'),1);
 assert.equal(compareDecimal('-1','0','desc'),1);
 assert.equal(formatPercent(null),'—');
 assert.equal(formatPercent('12.605',true),'+12.60%');
 assert.equal(formatPercent('-0.001',true),'0.00%');
});
