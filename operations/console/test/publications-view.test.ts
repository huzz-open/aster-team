import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import PublicationsView from '../src/views/PublicationsView.vue'

const navigation = vi.hoisted(() => ({ route: { path: '/commercial/publications', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
const api = vi.hoisted(() => ({ get: vi.fn(), list: vi.fn(), failures: vi.fn(), head: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', async original => ({
  ...await original<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  getPublication: api.get, listPublications: api.list, listPublicationFailures: api.failures, getPublicationHead: api.head,
}))

const publicationID = 'publication_1'
const publication = {
  snapshot: {
    id: publicationID,
    request: { reason: '官网核对', build_sha256: 'a'.repeat(64), accept_until: '2027-01-01T00:00:00.000Z', expected_active_id: '' },
    catalog: { id: 'catalog_1', request: { environment: 'local' } },
  },
  status: 'prepared',
}
beforeEach(() => {
  sessionStorage.clear()
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  api.get.mockReset().mockResolvedValue(publication)
  api.list.mockReset().mockResolvedValue([publication])
  api.failures.mockReset().mockResolvedValue([])
  api.head.mockReset().mockResolvedValue('')
})

test('a publication detail URL reopens the exact record after refresh', async () => {
  navigation.route.query = { publication: publicationID }
  const wrapper = mount(PublicationsView, { global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } } })
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith(publicationID)
  expect(wrapper.text()).toContain('发布核对详情')
  wrapper.unmount()
})
