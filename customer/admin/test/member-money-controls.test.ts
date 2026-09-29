import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { request } from '@aster/sdk'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import UsersView from '../src/views/UsersView.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})
vi.mock('../src/license-status', () => ({ useLicensedFeature: () => true }))

const member = {
  id: 'member-1', email: 'member@example.test', display_name: 'Member', status: 'active',
  password_change_required: false, created_at: '2026-09-20T00:00:00Z',
  money_currency: 'CNY',
  money_balance: '0',
  money_credited: '0',
  money_debited: '0',
}

function render() {
  return mount(UsersView, { global: { stubs: { Teleport: true } } })
}

describe('member monetary controls', () => {
  beforeEach(() => { vi.mocked(request).mockReset() })

  it('shows the default CNY zero balance and reads an empty grant history', async () => {
    vi.mocked(request).mockImplementation(async (url) => {
      if (url === '/api/admin/users') return { items: [{ ...member }] }
      if (url === '/api/admin/users/member-1/money') return { entries: [] }
      throw new Error(`Unexpected request: ${String(url)}`)
    })
    const wrapper = render()
    await flushPromises()

    expect(wrapper.get('tbody tr').text()).toContain('¥0')
    expect(wrapper.get('button[aria-label="禁用成员"]').exists()).toBe(true)
    await wrapper.get('button[aria-label="查看金额发放记录"]').trigger('click')
    await flushPromises()
    expect(wrapper.text()).toContain('暂无金额发放记录')
    expect(vi.mocked(request).mock.calls.some(([url]) => url === '/api/admin/users/member-1/money')).toBe(true)
    expect(vi.mocked(request).mock.calls.some(([url]) => url === '/api/admin/billing/settings')).toBe(false)
  })

  it('grants the entered amount and refreshes the member balance', async () => {
    let balance = '0.00'
    let credited = '0.00'
    vi.mocked(request).mockImplementation(async (url, options) => {
      if (url === '/api/admin/users') return { items: [{ ...member, money_currency: 'CNY', money_balance: balance, money_credited: credited, money_debited: '0.00' }] }
      if (url === '/api/admin/users/member-1/money/grants' && options?.method === 'POST') {
        balance = '12.00'
        credited = '12.00'
        return { ok: true }
      }
      throw new Error(`Unexpected request: ${String(url)}`)
    })
    const wrapper = render()
    await flushPromises()
    await wrapper.get('button[aria-label="发放金额"]').trigger('click')
    await flushPromises()
    expect(wrapper.get('.quota-target').text()).not.toContain('当前余额')
    expect(wrapper.text()).not.toContain('发放金额 (')
    await wrapper.get('input[placeholder="0.00"]').setValue('12')
    await wrapper.get('textarea[minlength="2"]').setValue('测试发放')
    await wrapper.findAll('form').find(form => form.find('.quota-target').exists())!.trigger('submit')
    await flushPromises()

    expect(vi.mocked(request).mock.calls.some(([url, options]) => url === '/api/admin/users/member-1/money/grants' && options?.method === 'POST')).toBe(true)
    expect(wrapper.get('tbody tr').text()).toContain('¥12.00')
  })
})
