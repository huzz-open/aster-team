import { DOMWrapper, enableAutoUnmount, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, it, vi } from 'vitest'
import { nextTick } from 'vue'
import MatrixStatus from '../../../website/docs/.vitepress/theme/MatrixStatus.vue'

enableAutoUnmount(afterEach)
beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  window.getSelection()?.removeAllRanges()
  vi.useRealTimers()
})
const render = () => mount(MatrixStatus, {
  attachTo: document.body,
  props: { status: 'mapped', label: '映射' },
  slots: { default: '<span>max_tokens</span>' },
})
const panel = () => new DOMWrapper(document.querySelector('.matrix-popover')!)
const isOpen = () => document.querySelector('.matrix-popover') !== null
const waitForClose = () => vi.advanceTimersByTimeAsync(200)

it('keeps the panel open while crossing the gap and closes after leaving', async () => {
  const wrapper = render()
  await wrapper.trigger('pointerenter', { pointerType: 'mouse' })
  expect(isOpen()).toBe(true)
  await wrapper.trigger('pointerleave')
  await vi.advanceTimersByTimeAsync(100)
  expect(isOpen()).toBe(true)
  await panel().trigger('mouseenter')
  await waitForClose()
  expect(isOpen()).toBe(true)
  await panel().trigger('mouseleave')
  await waitForClose()
  expect(isOpen()).toBe(false)
})

it('preserves selected data during and after a drag outside, until selection is cleared', async () => {
  const wrapper = render()
  await wrapper.trigger('pointerenter', { pointerType: 'mouse' })
  await panel().trigger('pointerdown')
  await wrapper.trigger('pointerleave')
  await waitForClose()
  expect(isOpen()).toBe(true)
  const range = document.createRange()
  range.selectNodeContents(panel().get('span').element)
  window.getSelection()!.addRange(range)
  document.dispatchEvent(new Event('pointerup'))
  await waitForClose()
  expect(window.getSelection()!.toString()).toBe('max_tokens')
  expect(isOpen()).toBe(true)
  window.getSelection()!.removeAllRanges()
  document.dispatchEvent(new Event('selectionchange'))
  await waitForClose()
  expect(isOpen()).toBe(false)
})

it('supports touch toggling and outside dismissal without sticky hover', async () => {
  const wrapper = render()
  await wrapper.trigger('pointerenter', { pointerType: 'touch' })
  expect(isOpen()).toBe(false)
  await wrapper.get('button').trigger('click')
  await wrapper.trigger('pointerleave')
  await waitForClose()
  expect(wrapper.get('button').attributes('aria-expanded')).toBe('true')
  document.body.dispatchEvent(new Event('pointerdown', { bubbles: true }))
  await nextTick()
  expect(isOpen()).toBe(false)
  await wrapper.get('button').trigger('click')
  await wrapper.get('button').trigger('click')
  expect(isOpen()).toBe(false)
})

it('opens on keyboard focus and lets Escape close the panel without losing focus', async () => {
  const wrapper = render()
  wrapper.get('button').element.focus()
  await nextTick()
  expect(isOpen()).toBe(true)
  document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape' }))
  await nextTick()
  expect(isOpen()).toBe(false)
  expect(document.activeElement).toBe(wrapper.get('button').element)
})
