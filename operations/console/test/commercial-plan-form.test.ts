import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import pricingData from '../../../contracts/test-vectors/pricing.v1.json'
import { expect, test } from 'vitest'
import type { CommercialPlanDefinition } from '../src/api/client'
import { definitionFromForm, discountBasisPoints, emptyPlanForm, formFromDefinition, formatAmount, termAmount } from '../src/commercial/plan-form'

const fixture = fixtureData as CommercialPlanDefinition

test('opening and revising a plan preserves all fixed rights and terms without aliasing', () => {
  const form = formFromDefinition(fixture)
  expect(definitionFromForm(form)).toEqual(fixture)
  form.quotas[0].value = '3'
  form.features.pop()
  expect(definitionFromForm(form).entitlements.quotas[0].limit).toEqual({ mode: 'limited', value: 3 })
  expect(fixture.entitlements.quotas[0].limit).toEqual({ mode: 'limited', value: 20 })
  expect(fixture.entitlements.features).toHaveLength(3)
})

test('new forms contain no implicit free term, commercial limits, prices or discounts', () => {
  const form = emptyPlanForm()
  expect(form.expiryMode).toBe('')
  expect(form.kind).toBe('')
  expect(form.annualAmountMinor).toBe('')
  expect(form.quotas.every(quota => quota.mode === '' && quota.value === '')).toBe(true)
  expect(form.terms.every(term => !term.enabled && term.discountPercent === '')).toBe(true)
  expect(() => definitionFromForm(form)).toThrow()
})

test('forbidden, finite and unlimited quotas are explicit and malformed numbers cannot be rounded', () => {
  const form = formFromDefinition(fixture)
  form.quotas[0].value = '0'
  expect(definitionFromForm(form).entitlements.quotas[0].limit).toEqual({ mode: 'limited', value: 0 })
  for (const invalid of ['', '-1', '1.5', '1e2', '4294967296', ' 3']) {
    form.quotas[0].value = invalid
    expect(() => definitionFromForm(form)).toThrow()
  }
  form.quotas[0].mode = 'unlimited'
  expect(definitionFromForm(form).entitlements.quotas[0].limit).toEqual({ mode: 'unlimited' })
  form.quotas.pop()
  expect(() => definitionFromForm(form)).toThrow('每人 Key')
})

test('price presentation uses shared server vectors and rounds the final amount once', () => {
  for (const value of pricingData.cases) expect(termAmount(value.annual_amount_minor, value.years, value.discount_basis_points)).toBe(value.amount_minor)
  expect(discountBasisPoints('85.25')).toBe(8525)
  for (const invalid of ['0', '100.01', '9e1', '85.001', '-1']) expect(() => discountBasisPoints(invalid)).toThrow()
  expect(formatAmount(599900, 'CNY')).toContain('5,999')
  expect(formatAmount(500, 'JPY')).toContain('500')
})

test('switching offer kinds strips unrelated fields and requires explicit free expiry', () => {
  const form = formFromDefinition(fixture)
  form.kind = 'contact'
  expect(definitionFromForm(form).offer).toEqual({ kind: 'contact' })
  form.kind = 'free'
  expect(() => definitionFromForm(form)).toThrow('到期方式')
  form.expiryMode = 'none'
  expect(definitionFromForm(form).offer).toEqual({ kind: 'free', expiry: { mode: 'none' } })
  form.expiryMode = 'fixed'; form.expiresAt = '2028-02-29T12:34:56Z'
  const definition = definitionFromForm(form)
  expect(definition.offer).toEqual({ kind: 'free', expiry: { mode: 'fixed', expires_at: '2028-02-29T12:34:56.000Z' } })
  expect(definitionFromForm(formFromDefinition(definition))).toEqual(definition)
  for (const invalid of ['2027-02-29T00:00:00Z', '2027-01-01', '2027-01-01T00:00:00+08:00', '2027-01-01T00:00:00.0001Z']) {
    form.expiresAt = invalid
    expect(() => definitionFromForm(form)).toThrow('UTC')
  }
})

test('symbolic subscription grants survive editing and cannot enter a free plan', () => {
  const definition = structuredClone(fixture)
  definition.entitlements.feature_sets = ['standard']
  definition.entitlements.features = []
  const form = formFromDefinition(definition)
  expect(definitionFromForm(form)).toEqual(definition)
  form.featureSets.push('standard')
  expect(definition.entitlements.feature_sets).toEqual(['standard'])
  form.kind = 'free'
  form.expiryMode = 'none'
  expect(() => definitionFromForm(form)).toThrow('免费套餐只能逐项')
})
