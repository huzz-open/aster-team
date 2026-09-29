import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import SigningEnvironmentView from '../src/views/SigningEnvironmentView.vue'

const navigation = vi.hoisted(() => ({ route: { path: '/base/signing', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
const api = vi.hoisted(() => ({ list: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', async original => ({ ...await original<typeof import('../src/api/client')>(), listV2IssuerProfiles: api.list }))

beforeEach(() => {
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  api.list.mockReset().mockResolvedValue([
    { key_id: 'issuer_1', policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'] } },
    { key_id: 'issuer_2', policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'] } },
  ])
})

test('a signing profile URL restores the selected detail', async () => {
  navigation.route.query = { key: 'issuer_2' }
  const wrapper = mount(SigningEnvironmentView)
  await flushPromises()
  expect(wrapper.find('.master-list button.active').text()).toContain('issuer_2')
  expect(wrapper.find('.detail-panel h2').text()).toBe('issuer_2')
  wrapper.unmount()
})
