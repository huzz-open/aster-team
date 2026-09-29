import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { request } from '@aster/sdk'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import UpstreamAccountsView from '../src/views/UpstreamAccountsView.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

describe('subscription and account actions', () => {
  let connections: Array<{
    id: string; provider: string; channel_id: string; display_name: string; billing_mode: string;
    status: string; revision: number; created_at: string; updated_at: string
  }>
  beforeEach(() => {
    connections = [{
      id: 'connection_12345678', provider: 'deepseek', channel_id: 'deepseek.api', display_name: 'DeepSeek',
      billing_mode: 'usage', status: 'active', revision: 1, created_at: '2026-09-18T00:00:00Z', updated_at: '2026-09-20T00:00:00Z',
    }]
    vi.mocked(request).mockReset().mockImplementation(async (url) => {
      if (url === '/api/admin/upstream-accounts') return { items: [{
        id: 'account_12345678', provider: 'openai', email: 'member@example.com', plan: 'Plus', status: 'active',
        credential_count: 1, active_credential_count: 1, created_at: '2026-09-18T00:00:00Z', updated_at: '2026-09-20T00:00:00Z',
      }] }
      if (url === '/api/admin/upstream-providers') return {
        items: [],
        api_key_channels: [{
          id: 'deepseek.api', display_name: 'DeepSeek API', provider: 'deepseek', endpoint_profile: 'deepseek',
          enrollment_kind: 'api_key', billing_mode: 'usage', default_base_url: 'https://api.deepseek.com',
          native_protocols: ['openai'], supports_model_discovery: true,
        }],
      }
      if (url === '/api/admin/upstream-connections') return { items: connections }
      if (url === '/api/admin/plugins/status') return { configured: true, plugins: [] }
      throw new Error(`Unexpected request: ${String(url)}`)
    })
  })

  it('uses compact icon actions with object-specific labels', async () => {
    const wrapper = mount(UpstreamAccountsView, { global: { stubs: { Teleport: true } } })
    await flushPromises()

    const tables = wrapper.findAll('.connection-table')
    expect(tables).toHaveLength(1)
    expect(tables[0].findAll('thead th')).toHaveLength(15)
    expect(tables[0].findAll('tbody tr')).toHaveLength(2)
    for (const row of tables[0].findAll('tbody tr')) expect(row.findAll('td')).toHaveLength(15)

    const subscriptionActions = tables[0].findAll('tbody .row-actions')[0]
    expect(subscriptionActions.findAll('.a-button')).toHaveLength(0)
    expect(subscriptionActions.findAll('.a-icon-button')).toHaveLength(3)
    expect(subscriptionActions.find('button[aria-label="同步订阅账号模型"]').exists()).toBe(true)
    expect(subscriptionActions.find('button[aria-label="停用订阅账号"]').exists()).toBe(true)
    expect(subscriptionActions.find('button[aria-label="删除订阅账号"]').exists()).toBe(true)

    const connectionActions = tables[0].findAll('tbody .row-actions')[1]
    expect(connectionActions.findAll('.a-button')).toHaveLength(0)
    expect(connectionActions.findAll('.a-icon-button')).toHaveLength(5)
    for (const label of ['验证 API Key 连接', '同步连接模型', '为连接添加模型', '停用 API Key 连接', '编辑 API Key 连接']) {
      expect(connectionActions.find(`button[aria-label="${label}"]`).exists()).toBe(true)
    }
    expect(wrapper.findAll('.a-pagination')).toHaveLength(1)
    expect(wrapper.get('.a-pagination-summary').text()).toContain('共 2 条')
  })

  it('paginates both account types together and resets the page when filtering', async () => {
    connections = Array.from({ length: 51 }, (_, index) => ({
      ...connections[0], id: `connection_${String(index).padStart(8, '0')}`, display_name: `DeepSeek ${index}`,
    }))
    const wrapper = mount(UpstreamAccountsView, { global: { stubs: { Teleport: true } } })
    await flushPromises()

    expect(wrapper.findAll('.connection-table tbody tr')).toHaveLength(50)
    expect(wrapper.get('.a-pagination-summary').text()).toContain('共 52 条')
    await wrapper.get('.a-pagination-actions button:last-child').trigger('click')
    expect(wrapper.findAll('.connection-table tbody tr')).toHaveLength(2)

    await wrapper.get('input[aria-label="搜索订阅账号和 API Key 连接"]').setValue('member@example.com')
    expect(wrapper.findAll('.connection-table tbody tr')).toHaveLength(1)
    expect(wrapper.get('.a-pagination-summary').text()).toContain('第 1 / 1 页 · 共 1 条')
  })
})
