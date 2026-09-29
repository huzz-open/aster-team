import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { beforeEach, expect, test, vi } from 'vitest'
import PaidAuthorizationsView from '../src/views/PaidAuthorizationsView.vue'
import type { CommercialOrderRecord, CommercialPlanDefinition } from '../src/api/client'

const api = vi.hoisted(() => ({ list: vi.fn(), get: vi.fn() }))
const navigation = vi.hoisted(() => ({ route: { path: '/workflows/business', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  listCommercialOrders: api.list,
  getCommercialOrder: api.get,
  listCustomers: vi.fn().mockResolvedValue({ items: [], next: '' }),
  listCommercialPlans: vi.fn().mockResolvedValue([]),
}))
vi.mock('vue-router', () => ({
  useRoute: () => navigation.route,
  useRouter: () => navigation,
}))

const definition = fixtureData as CommercialPlanDefinition
function order(id: string, status: CommercialOrderRecord['status'], fulfillmentStatus?: CommercialOrderRecord['fulfillment_status']): CommercialOrderRecord {
  return {
    snapshot: {
      schema: 'aster.order-snapshot.v1', order_id: id, customer_id: `customer_${id}`, plan: { schema: 'aster.plan-snapshot.v1', plan_id: 'paid_20', version: 2, definition }, plan_sha256: 'a'.repeat(64),
      years: 1, discount_basis_points: 10000, amount_minor: 599900, currency: 'CNY', tax_mode: 'inclusive', starts_at: '2026-09-06T00:00:00.000Z', ends_at: '2027-09-06T00:00:00.000Z',
    },
    sha256: 'b'.repeat(64), operation_id: `operation_${id}`, status, created_by: 'operator_1', created_at: '2026-09-06T00:00:00.000Z', customer_name: `客户 ${id}`,
    ...(fulfillmentStatus ? { fulfillment_id: `fulfillment_${id}`, fulfillment_status: fulfillmentStatus } : {}),
  }
}
const paymentStart = vi.fn()
const fulfillmentStart = vi.fn()
const controllerStub = (method: ReturnType<typeof vi.fn>, label: string) => defineComponent({
  setup(_, { expose }) { expose({ startForOrder: method, startSeeded: vi.fn() }); return () => h('button', label) },
})
function render() {
  return mount(PaidAuthorizationsView, { global: { stubs: {
    CommercialPaymentConfirm: controllerStub(paymentStart, '手工到账入口'),
    CommercialFulfillment: controllerStub(fulfillmentStart, '手工授权入口'),
  } } })
}

beforeEach(() => {
  sessionStorage.clear()
  navigation.route.query = {}
  paymentStart.mockReset()
  fulfillmentStart.mockReset()
  navigation.push.mockReset()
  navigation.replace.mockReset()
  api.list.mockReset().mockResolvedValue({ items: [
    order('payment', 'pending_payment'),
    order('approval', 'fulfillment_pending'),
    order('issue', 'fulfillment_pending', 'approved'),
    order('issued', 'fulfilled', 'issued'),
  ], total: 4 })
  api.get.mockReset().mockResolvedValue(order('approval', 'fulfillment_pending'))
})

test('shows the business stage and one direct next action for every order', async () => {
  const wrapper = render()
  await flushPromises()
  expect(wrapper.findAll('tbody tr')).toHaveLength(4)
  expect(wrapper.text()).toContain('待确认到账')
  expect(wrapper.text()).toContain('待导入申请')
  expect(wrapper.text()).toContain('待签发')
  expect(wrapper.text()).toContain('已签发')

  await wrapper.findAll('tbody tr')[0]!.get('button').trigger('click')
  await flushPromises()
  expect(paymentStart).toHaveBeenCalledWith('payment')
  expect(navigation.push).toHaveBeenCalledWith(expect.objectContaining({ query: expect.objectContaining({ order: 'payment', step: '3' }) }))
  await wrapper.findAll('button').find(button => button.text() === '办理列表')!.trigger('click')
  await wrapper.findAll('tbody tr')[1]!.get('button').trigger('click')
  await flushPromises()
  expect(fulfillmentStart).toHaveBeenCalledWith('approval')
  wrapper.unmount()
})

test('an order URL restores the exact workflow record after refresh', async () => {
  navigation.route.query = { order: 'approval', step: '4' }
  const wrapper = render()
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith('approval')
  expect(fulfillmentStart).toHaveBeenCalledWith('approval')
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('导入申请')
  wrapper.unmount()
})

test('reached business nodes can be opened again without changing saved order data', async () => {
  const wrapper = render()
  await flushPromises()
  await wrapper.findAll('tbody tr')[3]!.get('button').trigger('click')
  await flushPromises()
  const nodes = wrapper.findAll('.workflow-stepper li button')
  expect(nodes).toHaveLength(8)
  await nodes[0]!.trigger('click')
  expect(wrapper.text()).toContain('客户 issued')
  expect(navigation.push).toHaveBeenCalledWith(expect.objectContaining({ query: expect.objectContaining({ order: 'issued', step: '1' }) }))
  await nodes[5]!.trigger('click')
  await flushPromises()
  expect(fulfillmentStart).toHaveBeenLastCalledWith('issued')
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('选择证书')
  wrapper.unmount()
})

test('a prepared signing record restores at the signing node', async () => {
  api.get.mockResolvedValue(order('prepared', 'fulfillment_pending', 'prepared'))
  navigation.route.query = { order: 'prepared', step: '7' }
  const wrapper = render()
  await flushPromises()
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('签发授权')
  wrapper.unmount()
})

test('uses server pagination and workflow filters', async () => {
  const wrapper = render()
  await flushPromises()
  expect(api.list).toHaveBeenCalledWith({ limit: 50, offset: 0, keyword: '', stage: 'all' })
  await wrapper.get('input[placeholder="客户、订单编号或客户编号"]').setValue('测试客户')
  await new Promise(resolve => window.setTimeout(resolve, 220))
  await flushPromises()
  expect(api.list).toHaveBeenLastCalledWith({ limit: 50, offset: 0, keyword: '测试客户', stage: 'all' })
  wrapper.unmount()
})
