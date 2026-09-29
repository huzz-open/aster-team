import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { request, type Model } from '@aster/sdk'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import BillingPriceEditor from '../src/components/BillingPriceEditor.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

const model: Model = {
  id: 'model_1', public_name: 'gpt-6-sol', upstream_name: 'gpt-6-sol',
  display_name: 'GPT-6 Sol', enabled: true,
}

describe('billing price editor', () => {
  beforeEach(() => {
    vi.mocked(request).mockReset().mockResolvedValue({ ok: true }).mockResolvedValueOnce({
      active_version: 'builtin-2026-09-28', sync_supported: true,
      versions: [{
        source: 'builtin', source_url: 'https://developers.openai.com/api/docs/pricing', verified_at: '2026-09-28',
        plan: {
          public_model: model.public_name, version: 'builtin-2026-09-28', currency: 'USD',
          tiers: [{ id: 'standard', schedule: { base: { kind: 'context_tokens', rates: {
            input_threshold: 272000,
            short: { input: 2_000_000_000, cached_read: 200_000_000, cached_write: 2_500_000_000, output: 10_000_000_000, image_input: null, image_output: null },
            long: { input: 4_000_000_000, cached_read: 400_000_000, cached_write: 5_000_000_000, output: 15_000_000_000, image_input: null, image_output: null },
          } }, windows: [] } }],
        },
      }],
    })
  })

  it('shows prices without exposing internal version or tier IDs and creates a version on save', async () => {
    const wrapper = mount(BillingPriceEditor, { props: { model }, global: { stubs: { Teleport: true } } })
    await flushPromises()

    expect(wrapper.text()).toContain('预设价格')
    expect(wrapper.text()).toContain('计价单位')
    expect(wrapper.text()).toContain('长上下文使用不同单价')
    expect(wrapper.text()).toContain('标准处理')
    expect(wrapper.text()).not.toContain('新版本')
    expect(wrapper.text()).not.toContain('档位 ID')

    await wrapper.get('form').trigger('submit')
    await flushPromises()
    const save = vi.mocked(request).mock.calls.find(([path]) => path === '/api/admin/billing/prices')
    expect(save).toBeDefined()
    const plan = JSON.parse(String(save![1]?.body)).plan
    expect(plan.version).toMatch(/^manual-/)
    expect(plan.tiers[0].id).toBe('standard')
  })
})
