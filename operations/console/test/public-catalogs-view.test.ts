import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import PublicCatalogsView from '../src/views/PublicCatalogsView.vue'
import ReauthActionModal from '../src/components/ReauthActionModal.vue'
import { OperationsAPIError, type ApprovePublicCatalogInput, type CatalogApprovalRecord, type PublicCatalogPreview, type PublicCatalogRequest } from '../src/api/client'

const api = vi.hoisted(() => ({ list: vi.fn(), plans: vi.fn(), preview: vi.fn(), approve: vi.fn(), get: vi.fn(), export: vi.fn(), download: vi.fn() }))
const navigation = vi.hoisted(() => ({ route: { path: '/commercial/catalogs', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
vi.mock('../src/api/client', async original => ({ ...await original<typeof import('../src/api/client')>(), getCurrentOperationsOperatorID: () => 'operator_1', listCatalogApprovals: api.list, listCommercialPlans: api.plans, previewPublicCatalog: api.preview, approvePublicCatalog: api.approve, getCatalogApproval: api.get, exportPublicCatalog: api.export, downloadPublicCatalog: api.download }))
const id = `catalog_${'a'.repeat(48)}`
function preview(request: PublicCatalogRequest): PublicCatalogPreview { return { catalog: { schema: 'aster.public-plans.v1', revision: id, product: 'aster-team', environment: request.environment, plans: [] }, sha256: 'b'.repeat(64) } }
function record(request: PublicCatalogRequest = { operation_id: 'initial', environment: 'local', reason: '测试目录', plans: [] }): CatalogApprovalRecord { return { snapshot: { schema: 'aster.catalog-approval.v1', id, request, plans: [], public_sha256: 'b'.repeat(64), approved_by: 'operator_1', approved_at: '2026-09-06T00:00:00.000Z' }, sha256: 'c'.repeat(64), public: preview(request), status: 'approved' } }
function render() { return mount(PublicCatalogsView, { global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } } }) }
function button(wrapper: ReturnType<typeof render>, text: string) { return wrapper.findAll('button').find(b => b.text() === text)! }
async function inspect(wrapper: ReturnType<typeof render>) { await button(wrapper, '新建公开目录').trigger('click'); await flushPromises(); await wrapper.get('textarea[aria-label="批准说明"]').setValue('审核这份完整目录'); await wrapper.get('form').trigger('submit'); await flushPromises() }
async function confirmPassword(wrapper: ReturnType<typeof render>, password: string) {
  const dialog = wrapper.getComponent(ReauthActionModal)
  expect(dialog.props('open')).toBe(true)
  dialog.vm.$emit('submit', password)
  await flushPromises()
}
beforeEach(() => {
  navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset()
  sessionStorage.clear(); api.list.mockReset().mockResolvedValue([record()]); api.plans.mockReset().mockResolvedValue([])
  api.preview.mockReset().mockImplementation((request: PublicCatalogRequest) => Promise.resolve(preview(request)))
  api.approve.mockReset().mockImplementation((input: ApprovePublicCatalogInput) => Promise.resolve(record(input.request)))
  api.get.mockReset().mockResolvedValue(record()); api.export.mockReset().mockResolvedValue({ ...record(), status: 'exported' }); api.download.mockReset().mockResolvedValue(undefined)
})
test('catalog approval keeps exact input across response loss, later denial and reload without persisting passwords', async () => {
  api.approve.mockRejectedValueOnce(new TypeError('响应丢失')).mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_PERMISSION_DENIED', '权限暂不可用', 403))
  let wrapper = render(); await flushPromises(); await inspect(wrapper)
  await wrapper.get('form').trigger('submit'); await confirmPassword(wrapper, 'first-secret')
  const first = structuredClone(api.approve.mock.calls[0][0])
  expect(sessionStorage.getItem('aster.operations.pending.v1:catalog:operator_1')).not.toContain('first-secret')
  expect(wrapper.find('input[aria-label="当前密码"]').exists()).toBe(false)
  await confirmPassword(wrapper, 'second-secret')
  expect(api.approve.mock.calls[1][0]).toEqual(first); expect(sessionStorage.length).toBe(1)
  wrapper.unmount(); api.get.mockRejectedValueOnce(new OperationsAPIError('NOT_FOUND', '尚未保存', 404))
  wrapper = render(); await flushPromises(); await button(wrapper, '继续批准').trigger('click'); await flushPromises()
  expect(api.preview).toHaveBeenLastCalledWith(first.request)
  await wrapper.get('form').trigger('submit'); await confirmPassword(wrapper, 'third-secret')
  expect(api.approve.mock.calls[2][0]).toEqual(first); expect(sessionStorage.length).toBe(0); expect(wrapper.find('form').exists()).toBe(false)
  wrapper.unmount()
})
test('an already stored approval is recovered by identity without issuing a new write', async () => {
  api.approve.mockRejectedValueOnce(new TypeError('响应丢失'))
  let wrapper = render(); await flushPromises(); await inspect(wrapper)
  await wrapper.get('form').trigger('submit'); await confirmPassword(wrapper, 'secret')
  const input = api.approve.mock.calls[0][0] as ApprovePublicCatalogInput
  wrapper.unmount(); api.get.mockResolvedValue(record(input.request))
  wrapper = render(); await flushPromises(); await button(wrapper, '继续批准').trigger('click'); await flushPromises()
  expect(api.approve).toHaveBeenCalledTimes(1); expect(sessionStorage.length).toBe(0); expect(wrapper.find('form').exists()).toBe(false)
  wrapper.unmount()
})
test('confirmed preview conflict requires a fresh preview and retains the operator reason', async () => {
  api.approve.mockRejectedValueOnce(new OperationsAPIError('COMMERCIAL_CATALOG_PREVIEW_CONFLICT', '需要重新核对', 409, 67710))
  const wrapper = render(); await flushPromises(); await inspect(wrapper)
  const first = api.preview.mock.calls[0][0]
  await wrapper.get('form').trigger('submit'); await confirmPassword(wrapper, 'secret')
  expect(sessionStorage.length).toBe(0); expect(wrapper.find('input[aria-label="当前密码"]').exists()).toBe(false)
  expect((wrapper.get('textarea[aria-label="批准说明"]').element as HTMLTextAreaElement).value).toBe(first.reason)
  await wrapper.get('form').trigger('submit'); await flushPromises()
  expect(api.preview.mock.calls[1][0].operation_id).not.toBe(first.operation_id)
  wrapper.unmount()
})
test('file errors allow retry of the same export and clear its password; list failures clear stale rows', async () => {
  const wrapper = render(); await flushPromises(); await button(wrapper, '查看').trigger('click'); await flushPromises()
  api.export.mockRejectedValueOnce(new Error('文件已写入但回执失败'))
  await button(wrapper, '导出到运营主机').trigger('click'); await confirmPassword(wrapper, 'export-secret')
  expect(wrapper.find('input[aria-label="导出验证密码"]').exists()).toBe(false)
  await confirmPassword(wrapper, 'new-secret')
  expect(api.export.mock.calls.map(call => call[0])).toEqual([id, id]); expect(sessionStorage.length).toBe(0)
  await button(wrapper, '下载 plans.json').trigger('click'); await flushPromises(); expect(api.download).toHaveBeenCalledTimes(1)
  wrapper.unmount()
  const next = render(); await flushPromises(); api.list.mockRejectedValueOnce(new Error('列表读取失败')); await button(next, '刷新').trigger('click'); await flushPromises()
  expect(next.findAll('table tbody tr')).toHaveLength(0); expect(next.text()).toContain('列表读取失败'); next.unmount()
})

test('a failed detail read can retry the exact record without exposing stale export actions', async () => {
  api.get.mockRejectedValueOnce(new Error('详情读取失败'))
  const wrapper = render(); await flushPromises(); await button(wrapper, '查看').trigger('click'); await flushPromises()
  expect(wrapper.text()).toContain('详情读取失败')
  expect(button(wrapper, '导出到运营主机')).toBeUndefined()
  await button(wrapper, '重新读取目录').trigger('click'); await flushPromises()
  expect(api.get.mock.calls.map(call => call[0])).toEqual([id, id])
  expect(button(wrapper, '导出到运营主机')).toBeDefined()
  wrapper.unmount()
})
test('a catalog detail URL restores the exact record', async () => {
  navigation.route.query = { catalog: id }
  const wrapper = render(); await flushPromises()
  expect(api.get).toHaveBeenCalledWith(id)
  expect(wrapper.text()).toContain('公开目录详情')
  wrapper.unmount()
})

test('embedded catalog approval stays in the workflow and starts with the selected fixed plan', async () => {
  api.plans.mockResolvedValueOnce([{
    snapshot: { plan_id: 'paid_20', version: 2, definition: { name: '企业版', code: 'enterprise' } },
    sha256: 'a'.repeat(64),
  }])
  const wrapper = mount(PublicCatalogsView, {
    props: { embedded: true, initialPlanId: 'paid_20' },
    global: { stubs: { Teleport: true, RouterLink: { template: '<a><slot /></a>' } } },
  })
  await flushPromises()
  await button(wrapper, '新建公开目录').trigger('click')
  await flushPromises()
  expect(wrapper.find('.catalog-workspace').exists()).toBe(true)
  expect(wrapper.find('[role="dialog"]').exists()).toBe(false)
  expect(wrapper.find('.catalog-selection').text()).toContain('企业版 · v2')
  wrapper.unmount()
})
