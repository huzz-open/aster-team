import assert from 'node:assert/strict'
import test from 'node:test'

import { formatNaturalTokenAmount } from '../src/format.ts'
import { formatMoney } from '../src/index.ts'

test('Chinese token amounts use 万 and 亿 instead of translated western units', () => {
  assert.equal(formatNaturalTokenAmount(10_000_000, 'zh-CN'), '1,000 万 Token')
  assert.equal(formatNaturalTokenAmount(100_000_000, 'zh-CN'), '1 亿 Token')
})

test('English token amounts use localized western units', () => {
  assert.equal(formatNaturalTokenAmount(10_000_000, 'en-US'), '10 million tokens')
  assert.equal(formatNaturalTokenAmount(1_500_000_000, 'en-US'), '1.5 billion tokens')
})

test('balances use two decimals and charges use four without floating-point drift', () => {
  assert.equal(formatMoney('99.9971525', 'CNY', 2), '¥100.00')
  assert.equal(formatMoney('0.0028475', 'CNY'), '¥0.0028')
  assert.equal(formatMoney('0.00285', 'CNY'), '¥0.0029')
  assert.equal(formatMoney('-0.005', 'USD', 2), '-$0.01')
})
