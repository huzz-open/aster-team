import { mount } from '@vue/test-utils'
import { nextTick } from 'vue'
import { afterEach, describe, expect, it, vi } from 'vitest'
import { AButton, ACheckbox, AFilePicker, AIconButton, AInfoTip, AModal, APagination, ASegmentedControl, ASelect } from '@aster/ui'

const options = [
  { value: 'one', label: '第一个' },
  { value: 'two', label: '第二个', description: '补充说明' },
  { value: 'disabled', label: '不可选择', disabled: true },
]

describe('shared production interaction components', () => {
  afterEach(() => vi.useRealTimers())

  it('keeps a wide platform menu inside the viewport near its right edge', async () => {
    const wrapper = mount(ASelect, { props: { options, popupMinWidth: 260 }, global: { stubs: { Teleport: true } } })
    const trigger = wrapper.get('[role="combobox"]')
    vi.spyOn(trigger.element, 'getBoundingClientRect').mockReturnValue({ left: window.innerWidth - 120, right: window.innerWidth - 20, width: 100, top: 100, bottom: 140, height: 40 } as DOMRect)
    await trigger.trigger('click')
    const popup = wrapper.get('.a-select-popup').element as HTMLElement
    expect(parseFloat(popup.style.left) + parseFloat(popup.style.width)).toBeLessThanOrEqual(window.innerWidth - 10)
    expect(parseFloat(popup.style.width)).toBe(260)
    wrapper.unmount()
  })

  it('supports pointer and keyboard selection without a native select', async () => {
    const wrapper = mount(ASelect, {
      props: { modelValue: 'one', options, ariaLabel: '测试选项' },
      global: { stubs: { Teleport: true } },
    })

    expect(wrapper.find('select').exists()).toBe(false)
    const trigger = wrapper.get('[role="combobox"]')
    await trigger.trigger('click')
    expect(trigger.attributes('aria-expanded')).toBe('true')
    await trigger.trigger('keydown', { key: 'ArrowDown' })
    await trigger.trigger('keydown', { key: 'Enter' })
    expect(wrapper.emitted('update:modelValue')).toEqual([['two']])
    expect(wrapper.emitted('change')).toEqual([['two']])
  })

  it('filters searchable choices and closes with Escape', async () => {
    const wrapper = mount(ASelect, {
      props: { modelValue: '', options, searchable: true, ariaLabel: '搜索选项' },
      global: { stubs: { Teleport: true } },
    })

    await wrapper.get('[role="combobox"]').trigger('click')
    await wrapper.get('[role="searchbox"]').setValue('第二')
    expect(wrapper.findAll('[role="option"]')).toHaveLength(1)
    expect(wrapper.get('[role="option"]').text()).toContain('第二个')
    await wrapper.get('[role="searchbox"]').trigger('keydown', { key: 'Escape' })
    expect(wrapper.get('[role="combobox"]').attributes('aria-expanded')).toBe('false')
  })

  it('marks required controls invalid and opens only the first missing select', async () => {
    const wrapper = mount({
      components: { ASelect },
      template: '<form><ASelect :options="options" required aria-label="第一项"/><ASelect :options="options" required aria-label="第二项"/></form>',
      setup: () => ({ options }),
    }, { global: { stubs: { Teleport: true } } })

    const proxies = wrapper.findAll('.a-select-proxy')
    proxies[0]!.element.dispatchEvent(new Event('invalid', { cancelable: true }))
    proxies[1]!.element.dispatchEvent(new Event('invalid', { cancelable: true }))
    await nextTick()
    const triggers = wrapper.findAll('[role="combobox"]')
    expect(triggers[0]!.attributes()).toMatchObject({ 'aria-expanded': 'true', 'aria-invalid': 'true' })
    expect(triggers[1]!.attributes()).toMatchObject({ 'aria-expanded': 'false', 'aria-invalid': 'true' })
  })

  it('shows semantic icon help on focus and suppresses a touch long-press click', async () => {
    vi.useFakeTimers()
    const wrapper = mount(AIconButton, {
      props: { icon: 'copy', label: '复制地址' },
      global: { stubs: { Teleport: true } },
    })

    const button = wrapper.get('button')
    await button.trigger('focus')
    await vi.runOnlyPendingTimersAsync()
    expect(wrapper.text()).toContain('复制地址')

    await button.trigger('pointerdown', { pointerType: 'touch', button: 0 })
    await vi.advanceTimersByTimeAsync(500)
    expect(button.classes()).toContain('is-long-press')
    await button.trigger('pointerup', { pointerType: 'touch', button: 0 })
    await button.trigger('click')
    expect(wrapper.emitted('click')).toBeUndefined()
  })

  it('teleports information tips outside clipping containers', async () => {
    const wrapper = mount(AInfoTip, { attachTo: document.body, props: { text: '不会被表格裁剪' } })
    await wrapper.get('.a-info-tip').trigger('mouseenter')
    await nextTick()
    const popover = document.body.querySelector('.a-floating-panel')
    expect(popover?.textContent).toBe('不会被表格裁剪')
    expect(wrapper.find('.a-floating-panel').exists()).toBe(false)
    await wrapper.get('.a-info-tip').trigger('mouseleave')
    await nextTick()
    expect(document.body.querySelector('.a-floating-panel')).toBeNull()
    wrapper.unmount()
  })

  it('keeps an interactive information card open while the pointer is over it', async () => {
    vi.useFakeTimers()
    const wrapper = mount(AInfoTip, {
      attachTo: document.body,
      props: { text: '模型命名规则', interactive: true, tone: 'surface' },
      slots: { default: '<button type="button">模型变体示例</button>' },
    })

    await wrapper.get('.a-info-tip').trigger('mouseenter')
    await nextTick()
    const panel = document.body.querySelector('.a-floating-panel') as HTMLElement
    expect(panel?.textContent).toContain('模型变体示例')

    await wrapper.get('.a-info-tip').trigger('mouseleave')
    panel.dispatchEvent(new MouseEvent('mouseenter'))
    await vi.advanceTimersByTimeAsync(150)
    expect(document.body.querySelector('.a-floating-panel')).toBe(panel)

    panel.dispatchEvent(new MouseEvent('mouseleave'))
    await nextTick()
    expect(document.body.querySelector('.a-floating-panel')).toBeNull()
    wrapper.unmount()
  })

  it('keeps checkbox pointer, keyboard, boolean, and multi-select semantics native', async () => {
    const booleanWrapper = mount(ACheckbox, { props: { modelValue: false, label: '启用网关' } })
    await booleanWrapper.get('input').setValue(true)
    expect(booleanWrapper.emitted('update:modelValue')).toEqual([[true]])

    const arrayWrapper = mount(ACheckbox, { props: { modelValue: ['first'], value: 'second', label: '第二位成员' } })
    await arrayWrapper.get('input').setValue(true)
    expect(arrayWrapper.emitted('update:modelValue')).toEqual([[['first', 'second']]])
    expect(arrayWrapper.get('input').attributes('type')).toBe('checkbox')
  })

  it('opens a semantic file chooser, reports the selected file, and supports clearing', async () => {
    const wrapper = mount(AFilePicker, {
      props: { label: '选择授权文件', accept: 'application/json,.json', required: true },
      global: { stubs: { Teleport: true } },
    })
    const input = wrapper.get('input[type="file"]')
    const file = new File(['{}'], 'license.json', { type: 'application/json' })
    Object.defineProperty(input.element, 'files', { configurable: true, value: [file] })
    await input.trigger('change')
    expect(wrapper.emitted('select')?.[0]?.[0]).toBe(file)
    expect(wrapper.text()).toContain('license.json')
    await wrapper.get('.a-file-picker-clear').trigger('click')
    expect(wrapper.emitted('update:modelValue')?.at(-1)).toEqual([null])
  })

  it('focuses a missing required file picker instead of exposing an inaccessible input', async () => {
    const wrapper = mount(AFilePicker, { attachTo: document.body, props: { label: '选择离线请求', required: true } })
    wrapper.get('.a-file-picker-proxy').element.dispatchEvent(new Event('invalid', { cancelable: true }))
    await nextTick()
    const action = wrapper.get('.a-file-picker-action')
    expect(action.attributes('aria-invalid')).toBe('true')
    expect(document.activeElement).toBe(action.element)
    wrapper.unmount()
  })

  it('accepts a matching dropped file and explains a rejected type', async () => {
    const wrapper = mount(AFilePicker, { props: { label: '导入请求', accept: '.json' } })
    const badFile = new File(['plain'], 'request.txt', { type: 'text/plain' })
    await wrapper.trigger('drop', { dataTransfer: { files: [badFile] } })
    expect(wrapper.text()).toContain('文件类型不受支持')
    expect(wrapper.get('.a-file-picker-action').attributes('aria-invalid')).toBe('true')

    const goodFile = new File(['{}'], 'request.json', { type: 'application/json' })
    await wrapper.trigger('drop', { dataTransfer: { files: [goodFile] } })
    expect(wrapper.emitted('select')?.at(-1)).toEqual([goodFile])
    expect(wrapper.text()).toContain('request.json')
  })

  it('uses roving focus and arrow keys for segmented choices', async () => {
    const wrapper = mount(ASegmentedControl, {
      attachTo: document.body,
      props: { modelValue: 'one', options, label: '时间范围' },
    })
    const controls = wrapper.findAll('[role="radio"]')
    expect(controls.map(item => item.attributes('tabindex'))).toEqual(['0', '-1', '-1'])
    await controls[0]!.trigger('keydown', { key: 'ArrowRight' })
    await nextTick()
    expect(wrapper.emitted('update:modelValue')).toEqual([['two']])
    expect(document.activeElement).toBe(controls[1]!.element)
    wrapper.unmount()
  })

  it('shares accessible bounded and cursor pagination behavior', async () => {
    const bounded = mount(APagination, { props: { page: 2, pageSize: 50, total: 121, locale: 'zh-CN' } })
    expect(bounded.get('nav').attributes('aria-label')).toBe('分页导航')
    expect(bounded.text()).toContain('第 2 / 3 页 · 共 121 条')
    expect(bounded.get('.a-select-trigger').attributes('aria-label')).toBe('每页条数')
    expect(bounded.get('.a-pagination-size').text()).toBe('每页50条')
    expect(bounded.getComponent(ASelect).props('options')).toEqual([
      { value: 20, label: '20' },
      { value: 50, label: '50' },
      { value: 100, label: '100' },
    ])
    expect(bounded.getComponent(ASelect).props('popupMinWidth')).toBe(50)
    expect(bounded.getComponent(ASelect).props('align')).toBe('center')
    bounded.getComponent(ASelect).vm.$emit('update:modelValue', 100)
    await nextTick()
    expect(bounded.emitted('update:pageSize')).toEqual([[100]])
    expect(bounded.emitted('update:page')).toEqual([[1]])
    await bounded.findAll('.a-pagination-actions button')[1]!.trigger('click')
    expect(bounded.emitted('update:page')).toEqual([[1], [3]])

    const cursor = mount(APagination, { props: { page: 1, hasNext: true, locale: 'en-US' } })
    expect(cursor.text()).toContain('Page 1')
    expect(cursor.findAll('button')[0]!.attributes()).toHaveProperty('disabled')
    await cursor.findAll('button')[1]!.trigger('click')
    expect(cursor.emitted('change')).toEqual([[2]])
  })

  it('keeps modal focus inside, ignores backdrop clicks, closes with Escape, and exposes loading state', async () => {
    const wrapper = mount({
      components: { AButton, AModal },
      template: '<AModal :open="true" title="生产确认" @close="closed=true"><input autofocus aria-label="确认字段"><AButton :loading="true">提交</AButton></AModal>',
      data: () => ({ closed: false }),
    }, { attachTo: document.body, global: { stubs: { Teleport: true } } })
    await nextTick()
    expect((document.activeElement as HTMLElement)?.getAttribute('aria-label')).toBe('确认字段')
    const loadingButton = wrapper.get('.a-button')
    expect(loadingButton.attributes()).toMatchObject({ disabled: '', 'aria-busy': 'true' })
    await wrapper.get('.modal-backdrop').trigger('click')
    expect((wrapper.vm as unknown as { closed: boolean }).closed).toBe(false)
    await wrapper.get('[role="dialog"]').trigger('keydown', { key: 'Escape' })
    expect((wrapper.vm as unknown as { closed: boolean }).closed).toBe(true)
    wrapper.unmount()
  })

  it('focuses a modal heading by default without opening an icon-button tooltip', async () => {
    const wrapper = mount({
      components: { AIconButton, AModal },
      template: '<AModal :open="true" title="找回账号" compact-header><AIconButton icon="copy" label="复制" /></AModal>',
    }, { attachTo: document.body, global: { stubs: { Teleport: true } } })
    await nextTick()
    expect(document.activeElement).toBe(wrapper.get('.modal-head h2').element)
    expect(wrapper.get('.modal-card').classes()).toContain('is-compact-header')
    expect(wrapper.find('[role="tooltip"]').exists()).toBe(false)
    await wrapper.get('.modal-head h2').trigger('keydown', { key: 'Tab', shiftKey: true })
    expect((document.activeElement as HTMLElement).getAttribute('aria-label')).toBe('复制')
    wrapper.unmount()
  })
})
