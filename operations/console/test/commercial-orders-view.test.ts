import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { ASelect } from '@aster/ui'
import { beforeEach, expect, test, vi } from 'vitest'
import CommercialOrdersView from '../src/views/CommercialOrdersView.vue'
import { OperationsAPIError, type CommercialPlanDefinition, type CommercialPlanRecord, type CreateCommercialOrderInput } from '../src/api/client'
import { orderInput } from '../src/commercial/order-form'

const api = vi.hoisted(() => ({ list: vi.fn(), create: vi.fn(), get: vi.fn(), plans: vi.fn(), customers: vi.fn() }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  listCommercialOrders: api.list, createCommercialOrder: api.create, getCommercialOrder: api.get, listCommercialPlans: api.plans, listCustomers: api.customers,
}))
const definition = fixtureData as CommercialPlanDefinition
function plan(): CommercialPlanRecord { return { snapshot: { schema: 'aster.plan-snapshot.v1', plan_id: 'plan_1', version: 2, definition: structuredClone(definition) }, sha256: 'a'.repeat(64), operation_id: 'plan_op', created_at: '2026-09-01T00:00:00.000Z', created_by: 'operator_1' } }
function render() { return mount(CommercialOrdersView, { global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } } }) }
async function fill(wrapper: ReturnType<typeof render>) {
  await wrapper.findAll('button').find(button => button.text() === '新增订单')!.trigger('click'); await flushPromises()
  for (const [label, value] of [['客户', 'customer_1'], ['套餐版本', 'plan_1'], ['订阅期限', '3']]) {
    const select = wrapper.findAllComponents(ASelect).find(item => item.props('ariaLabel') === label)!
    select.vm.$emit('update:modelValue', value); select.vm.$emit('change', value); await flushPromises()
  }
  await wrapper.get('input[type="datetime-local"]').setValue('2028-02-29T16:30:00')
}
beforeEach(() => {
  sessionStorage.clear()
  api.list.mockReset().mockResolvedValue({ items: [], total: 0 })
  api.plans.mockReset().mockResolvedValue([plan(), { ...plan(), snapshot: { ...plan().snapshot, plan_id: 'free_1', definition: { ...definition, offer: { kind: 'free', expiry: { mode: 'none' } } } } }])
  api.customers.mockReset().mockResolvedValue({ items: [{ id: 'customer_1', name: '测试客户', legal_name: '测试主体', status: 'active' }], next: '' })
  api.create.mockReset().mockImplementation(async (input: CreateCommercialOrderInput) => ({
    snapshot: { schema: 'aster.order-snapshot.v1', order_id: 'order_1', customer_id: input.customer_id, plan: plan().snapshot, plan_sha256: plan().sha256, years: input.years, discount_basis_points: 8500, amount_minor: 1529745, currency: 'CNY', tax_mode: 'inclusive', starts_at: input.starts_at, ends_at: '2031-02-28T16:30:00.000Z' },
    sha256: 'b'.repeat(64), operation_id: input.operation_id, status: 'pending_payment', created_at: '2026-09-06T00:00:00.000Z', created_by: 'operator_1',
  }))
  api.get.mockReset()
})

test('order input fixes the exact selected version and UTC start without accepting client prices', () => {
  const value = orderInput('customer_1', plan(), 3, '2028-02-29T16:30')
  expect(value).toMatchObject({ plan_id: 'plan_1', plan_version: 2, years: 3, starts_at: '2028-02-29T16:30:00.000Z' })
  expect(Object.keys(value).sort()).toEqual(['customer_id', 'operation_id', 'plan_id', 'plan_version', 'starts_at', 'years'])
  for (const date of ['2027-02-29T00:00', '2028-01-01', '2028-01-01T24:00', '']) expect(() => orderInput('customer_1', plan(), 3, date)).toThrow()
  expect(() => orderInput('customer_1', plan(), 6, '2028-01-01T00:00')).toThrow('订阅期限')
})

test('order list does not depend on plan or customer read access', async () => {
  const wrapper = render(); await flushPromises()
  expect(api.list).toHaveBeenCalledOnce()
  expect(api.plans).not.toHaveBeenCalled()
  expect(api.customers).not.toHaveBeenCalled()
  expect(wrapper.text()).toContain('还没有权益订单')
  wrapper.unmount()
})

test('create uses annual plans only and shows server-fixed order rights, price and dates', async () => {
  const wrapper = render(); await flushPromises(); await fill(wrapper)
  const choices = wrapper.findAllComponents(ASelect).find(item => item.props('ariaLabel') === '套餐版本')!.props('options')
  expect(choices).toHaveLength(1)
  expect(wrapper.text()).toContain('15,297.45')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.create.mock.calls[0][0]).toMatchObject({ plan_id: 'plan_1', plan_version: 2, years: 3, starts_at: '2028-02-29T16:30:00.000Z' })
  expect(api.create.mock.calls[0][0]).not.toHaveProperty('amount_minor')
  expect(wrapper.text()).toContain('2031-02-28T16:30:00.000Z')
  expect(wrapper.text()).toContain('订单权益快照')
  wrapper.unmount()
})

test('ambiguous create preserves operation and start across modal close and retry', async () => {
  api.create.mockRejectedValueOnce(new TypeError('响应丢失')).mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '暂时没有权限', 403))
  let wrapper = render(); await flushPromises(); await fill(wrapper)
  await wrapper.get('form').trigger('submit'); await flushPromises()
  const first = JSON.parse(JSON.stringify(api.create.mock.calls[0][0]))
  expect(wrapper.get('.commercial-fields').attributes('disabled')).toBeDefined()
  await wrapper.findAll('button').find(button => button.text() === '返回')!.trigger('click')
  await wrapper.findAll('button').find(button => button.text() === '继续创建')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.create.mock.calls[1][0]).toEqual(first)
  expect(wrapper.get('.commercial-fields').attributes('disabled')).toBeDefined()
  wrapper.unmount(); wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '继续创建')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.create.mock.calls[2][0]).toEqual(first)
  expect(api.plans).toHaveBeenCalledOnce()
  expect(sessionStorage.length).toBe(0)
  wrapper.unmount()
})

test('plan lookup denial stays inside the create dialog and preserves order browsing', async () => {
  api.plans.mockRejectedValue(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '缺少套餐读取权限', 403))
  const wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '新增订单')!.trigger('click'); await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).toContain('缺少套餐读取权限')
  expect(wrapper.findAll('button').find(button => button.text() === '创建订单')!.attributes('disabled')).toBeDefined()
  expect(wrapper.text()).toContain('还没有权益订单')
  expect(api.create).not.toHaveBeenCalled()
  wrapper.unmount()
})
