import { mount } from '@vue/test-utils'
import { expect, test } from 'vitest'
import WorkflowStepper from '../src/components/WorkflowStepper.vue'

test('renders workflow progress and emits an explicitly selected step', async () => {
  const wrapper = mount(WorkflowStepper, {
    props: { steps: ['选择版本', '批准目录', '公开发布'], current: 2, interactive: true },
  })

  expect(wrapper.findAll('li')[0]!.classes()).toContain('complete')
  expect(wrapper.findAll('li')[1]!.classes()).toContain('active')
  await wrapper.findAll('button')[2]!.trigger('click')
  expect(wrapper.emitted('select')).toEqual([[3]])
})
