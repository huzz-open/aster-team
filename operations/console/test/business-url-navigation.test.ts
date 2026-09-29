import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { expect, test, vi } from 'vitest'
import PaidAuthorizationsView from '../src/views/PaidAuthorizationsView.vue'
import type { CommercialOrderRecord, CommercialPlanDefinition } from '../src/api/client'

const record: CommercialOrderRecord = {
  snapshot: {
    schema: 'aster.order-snapshot.v1', order_id: 'order_1', customer_id: 'customer_1',
    plan: { schema: 'aster.plan-snapshot.v1', plan_id: 'plan_1', version: 1, definition: fixtureData as CommercialPlanDefinition },
    plan_sha256: 'a'.repeat(64), years: 1, discount_basis_points: 10000, amount_minor: 599900,
    currency: 'CNY', tax_mode: 'inclusive', starts_at: '2026-09-06T00:00:00.000Z', ends_at: '2027-09-06T00:00:00.000Z',
  },
  sha256: 'b'.repeat(64), operation_id: 'operation_1', status: 'pending_payment',
  created_by: 'operator_1', created_at: '2026-09-06T00:00:00.000Z', customer_name: '测试客户',
}
const api = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn() }))
vi.mock('../src/api/client', async original => ({
  ...await original<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  listCommercialOrders: api.list, getCommercialOrder: api.get,
  listCustomers: vi.fn().mockResolvedValue({ items: [], next: '' }), listCommercialPlans: vi.fn().mockResolvedValue([]),
}))

const paymentStart = vi.fn()
const PaymentStub = defineComponent({ setup(_, { expose }) { expose({ startForOrder: paymentStart }); return () => h('div', '到账视口') } })
const FulfillmentStub = defineComponent({ setup(_, { expose }) { expose({ startForOrder: vi.fn(), startSeeded: vi.fn() }); return () => h('div', '交付视口') } })

test('business record URL reopens the same order and browser back returns to the list', async () => {
  sessionStorage.clear()
  api.list.mockResolvedValue({ items: [record], total: 1 })
  api.get.mockResolvedValue(record)
  paymentStart.mockReset()
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/workflows/business', component: PaidAuthorizationsView }] })
  await router.push('/workflows/business')
  await router.isReady()
  const options = { global: { plugins: [router], stubs: { CommercialPaymentConfirm: PaymentStub, CommercialFulfillment: FulfillmentStub } } }
  const wrapper = mount(PaidAuthorizationsView, options)
  await flushPromises()
  await wrapper.get('tbody button').trigger('click')
  await flushPromises()
  expect(router.currentRoute.value.query).toMatchObject({ order: 'order_1', step: '3' })
  expect(paymentStart).toHaveBeenCalledWith('order_1')
  wrapper.unmount()

  const restored = mount(PaidAuthorizationsView, options)
  await flushPromises()
  expect(restored.find('.workflow-stepper li.active').text()).toContain('确认到账')
  expect(paymentStart).toHaveBeenCalledTimes(2)

  router.back()
  await vi.waitFor(() => expect(router.currentRoute.value.query.order).toBeUndefined())
  await flushPromises()
  expect(restored.findAll('tbody tr')).toHaveLength(1)
  restored.unmount()
})
