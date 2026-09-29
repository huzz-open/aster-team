import { flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { ASelect } from '@aster/ui'
import EnvironmentUpgradesView from '../src/views/EnvironmentUpgradesView.vue'

const api = vi.hoisted(() => ({ list: vi.fn(), inspect: vi.fn(), create: vi.fn(), detail: vi.fn() }))
const navigation = vi.hoisted(() => ({ route: { path: '/workflows/release', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', () => ({
  listUpgradeEnvironments: vi.fn(async () => [{ id: 'env_140', name: '140', installation_id: 'installation_140' }]),
  listReleaseArtifacts: vi.fn(async () => [{ id: 'release_1', version: '2.0.2', platform: 'linux', architecture: 'amd64', runtime_linkage: 'musl-static', signature_ref: 'release-key:test', github_run_id: 1, sha256: 'a'.repeat(64) }]),
  listEnvironmentUpgrades: api.list, inspectUpgradeEnvironment: api.inspect, createEnvironmentUpgrade: api.create,
  getEnvironmentUpgrade: api.detail, createUpgradeEnvironment: vi.fn(), rotateUpgradeCredentials: vi.fn(),
}))
beforeEach(() => {
  vi.useFakeTimers()
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  api.list.mockResolvedValue([])
  api.inspect.mockResolvedValue({ current_version: '2.0.1', correlated_upgrades: true, busy: false, upgrade_capabilities: { database_driver: 'sqlcipher', supported_modes: ['maintenance'] } })
  api.create.mockResolvedValue({ id: 'upgrade_1', environment_id: 'env_140', phase: 'queued', upgrade_result: 'pending', continuity_result: 'not_applicable', recovery_result: 'pending' })
  api.detail.mockResolvedValue({ task: { id: 'upgrade_1', environment_id: 'env_140', phase: 'tracking', upgrade_result: 'unknown', continuity_result: 'not_applicable', recovery_result: 'pending', coverage_gap: true }, samples: [] })
  api.create.mockClear()
})
afterEach(() => { vi.useRealTimers() })
function button(wrapper: ReturnType<typeof mount>, text: string) { const found = wrapper.findAll('button').find(item => item.text().includes(text)); if (!found) throw new Error(text); return found }
const options = { global: { stubs: { RouterLink: { template: '<a><slot /></a>' }, Teleport: true } } }

test('upgrade needs target capability and unmount stops only page polling', async () => {
  const wrapper = mount(EnvironmentUpgradesView, options)
  await flushPromises()
  const selects = wrapper.findAllComponents(ASelect)
  selects[0]!.vm.$emit('update:modelValue', 'env_140'); selects[1]!.vm.$emit('update:modelValue', 'release_1')
  await flushPromises()
  expect(button(wrapper, '开始维护升级').attributes('disabled')).toBeDefined()
  await button(wrapper, '检查目标能力').trigger('click'); await flushPromises()
  expect(button(wrapper, '开始维护升级').attributes('disabled')).toBeUndefined()
  await button(wrapper, '开始维护升级').trigger('click'); await flushPromises()
  expect(api.create).toHaveBeenCalledExactlyOnceWith('env_140', 'release_1')
  expect(navigation.push).toHaveBeenCalledWith(expect.objectContaining({ query: expect.objectContaining({ upgrade: 'upgrade_1' }) }))
  const calls = api.list.mock.calls.length
  wrapper.unmount(); await vi.advanceTimersByTimeAsync(10000)
  expect(api.list.mock.calls).toHaveLength(calls)
})

test('an upgrade detail URL restores its report', async () => {
  navigation.route.query = { step: '6', upgrade: 'upgrade_1' }
  api.list.mockResolvedValue([{ id: 'upgrade_1', environment_id: 'env_140', phase: 'tracking' }])
  const wrapper = mount(EnvironmentUpgradesView, options); await flushPromises()
  expect(api.detail).toHaveBeenCalledWith('upgrade_1', 0)
  expect(wrapper.text()).toContain('本次探测过程')
  wrapper.unmount()
})

test('pending target blocks duplicates and report keeps unknown and coverage gap', async () => {
  const task = { id: 'upgrade_1', environment_id: 'env_140', phase: 'tracking', upgrade_result: 'unknown', continuity_result: 'not_applicable', recovery_result: 'pending' }
  api.list.mockResolvedValue([task])
  const wrapper = mount(EnvironmentUpgradesView, options); await flushPromises()
  wrapper.findAllComponents(ASelect)[0]!.vm.$emit('update:modelValue', 'env_140')
  await flushPromises(); await button(wrapper, '查看过程').trigger('click'); await flushPromises()
  expect(wrapper.text()).toContain('状态待确认')
  expect(wrapper.text()).toContain('观察存在重启缺口')
  expect(wrapper.text()).toContain('不适用（维护升级允许中断）')
  expect(button(wrapper, '开始维护升级').attributes('disabled')).toBeDefined()
  expect(api.create).not.toHaveBeenCalled()
  wrapper.unmount()
})
