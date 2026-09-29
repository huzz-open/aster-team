import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { request } from '@aster/sdk'
import { ACheckbox, ASelect } from '@aster/ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import UsersView from '../src/views/UsersView.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

describe('member model access controls', () => {
  beforeEach(() => {
    vi.mocked(request).mockReset().mockImplementation(async (url) => {
      if (url === '/api/admin/users') return { items: [{
        id: 'identity_member', email: 'member@example.test', display_name: 'Member', status: 'active',
        password_change_required: false, balance_tokens: 0, granted_tokens: 0, used_tokens: 0,
        request_count: 0, raw_tokens: 0, billed_tokens: 0, created_at: '2026-09-20T00:00:00Z',
      }] }
      if (url === '/api/admin/users/identity_member/model-access') {
        return { mode: 'selected', revision: 0, model_ids: ['model_flash', 'model_pro'] }
      }
      if (url === '/api/admin/models') return { items: [
        { id: 'model_flash', public_name: 'deepseek-flash', display_name: 'DeepSeek Flash', provider: 'deepseek', enabled: true },
        { id: 'model_pro', public_name: 'deepseek-v4-pro', display_name: 'DeepSeek V4 Pro', provider: 'deepseek', enabled: true },
      ] }
      throw new Error(`Unexpected request: ${String(url)}`)
    })
  })

  it('uses the shared select and checkbox controls', async () => {
    const wrapper = mount(UsersView, { global: { stubs: { Teleport: true } } })
    await flushPromises()
    await wrapper.get('button[aria-label="模型权限"]').trigger('click')
    await flushPromises()

    const accessSelect = wrapper.findAllComponents(ASelect).at(-1)
    expect(accessSelect?.props('modelValue')).toBe('selected')
    expect(accessSelect?.props('options')).toEqual([
      { value: 'selected', label: '指定模型' },
      { value: 'all_enabled', label: '所有当前及未来启用的模型' },
    ])

    const modelChoices = wrapper.findAllComponents(ACheckbox)
    expect(modelChoices).toHaveLength(2)
    expect(modelChoices.map(choice => choice.props('modelValue'))).toEqual([
      ['model_flash', 'model_pro'],
      ['model_flash', 'model_pro'],
    ])
  })
})
