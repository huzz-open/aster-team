import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import CommercialPlansView from '../src/views/CommercialPlansView.vue'
import { OperationsAPIError, type CommercialPlanDefinition, type CommercialPlanRecord } from '../src/api/client'

const api = vi.hoisted(() => ({ list: vi.fn(), freeze: vi.fn(), get: vi.fn(), current: vi.fn() }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  listCommercialPlans: api.list, freezeCommercialPlan: api.freeze, getCommercialPlan: api.get, getCurrentCommercialPlan: api.current,
}))
const fixture = fixtureData as CommercialPlanDefinition
function record(version = 1): CommercialPlanRecord {
  return { snapshot: { schema: 'aster.plan-snapshot.v1', plan_id: 'plan_test', version, definition: structuredClone(fixture) }, sha256: 'a'.repeat(64), operation_id: 'test-operation', created_at: '2026-09-01T00:00:00.000Z', created_by: 'operator_test' }
}
function render() { return mount(CommercialPlansView, { global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } } }) }
beforeEach(() => {
  sessionStorage.clear()
  api.list.mockReset().mockResolvedValue([record()])
  api.freeze.mockReset().mockImplementation(async input => ({ ...record(2), operation_id: input.operation_id, snapshot: { ...record(2).snapshot, definition: input.definition } }))
  api.get.mockReset()
  api.current.mockReset().mockResolvedValue(record(2))
})

test('revising sends the displayed version with full rights and preserves the old record', async () => {
  const wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '修订')!.trigger('click')
  expect(wrapper.get('input[aria-label="套餐代码"]').attributes('readonly')).toBeDefined()
  await wrapper.get('input[aria-label="member_seats 数量"]').setValue('30')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.freeze).toHaveBeenCalledOnce()
  expect(api.freeze.mock.calls[0][0]).toMatchObject({ plan_id: 'plan_test', expected_version: 1, definition: { entitlements: { quotas: [{ id: 'member_seats', limit: { mode: 'limited', value: 30 } }, ...fixture.entitlements.quotas.slice(1)] }, offer: fixture.offer } })
  expect(wrapper.text()).toContain('v2')
  expect(fixture.entitlements.quotas[0].limit).toEqual({ mode: 'limited', value: 20 })
  wrapper.unmount()
})

test('ambiguous save retries identical operation and payload after closing and reopening', async () => {
  api.freeze.mockRejectedValueOnce(new TypeError('连接中断')).mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '暂时没有权限', 403))
  let wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '修订')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  const first = JSON.parse(JSON.stringify(api.freeze.mock.calls[0][0]))
  expect(wrapper.get('.commercial-fields').attributes('disabled')).toBeDefined()
  expect(wrapper.get('[role="alert"]').text()).toContain('结果尚未确认')
  await wrapper.findAll('button').find(button => button.text() === '返回')!.trigger('click')
  await wrapper.findAll('button').find(button => button.text() === '继续保存')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.freeze.mock.calls[1][0]).toEqual(first)
  expect(wrapper.get('.commercial-fields').attributes('disabled')).toBeDefined()
  wrapper.unmount(); wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '继续保存')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.freeze.mock.calls[2][0]).toEqual(first)
  expect(wrapper.find('form').exists()).toBe(false)
  expect(sessionStorage.length).toBe(0)
  wrapper.unmount()
})

test('version conflict requires explicit comparison before retaining edits on a newer base', async () => {
  api.freeze.mockRejectedValueOnce(new TypeError('冲突回包丢失')).mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_PLAN_VERSION_CONFLICT', '套餐版本冲突', 409))
  let wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '修订')!.trigger('click')
  await wrapper.get('input[aria-label="套餐名称"]').setValue('保留修改')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  wrapper.unmount(); wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '继续保存')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(wrapper.get('input[aria-label="套餐名称"]').element.value).toBe('保留修改')
  expect(wrapper.get('.commercial-fields').attributes('disabled')).toBeDefined()
  expect(api.list).toHaveBeenCalledTimes(2)
  expect(api.freeze.mock.calls[0][0].expected_version).toBe(1)
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.freeze).toHaveBeenCalledTimes(2)
  await wrapper.findAll('button').find(button => button.text() === '比较最新版本')!.trigger('click'); await flushPromises()
  expect(api.current).toHaveBeenCalledWith('plan_test')
  expect(wrapper.text()).toContain('已保存 v2')
  expect(wrapper.text()).toContain('你的配置 · 保留修改')
  await wrapper.findAll('button').find(button => button.text() === '保留我的配置')!.trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  const [first, , second] = api.freeze.mock.calls.map(call => call[0])
  expect(second.expected_version).toBe(2)
  expect(second.operation_id).not.toBe(first.operation_id)
  expect(second.definition.name).toBe('保留修改')
  wrapper.unmount()
})

test('permission failure is visible and is not presented as an empty catalog', async () => {
  api.list.mockRejectedValue(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '没有套餐读取权限', 403))
  const wrapper = render(); await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).toContain('没有套餐读取权限')
  expect(wrapper.text()).not.toContain('还没有套餐')
  wrapper.unmount()
})

test('history loads the exact prior snapshot and ignores a late response after closing', async () => {
  api.list.mockResolvedValue([record(2)])
  let resolve!: (value: CommercialPlanRecord) => void
  api.get.mockReturnValue(new Promise<CommercialPlanRecord>(done => { resolve = done }))
  const wrapper = render(); await flushPromises()
  await wrapper.findAll('button').find(button => button.text() === '查看版本')!.trigger('click')
  await wrapper.findAll('button').find(button => button.text() === '上一版本')!.trigger('click')
  expect(api.get).toHaveBeenCalledWith('plan_test', 1)
  await wrapper.get('[data-modal-close]').trigger('click')
  resolve(record(1)); await flushPromises()
  expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
  wrapper.unmount()
})
