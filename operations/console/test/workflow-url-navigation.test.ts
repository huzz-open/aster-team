import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { createMemoryHistory, createRouter } from 'vue-router'
import { expect, test, vi } from 'vitest'
import { ASelect } from '@aster/ui'
import ReleaseWorkflowView from '../src/views/ReleaseWorkflowView.vue'

vi.mock('../src/api/client', async original => ({
  ...await original<typeof import('../src/api/client')>(),
  listCommercialPlans: vi.fn().mockResolvedValue([{
    snapshot: { plan_id: 'paid_20', version: 2, definition: { name: '企业版', code: 'enterprise', edition: 'team' } },
    sha256: 'a'.repeat(64),
  }]),
}))

const Placeholder = defineComponent({ render: () => h('div') })

test('release node URL survives refresh-style mounting and browser history navigation', async () => {
  const router = createRouter({ history: createMemoryHistory(), routes: [{ path: '/workflows/release', component: ReleaseWorkflowView }] })
  await router.push('/workflows/release')
  await router.isReady()
  const options = { global: { plugins: [router], stubs: {
    PublicCatalogsView: Placeholder, PublicationsView: Placeholder, ReleaseCenterView: Placeholder,
    ReleaseArtifactsView: Placeholder, EnvironmentUpgradesView: Placeholder,
  } } }
  const wrapper = mount(ReleaseWorkflowView, options)
  await flushPromises()
  wrapper.findComponent(ASelect).vm.$emit('update:modelValue', 'paid_20')
  await flushPromises()
  await wrapper.findAll('.workflow-actions button')[1]!.trigger('click')
  await flushPromises()
  expect(router.currentRoute.value.query).toMatchObject({ step: '2', plan: 'paid_20' })
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('批准目录')

  router.back()
  await vi.waitFor(() => expect(router.currentRoute.value.query.step).toBe('1'))
  await flushPromises()
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('选择版本')
  wrapper.unmount()

  const restored = mount(ReleaseWorkflowView, options)
  await flushPromises()
  expect(restored.find('.workflow-stepper li.active').text()).toContain('选择版本')
  restored.unmount()
})
