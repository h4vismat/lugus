import assert from 'node:assert/strict';
import {test} from 'node:test';
import {acceptsPortfolioResult} from './portfolio/state.ts';
import {formatUsd,decimalProduct} from './portfolio/format.ts';
test('late account result cannot replace selected account',()=>{
 const a={portfolioId:'p',accountId:'a',generation:1};assert.equal(acceptsPortfolioResult({...a,accountId:'b',generation:2},a),false);
});
test('money formatting and trade default preserve precision',()=>{
 assert.equal(formatUsd('9007199254740993.015'),'$9,007,199,254,740,993.02');
 assert.equal(formatUsd('-1.005'),'-$1.00');
 assert.equal(decimalProduct('3','0.333333333333333333'),'1.00');
 assert.equal(decimalProduct('0.1','0.2'),'0.02');
});
