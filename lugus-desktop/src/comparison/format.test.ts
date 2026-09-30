import test from 'node:test';import assert from 'node:assert/strict';import {formatReported,formatCalculated} from './format.ts';
test('reported decimals keep exact precision beyond safe integers',()=>{assert.equal(formatReported('9007199254740993'),'9,007,199,254,740,993');assert.equal(formatReported('-1234.001'),'-1,234.001');});
test('calculated display preserves backend rounding and absence',()=>{assert.equal(formatCalculated('3.12'),'3.12%');assert.equal(formatCalculated('9.38'),'9.38%');assert.equal(formatCalculated(null),'—');});
