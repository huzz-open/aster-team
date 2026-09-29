import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { request } from '@aster/sdk'
import { AIconButton, ASegmentedControl, ASelect } from '@aster/ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import ModelsView from '../src/views/ModelsView.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

describe('model availability actions', () => {
  beforeEach(() => {
    vi.mocked(request).mockReset().mockResolvedValue({ items: [
      { id: 'model_enabled', public_name: 'gpt-6-sol', upstream_name: 'gpt-6-sol', display_name: 'GPT-6 Sol', enabled: true, available: true, provider: 'OpenAI', discovered_at: '2026-09-20T00:00:00Z', price_source: 'builtin', price_preview: { currency: 'USD', rate: { kind: 'context_tokens', rates: { input_threshold: 272000, short: { input: 2_000_000_000, output: 10_000_000_000 }, long: { input: 4_000_000_000, output: 15_000_000_000 } } }, has_other_prices: false } },
      { id: 'model_closed', public_name: 'deepseek-reasoner', upstream_name: 'deepseek-reasoner', display_name: 'DeepSeek Reasoner', enabled: false, available: false, provider: 'DeepSeek', discovered_at: '2026-09-20T00:00:00Z' },
      { id: 'model_unavailable', public_name: 'deepseek-code', upstream_name: 'deepseek-code', display_name: 'DeepSeek Code', enabled: true, available: false, provider: 'DeepSeek', discovered_at: '2026-09-20T00:00:00Z', price_source: 'manual' },
    ] })
  })

  it('uses distinct pause and play icons for closing and opening models', async () => {
    const wrapper = mount(ModelsView, { global: { stubs: { Teleport: true } } })
    await flushPromises()

    const pricing = wrapper.findAllComponents(AIconButton).filter(button => button.props('icon') === 'payment')
    expect(pricing).toHaveLength(3)
    const actions = wrapper.findAllComponents(AIconButton).filter(button => button.props('icon') !== 'payment')
    expect(actions).toHaveLength(3)
    expect(actions[0].props()).toMatchObject({ icon: 'pause', label: '停止向成员开放' })
    expect(actions[1].props()).toMatchObject({ icon: 'play', label: '向成员开放' })
    expect(actions[2].props()).toMatchObject({ icon: 'pause', label: '停止向成员开放' })
    expect(wrapper.text()).toContain('无可用连接')
    expect(wrapper.text()).not.toContain('无可用账号')
    expect(wrapper.text()).toContain('预设价格')
    expect(wrapper.text()).toContain('未定价')
    expect(wrapper.text()).toContain('管理员设置')
    expect(wrapper.text()).toContain('短 $2 / $10 · 长 $4 / $15')
    expect(wrapper.findComponent(ASegmentedControl).exists()).toBe(true)
  })

  it('filters models by provider and pricing status', async () => {
    const wrapper = mount(ModelsView, { global: { stubs: { Teleport: true } } })
    await flushPromises()

    const filters = wrapper.findAllComponents(ASelect)
    expect(filters).toHaveLength(3)
    filters[0]!.vm.$emit('update:modelValue', 'DeepSeek')
    await flushPromises()
    expect(wrapper.findAll('tbody tr')).toHaveLength(2)
    filters[2]!.vm.$emit('update:modelValue', 'unpriced')
    await flushPromises()
    expect(wrapper.findAll('tbody tr')).toHaveLength(1)
    expect(wrapper.text()).toContain('deepseek-reasoner')
  })
})
