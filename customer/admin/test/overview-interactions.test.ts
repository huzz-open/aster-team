import { flushPromises, mount } from '@vue/test-utils'
import { request, type AdminBillingOverview } from '@aster/sdk'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { adminOverview } from '../../demo/src/fixtures'
import OverviewView from '../src/views/OverviewView.vue'

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

const sampleBilling: AdminBillingOverview = {
  configured: true,
  currency: 'CNY',
  balance: '80.00',
  credited: '200.00',
  debited: '120.00',
  billing_errors: 0,
  daily: [],
  models: [],
  members: [{ identity_id: 'member-1', name: '测试成员', email: 'member@example.test', balance: '80.00', credited: '200.00', debited: '120.00' }],
  entries: [],
}

function renderOverview(billing: AdminBillingOverview = sampleBilling) {
  vi.mocked(request).mockImplementation(async (path) => path.startsWith('/api/admin/overview')
    ? structuredClone(adminOverview)
    : structuredClone(billing))
  return mount(OverviewView, { global: { stubs: { ADatePeriodRange: true, Transition: false } } })
}

describe('admin operations and cost overview', () => {
  beforeEach(() => { vi.mocked(request).mockReset() })

  it('retains the established dashboard structure even before billing is configured', async () => {
    const wrapper = renderOverview({ configured: false })
    await flushPromises()

    expect(wrapper.get('h1').text()).toBe('运行概览')
    expect(wrapper.findAll('.kpi-card')).toHaveLength(6)
    expect(wrapper.get('.trend-chart svg').attributes('aria-label')).toBe('Token 使用趋势图')
    expect(wrapper.findAll('.donut-segment')).toHaveLength(4)
    expect(wrapper.get('.resources-panel').text()).toContain('运行资源')
    expect(wrapper.text()).toContain('请先配置费用结算')
    expect(vi.mocked(request).mock.calls.map(([path]) => path)).toEqual([
      '/api/admin/overview?period=14d',
      '/api/admin/billing/overview',
    ])
  })

  it('shows period costs in the old chart layout and allows switching to Token usage', async () => {
    const date = adminOverview.trend[0]!.date
    const billing: AdminBillingOverview = {
      ...sampleBilling,
      entries: [{
        id: 'charge-1', identity_id: 'member-1', member_name: '测试成员', member_email: 'member@example.test',
        kind: 'charge', status: 'settled', amount: '-12.50', reference_id: 'request-1',
        created_at: `${date}T12:00:00Z`, details: { public_model: 'gpt-test' },
      }],
    }
    const wrapper = renderOverview(billing)
    await flushPromises()

    expect(wrapper.get('h1').text()).toBe('运行概览')
    expect(wrapper.get('.kpi-card').text()).toContain('¥12.50')
    expect(wrapper.get('.trend-chart svg').attributes('aria-label')).toBe('费用趋势图')
    expect(wrapper.get('.ranking-list').text()).toContain('测试成员')
    expect(wrapper.get('.model-list').text()).toContain('gpt-test')
    await wrapper.get('.chart-modes button:last-child').trigger('click')
    expect(wrapper.get('.trend-chart svg').attributes('aria-label')).toBe('Token 使用趋势图')
  })
})
