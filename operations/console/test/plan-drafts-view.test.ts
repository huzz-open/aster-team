import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import PlanDraftsView from '../src/views/PlanDraftsView.vue'
import { OperationsAPIError, type CommercialPlanDefinition, type PlanDraftRecord } from '../src/api/client'

const api = vi.hoisted(() => ({ list: vi.fn(), plans: vi.fn(), save: vi.fn(), get: vi.fn(), current: vi.fn(), freeze: vi.fn() }))
const navigation = vi.hoisted(() => ({ route: { path: '/base/plans', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(), getCurrentOperationsOperatorID: () => 'operator_1',
  listPlanDrafts: api.list, listCommercialPlans: api.plans, savePlanDraft: api.save, getPlanDraft: api.get, getCurrentCommercialPlan: api.current, freezePlanDraft: api.freeze,
}))
const fixture = fixtureData as CommercialPlanDefinition
function record(revision = 1): PlanDraftRecord { return { snapshot: { schema: 'aster.plan-draft.v1', draft_id: 'draft_1', revision, plan_id: 'plan_1', expected_version: 0, definition: structuredClone(fixture) }, sha256: 'a'.repeat(64), operation_id: 'draftop_1', created_by: 'operator_1', created_at: '2026-09-06T00:00:00.000Z' } }
function render() { return mount(PlanDraftsView, { global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } } }) }
function button(wrapper: ReturnType<typeof render>, text: string) { return wrapper.findAll('button').find(item => item.text() === text)! }
beforeEach(() => {
  sessionStorage.clear()
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  api.list.mockReset().mockResolvedValue([record()]); api.plans.mockReset().mockResolvedValue([])
  api.get.mockReset().mockResolvedValue(record()); api.save.mockReset().mockResolvedValue(record(2))
  api.current.mockReset(); api.freeze.mockReset().mockResolvedValue({ snapshot: { plan_id: 'plan_1', version: 1 } })
})

test('a draft URL reopens the exact editor node', async () => {
  navigation.route.query = { draft: 'draft_1', step: '3' }
  const wrapper = render(); await flushPromises()
  expect(wrapper.find('form.plan-editor').exists()).toBe(true)
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('资源额度')
  wrapper.unmount()
})

test('saved draft retries its exact request after response loss and later permission denial across reload', async () => {
  api.save.mockRejectedValueOnce(new TypeError('断线')).mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '临时没有权限', 403))
  let wrapper = render(); await flushPromises()
  await button(wrapper, '编辑草稿').trigger('click')
  await wrapper.get('input[aria-label="套餐名称"]').setValue('保留草稿输入')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  const first = structuredClone(api.save.mock.calls[0][0])
  expect(first).toMatchObject({ draft_id: 'draft_1', expected_revision: 1, expected_version: 0, definition: { name: '保留草稿输入' } })
  expect(wrapper.get('fieldset.draft-fields').attributes('disabled')).toBeDefined()
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.save.mock.calls[1][0]).toEqual(first)
  wrapper.unmount(); wrapper = render(); await flushPromises()
  await button(wrapper, '继续保存').trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.save.mock.calls[2][0]).toEqual(first)
  expect(sessionStorage.length).toBe(0); expect(wrapper.find('form').exists()).toBe(false)
  wrapper.unmount()
})

test('a stale draft requires a visible comparison before saving retained input on the newer revision', async () => {
  api.save.mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_DRAFT_REVISION_CONFLICT', '修订冲突', 409))
  const current = record(2); current.snapshot.definition.name = '其他人的草稿'
  api.get.mockResolvedValue(current)
  const wrapper = render(); await flushPromises()
  await button(wrapper, '编辑草稿').trigger('click')
  await wrapper.get('input[aria-label="套餐名称"]').setValue('我的配置')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.save).toHaveBeenCalledTimes(1)
  await button(wrapper, '比较最新修订').trigger('click'); await flushPromises()
  expect(wrapper.text()).toContain('服务器当前配置'); expect(wrapper.text()).toContain('你的配置')
  await button(wrapper, '保留我的配置').trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.save.mock.calls[1][0]).toMatchObject({ expected_revision: 2, definition: { name: '我的配置' } })
  expect(api.save.mock.calls[1][0].operation_id).not.toBe(api.save.mock.calls[0][0].operation_id)
  wrapper.unmount()
})

test('freeze recovery reads the exact historical draft and does not submit client definition fields', async () => {
  api.freeze.mockRejectedValueOnce(new TypeError('响应丢失'))
  let wrapper = render(); await flushPromises()
  await button(wrapper, '生成版本').trigger('click'); await flushPromises()
  await button(wrapper, '生成固定版本').trigger('click'); await flushPromises()
  const first = structuredClone(api.freeze.mock.calls[0][0])
  expect(Object.keys(first).sort()).toEqual(['draft_id', 'expected_sha256', 'operation_id', 'revision'])
  wrapper.unmount(); wrapper = render(); await flushPromises()
  await button(wrapper, '继续生成版本').trigger('click'); await flushPromises()
  expect(api.get).toHaveBeenLastCalledWith('draft_1', 1)
  await button(wrapper, '重试原请求').trigger('click'); await flushPromises()
  expect(api.freeze.mock.calls[1][0]).toEqual(first)
  expect(sessionStorage.length).toBe(0)
  wrapper.unmount()
})

test('read failure clears stale draft actions while pending recovery stays available', async () => {
  const wrapper = render(); await flushPromises()
  expect(button(wrapper, '编辑草稿')).toBeDefined()
  api.list.mockRejectedValue(new Error('读取失败'))
  await button(wrapper, '刷新').trigger('click'); await flushPromises()
  expect(wrapper.get('[role="alert"]').text()).toContain('读取失败')
  expect(button(wrapper, '编辑草稿')).toBeUndefined()
  expect(wrapper.text()).not.toContain('还没有草稿')
  wrapper.unmount()
})

test('plan base updates only after explicit comparison and persists as another draft revision', async () => {
  api.current.mockResolvedValue({ snapshot: { plan_id: 'plan_1', version: 2, definition: { ...fixture, name: '已保存的套餐' } } })
  const wrapper = render(); await flushPromises()
  await button(wrapper, '编辑草稿').trigger('click')
  await wrapper.get('input[aria-label="套餐名称"]').setValue('仍用我的配置')
  await button(wrapper, '核对套餐当前版本').trigger('click'); await flushPromises()
  expect(api.save).not.toHaveBeenCalled()
  await button(wrapper, '保留我的配置').trigger('click')
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.save.mock.calls[0][0]).toMatchObject({ expected_version: 2, expected_revision: 1, definition: { name: '仍用我的配置' } })
  wrapper.unmount()
})
