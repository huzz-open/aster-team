import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { ASelect } from '@aster/ui'
import ReleaseCenterView from '../src/views/ReleaseCenterView.vue'
import { listReleaseTasks, reverifyReleaseTask, type ReleaseTask } from '../src/api/client'

vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  listReleaseTasks: vi.fn(),
  reverifyReleaseTask: vi.fn(async () => ({})),
  getReleaseCapabilities: vi.fn(async () => ({ configured: true, targets: [{ platform: 'linux', architecture: 'amd64' }, { platform: 'windows', architecture: 'amd64' }] })),
  listReleasePublishRequests: vi.fn(async () => []),
  listFreeDistributions: vi.fn(async () => []),
}))

enableAutoUnmount(afterEach)
afterEach(() => vi.unstubAllGlobals())
beforeEach(() => {
  vi.clearAllMocks()
  vi.spyOn(URL, 'createObjectURL').mockReturnValue('blob:release-download')
  vi.spyOn(URL, 'revokeObjectURL').mockImplementation(() => {})
  vi.spyOn(HTMLAnchorElement.prototype, 'click').mockImplementation(() => {})
  vi.mocked(listReleaseTasks).mockImplementation(async () => [
    { id: 'task_1', version: '2.0.0', source_commit_sha: 'abc123', free_distribution_id: 'dist_free_1', free_license_sha256: 'a'.repeat(64), status: 'completed', github_conclusion: 'success', packages: [
      { id: 'linux', platform: 'linux', architecture: 'amd64', verification_status: 'verified', release_artifact_id: 'release_linux', file_name: 'aster-team-2.0.0-linux-amd64.tar.gz' },
      { id: 'windows', platform: 'windows', architecture: 'amd64', verification_status: 'pending', github_digest_sha256: 'd'.repeat(64), file_name: 'aster-team-2.0.0-windows-amd64.tar.gz' },
    ] },
    { id: 'task_2', version: '2.0.1', source_commit_sha: 'def456', free_distribution_id: 'dist_free_1', free_license_sha256: 'a'.repeat(64), status: 'failed', github_conclusion: 'success', packages: [
      { id: 'linux_failed', platform: 'linux', architecture: 'amd64', verification_status: 'failed', verification_error_code: 'RELEASE_SIGNATURE_INVALID' },
      { id: 'windows_verified', platform: 'windows', architecture: 'amd64', verification_status: 'verified', release_artifact_id: 'release_windows', file_name: 'aster-team-2.0.1-windows-amd64.tar.gz' },
    ] },
  ] as ReleaseTask[])
})

function deferred<T>() {
  let resolve!: (value: T) => void
  let reject!: (reason: Error) => void
  const promise = new Promise<T>((resolvePromise, rejectPromise) => { resolve = resolvePromise; reject = rejectPromise })
  return { promise, resolve, reject }
}
function responseFor(platform = 'linux', version = '2.0.0') {
  return new Response('verified archive', { headers: { 'Content-Disposition': `attachment; filename="aster-team-${version}-${platform}-amd64.tar.gz"` } })
}
async function releaseCenter() {
  const wrapper = mount(ReleaseCenterView, { global: { stubs: { Teleport: true } } })
  await flushPromises()
  const downloads = wrapper.findAllComponents(ASelect).filter(select => select.props('placeholder') === '下载')
  return { wrapper, downloads }
}

test('the dropdown shows independent platform states and downloads the selected Windows package with its server filename', async () => {
  const fetcher = vi.fn().mockResolvedValue(responseFor('windows', '2.0.1'))
  vi.stubGlobal('fetch', fetcher)
  const { wrapper, downloads } = await releaseCenter()
  expect(wrapper.text()).toContain('部分复验完成')
  await downloads[0].get('[role="combobox"]').trigger('click')
  const unavailableWindows = downloads[0].findAll('[role="option"]')[1]
  expect(unavailableWindows.text()).toContain('Windows')
  expect(unavailableWindows.text()).toContain('未复验')
  expect(unavailableWindows.attributes('aria-disabled')).toBe('true')
  await unavailableWindows.trigger('click')
  expect(fetcher).not.toHaveBeenCalled()
  await downloads[1].get('[role="combobox"]').trigger('click')
  expect(downloads[1].findAll('[role="option"]')[0].attributes('aria-disabled')).toBe('true')
  await downloads[1].findAll('[role="option"]')[1].trigger('click')
  await flushPromises()
  expect(fetcher).toHaveBeenCalledExactlyOnceWith('/api/operations/v1/release-artifacts/release_windows/download', { credentials: 'include' })
  expect(HTMLAnchorElement.prototype.click).toHaveBeenCalledOnce()
  expect(vi.mocked(HTMLAnchorElement.prototype.click).mock.instances[0].download).toBe('aster-team-2.0.1-windows-amd64.tar.gz')
})

test('repeated activation makes one request and keeps that package disabled throughout the body transfer', async () => {
  const body = deferred<Blob>()
  const response = responseFor()
  vi.spyOn(response, 'blob').mockReturnValue(body.promise)
  const fetcher = vi.fn().mockResolvedValue(response)
  vi.stubGlobal('fetch', fetcher)
  const { downloads } = await releaseCenter()
  for (let index = 0; index < 3; index++) downloads[0].vm.$emit('change', 'linux-amd64')
  await flushPromises()
  expect(fetcher).toHaveBeenCalledOnce()
  expect(downloads[0].props('options')[0]).toMatchObject({ disabled: true, description: '下载中…' })
  expect(downloads[1].props('options')[1].disabled).toBe(false)
  expect(HTMLAnchorElement.prototype.click).not.toHaveBeenCalled()
  const archive = new Blob(['verified archive'])
  body.resolve(archive)
  await flushPromises()
  expect(URL.createObjectURL).toHaveBeenCalledExactlyOnceWith(archive)
  expect(HTMLAnchorElement.prototype.click).toHaveBeenCalledOnce()
  expect(vi.mocked(HTMLAnchorElement.prototype.click).mock.instances[0].download).toBe('aster-team-2.0.0-linux-amd64.tar.gz')
  expect(URL.revokeObjectURL).toHaveBeenCalledExactlyOnceWith('blob:release-download')
  expect(downloads[0].props('options')[0].disabled).toBe(false)
})

test.each(['request', 'body'] as const)('a failed %s unlocks the selected package for a manual retry', async stage => {
  const body = deferred<Blob>()
  const response = responseFor()
  vi.spyOn(response, 'blob').mockReturnValue(body.promise)
  const fetcher = vi.fn().mockResolvedValue(response)
  if (stage === 'request') fetcher.mockRejectedValueOnce(new Error('connection failed'))
  vi.stubGlobal('fetch', fetcher)
  const { downloads } = await releaseCenter()
  downloads[0].vm.$emit('change', 'linux-amd64')
  if (stage === 'body') body.reject(new Error('transfer interrupted'))
  await flushPromises()
  expect(downloads[0].props('options')[0].disabled).toBe(false)
  expect(HTMLAnchorElement.prototype.click).not.toHaveBeenCalled()
  fetcher.mockResolvedValueOnce(responseFor())
  downloads[0].vm.$emit('change', 'linux-amd64')
  await flushPromises()
  expect(fetcher).toHaveBeenCalledTimes(2)
  expect(HTMLAnchorElement.prototype.click).toHaveBeenCalledOnce()
})

test('verification is requested for the selected historical Windows package only', async () => {
  const { wrapper } = await releaseCenter()
  const selector = wrapper.findAllComponents(ASelect).find(select => select.props('placeholder') === '重新复验')!
  await selector.get('[role="combobox"]').trigger('click')
  expect(selector.findAll('[role="option"]')[0].attributes('aria-disabled')).toBe('true')
  await selector.findAll('[role="option"]')[1].trigger('click')
  await flushPromises()
  expect(reverifyReleaseTask).toHaveBeenCalledExactlyOnceWith('task_1', 'windows')
})

test('a missing server filename fails instead of silently saving a Windows response as Linux', async () => {
  vi.stubGlobal('fetch', vi.fn().mockResolvedValue(new Response('archive without filename')))
  const { downloads } = await releaseCenter()
  downloads[1].vm.$emit('change', 'windows-amd64')
  await flushPromises()
  expect(HTMLAnchorElement.prototype.click).not.toHaveBeenCalled()
  expect(downloads[1].props('options')[1].disabled).toBe(false)
})

test('multiple tasks referring to one immutable package share its download lock', async () => {
  const items = await listReleaseTasks()
  vi.mocked(listReleaseTasks).mockResolvedValue([...items, { ...items[0], id: 'task_duplicate' }])
  const body = deferred<Blob>()
  const response = responseFor()
  vi.spyOn(response, 'blob').mockReturnValue(body.promise)
  const fetcher = vi.fn().mockResolvedValue(response)
  vi.stubGlobal('fetch', fetcher)
  const { downloads } = await releaseCenter()
  downloads[0].vm.$emit('change', 'linux-amd64')
  downloads[2].vm.$emit('change', 'linux-amd64')
  await flushPromises()
  expect(fetcher).toHaveBeenCalledOnce()
  expect(downloads[2].props('options')[0].disabled).toBe(true)
  body.resolve(new Blob(['archive']))
  await flushPromises()
  expect(downloads[2].props('options')[0].disabled).toBe(false)
})
