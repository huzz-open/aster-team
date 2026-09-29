import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import fixture from '../../../contracts/test-vectors/plan-definition.v1.json'
import FreeDistributionsView from '../src/views/FreeDistributionsView.vue'
import type { CommercialPlanDefinition, FreeDistributionRecord } from '../src/api/client'

const api = vi.hoisted(() => ({ list: vi.fn() }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  listFreeDistributions: api.list,
}))
const record: FreeDistributionRecord = {
  snapshot: {
    schema: 'aster.free-distribution.v1', id: 'dist_test', plan_sha256: 'a'.repeat(64),
    plan: { schema: 'aster.plan-snapshot.v1', plan_id: 'free_test', version: 1, definition: { ...fixture as CommercialPlanDefinition, offer: { kind: 'free', expiry: { mode: 'none' } } } },
    not_before: '2026-09-06T00:00:00.000Z', approved_at: '2026-09-06T00:00:00.000Z', approved_by: 'operator_1', reason: 'view test',
  },
  sha256: 'b'.repeat(64), operation_id: 'approval_test', status: 'approved',
}
beforeEach(() => { sessionStorage.clear(); api.list.mockReset() })
const render = () => mount(FreeDistributionsView, { global: { stubs: { Teleport: true } } })

test('initial read failure has a retry state and never appears as an empty distribution list', async () => {
  api.list.mockRejectedValueOnce(new Error('读取服务不可用')).mockResolvedValueOnce([])
  const wrapper = render(); await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).toContain('免费分发记录未能加载')
  expect(wrapper.text()).toContain('读取服务不可用')
  expect(wrapper.text()).not.toContain('还没有免费分发')
  await wrapper.findAll('button').find(button => button.text() === '重新读取分发列表')!.trigger('click')
  await flushPromises()
  expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  expect(wrapper.text()).toContain('还没有免费分发')
  expect(api.list).toHaveBeenCalledTimes(2)
  wrapper.unmount()
})

test('failed refresh hides stale distribution actions until a successful read', async () => {
  api.list.mockResolvedValueOnce([record]).mockRejectedValueOnce(new Error('读取失败')).mockResolvedValueOnce([record])
  const wrapper = render(); await flushPromises()
  expect(wrapper.get('tbody').text()).toContain('dist_test')
  await wrapper.findAll('button').find(button => button.text() === '刷新')!.trigger('click')
  await flushPromises()
  expect(wrapper.find('tbody').exists()).toBe(false)
  expect(wrapper.text()).not.toContain('还没有免费分发')
  await wrapper.findAll('button').find(button => button.text() === '重新读取分发列表')!.trigger('click')
  await flushPromises()
  expect(wrapper.find('[role="alert"]').exists()).toBe(false)
  expect(wrapper.get('tbody').text()).toContain('dist_test')
  wrapper.unmount()
})
