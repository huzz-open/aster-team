import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { dismissToast, showToast } from '@aster/ui'
import { readdirSync, readFileSync } from 'node:fs'
import { join } from 'node:path'
import App from '../src/App.vue'

const createdToasts: number[] = []

afterEach(() => {
  for (const id of createdToasts.splice(0)) dismissToast(id)
  vi.useRealTimers()
})

describe('operations top toast feedback', () => {
  it('mounts globally and renders semantic icons for all feedback types', async () => {
    vi.useFakeTimers()
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, SessionExpiredDialog: true, Teleport: true } },
    })
    createdToasts.push(
      showToast('操作成功', 'success', 5_000),
      showToast('操作提醒', 'info', 5_000),
      showToast('操作告警', 'warning', 5_000),
      showToast('操作失败', 'error', 5_000),
    )
    await nextTick()

    expect(wrapper.text()).toContain('操作成功')
    expect(wrapper.find('.toast-item.success .a-icon').exists()).toBe(true)
    expect(wrapper.find('.toast-item.info .a-icon').exists()).toBe(true)
    expect(wrapper.find('.toast-item.warning .a-icon').exists()).toBe(true)
    expect(wrapper.find('.toast-item.error .a-icon').exists()).toBe(true)
    expect(wrapper.get('.toast-item.error').attributes('role')).toBe('alert')
    wrapper.unmount()
  })

  it('dismisses after two seconds by default and accepts a per-message duration', async () => {
    vi.useFakeTimers()
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, SessionExpiredDialog: true, Teleport: true } },
    })
    createdToasts.push(showToast('默认时长', 'success'), showToast('延长时长', 'warning', 3_500))
    await nextTick()

    await vi.advanceTimersByTimeAsync(1_999)
    expect(wrapper.text()).toContain('默认时长')
    await vi.advanceTimersByTimeAsync(1)
    expect(wrapper.text()).not.toContain('默认时长')
    expect(wrapper.text()).toContain('延长时长')
    await vi.advanceTimersByTimeAsync(1_500)
    expect(wrapper.text()).not.toContain('延长时长')
    wrapper.unmount()
  })

  it('does not render blank errors used by the session-expiry redirect flow', async () => {
    const wrapper = mount(App, {
      global: { stubs: { RouterView: true, SessionExpiredDialog: true, Teleport: true } },
    })
    expect(showToast('', 'error')).toBe(0)
    await nextTick()
    expect(wrapper.find('.toast-item').exists()).toBe(false)
    wrapper.unmount()
  })

  it('keeps operation results out of page-level inline message regions', () => {
    const viewDirectory = join(process.cwd(), 'src', 'views')
    for (const file of readdirSync(viewDirectory).filter(value => value.endsWith('.vue'))) {
      const source = readFileSync(join(viewDirectory, file), 'utf8')
      expect(source, file).not.toMatch(/(?:error|message)\.value\s*=/)
      expect(source, file).not.toContain('notice danger-text page-message')
      expect(source, file).not.toMatch(/window\.(?:alert|confirm)\s*\(/)
    }
  })
})
