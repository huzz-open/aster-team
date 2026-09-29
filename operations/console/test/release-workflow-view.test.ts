import { flushPromises, mount } from '@vue/test-utils'
import { defineComponent, h } from 'vue'
import { expect, test, vi } from 'vitest'
import { ASelect } from '@aster/ui'
import ReleaseWorkflowView from '../src/views/ReleaseWorkflowView.vue'
import PublicCatalogsView from '../src/views/PublicCatalogsView.vue'

const api = vi.hoisted(() => ({ plans: vi.fn() }))
const navigation = vi.hoisted(() => ({ route: { path: '/workflows/release', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', async original => ({
  ...await original<typeof import('../src/api/client')>(),
  listCommercialPlans: api.plans,
}))

const Placeholder = defineComponent({ render: () => h('div') })
const CatalogStub = defineComponent({ props: ['initialPlanId', 'embedded'], render: () => h('div') })

test('release flow references an existing fixed plan before moving to catalog approval', async () => {
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  api.plans.mockResolvedValueOnce([{
    snapshot: { plan_id: 'paid_20', version: 2, definition: { name: '企业版', code: 'enterprise', edition: 'team' } },
    sha256: 'a'.repeat(64),
  }])
  const wrapper = mount(ReleaseWorkflowView, { global: { stubs: {
    PublicCatalogsView: CatalogStub,
    PublicationsView: Placeholder,
    ReleaseCenterView: Placeholder,
    ReleaseArtifactsView: Placeholder,
    EnvironmentUpgradesView: Placeholder,
  } } })
  await flushPromises()

  expect(wrapper.text()).toContain('选择套餐版本')
  expect(wrapper.findAll('.workflow-actions button')[1]!.attributes('disabled')).toBeDefined()
  wrapper.findComponent(ASelect).vm.$emit('update:modelValue', 'paid_20')
  await flushPromises()
  expect(wrapper.text()).toContain('企业版')
  expect(wrapper.findAll('.workflow-actions button')[1]!.attributes('disabled')).toBeUndefined()

  await wrapper.findAll('.workflow-actions button')[1]!.trigger('click')
  expect(wrapper.findComponent(PublicCatalogsView).props('initialPlanId')).toBe('paid_20')
  expect(navigation.push).toHaveBeenCalledWith(expect.objectContaining({ query: expect.objectContaining({ step: '2', plan: 'paid_20' }) }))
  wrapper.unmount()
})

test('release URL restores the selected node and fixed plan', async () => {
  navigation.route.query = { step: '2', plan: 'paid_20' }
  api.plans.mockResolvedValueOnce([{
    snapshot: { plan_id: 'paid_20', version: 2, definition: { name: '企业版', code: 'enterprise', edition: 'team' } },
    sha256: 'a'.repeat(64),
  }])
  const wrapper = mount(ReleaseWorkflowView, { global: { stubs: {
    PublicCatalogsView: CatalogStub, PublicationsView: Placeholder, ReleaseCenterView: Placeholder,
    ReleaseArtifactsView: Placeholder, EnvironmentUpgradesView: Placeholder,
  } } })
  await flushPromises()
  expect(wrapper.find('.workflow-stepper li.active').text()).toContain('批准目录')
  expect(wrapper.findComponent(PublicCatalogsView).props('initialPlanId')).toBe('paid_20')
  wrapper.unmount()
})
