import { mount } from '@vue/test-utils'
import { afterEach, beforeEach, expect, test, vi } from 'vitest'
import { createMemoryHistory, createRouter } from 'vue-router'
import SessionExpiredDialog from '../src/components/SessionExpiredDialog.vue'
import { notifyOperationsSessionExpired, OPERATIONS_SESSION_EXPIRED_EVENT } from '../src/session-expiry'

beforeEach(() => vi.useFakeTimers())
afterEach(() => {
  vi.useRealTimers()
  document.cookie = 'aster_operations_csrf=; Max-Age=0; Path=/'
  vi.unstubAllGlobals()
})

async function mountDialog() {
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [
      { path: '/login', component: { template: '<div>login</div>' }, meta: { public: true } },
      { path: '/release-center', component: { template: '<div>release center</div>' } },
    ],
  })
  await router.push('/release-center')
  await router.isReady()
  const wrapper = mount(SessionExpiredDialog, { global: { plugins: [router], stubs: { Teleport: true } } })
  return { router, wrapper }
}

test('session expiry dialog explains the interruption and automatically returns to login', async () => {
  document.cookie = 'aster_operations_csrf=csrf-token; Path=/'
  const { router, wrapper } = await mountDialog()

  notifyOperationsSessionExpired()
  await wrapper.vm.$nextTick()

  expect(wrapper.text()).toContain('登录状态已过期')
  expect(wrapper.text()).toContain('即将自动返回登录页')
  expect(document.cookie).not.toContain('aster_operations_csrf=csrf-token')
  await vi.advanceTimersByTimeAsync(1_500)
  expect(router.currentRoute.value.path).toBe('/login')
  expect(router.currentRoute.value.query.redirect).toBe('/release-center')
})

test('protected API unauthorized errors trigger the dialog event without exposing an inline message', async () => {
  const expired = vi.fn()
  window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  const fetcher = vi.fn().mockResolvedValue(Response.json({
    error: { code: 'UNAUTHORIZED', message: '请先登录', number: 61_001 },
  }, { status: 401 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { getOverview } = await import('../src/api/client')

  await expect(getOverview()).rejects.toEqual(expect.objectContaining({
    name: 'OperationsSessionExpiredError', message: '', code: 'UNAUTHORIZED', status: 401, number: 61_001,
  }))
  expect(expired).toHaveBeenCalledTimes(1)
  window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
})

test('raw and download API paths use the same silent session expiry handling', async () => {
  const expired = vi.fn()
  window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  const unauthorized = () => Response.json({
    error: { code: 'UNAUTHORIZED', message: '会话无效或已过期', number: 61_001 },
  }, { status: 401 })
  const fetcher = vi.fn().mockImplementation(() => Promise.resolve(unauthorized()))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { downloadReleaseArtifact, listCommercialOrders } = await import('../src/api/client')

  await expect(listCommercialOrders()).rejects.toMatchObject({ name: 'OperationsSessionExpiredError', message: '' })
  await expect(downloadReleaseArtifact('release_1')).rejects.toMatchObject({ name: 'OperationsSessionExpiredError', message: '' })
  expect(expired).toHaveBeenCalledTimes(2)
  window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
})

test('free approval and signing password failures preserve the session and show a retryable error', async () => {
  document.cookie = 'aster_operations_csrf=still-valid; Path=/'
  const expired = vi.fn()
  window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  const fetcher = vi.fn().mockResolvedValueOnce(Response.json({ operator: { id: 'operator_1', email: 'admin@example.com' } }))
    .mockImplementation(() => Promise.resolve(Response.json({
      error: { code: 'REAUTHENTICATION_FAILED', message: '当前密码验证失败', number: 61_007 },
    }, { status: 401 })))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { getSession, approveFreeDistribution, issueFreeDistribution, getCurrentOperationsOperatorID } = await import('../src/api/client')
  await getSession()
  try {
    for (const request of [
      () => approveFreeDistribution({ operation_id: 'original_operation' } as never, 'wrong-password'),
      () => issueFreeDistribution('dist_1', 'free_key', 'wrong-password'),
    ]) {
      await expect(request()).rejects.toMatchObject({ name: 'OperationsAPIError', code: 'REAUTHENTICATION_FAILED', message: '当前密码验证失败（错误码：61007）', status: 401 })
      expect(getCurrentOperationsOperatorID()).toBe('operator_1')
      expect(document.cookie).toContain('aster_operations_csrf=still-valid')
    }
    expect(expired).not.toHaveBeenCalled()
    const approval = fetcher.mock.calls[1][0] as Request
    expect(approval.headers.get('X-CSRF-Token')).toBe('still-valid')
    expect(approval.url).not.toContain('wrong-password')
    await expect(approval.clone().json()).resolves.toEqual({ operation_id: 'original_operation', current_password: 'wrong-password' })
  } finally {
    window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  }
})

test('login failures stay visible on the login form and do not announce session expiry', async () => {
  const expired = vi.fn()
  window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  const fetcher = vi.fn().mockResolvedValue(Response.json({
    error: { code: 'UNAUTHORIZED', message: '邮箱或密码错误', number: 61_001 },
  }, { status: 401 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { login } = await import('../src/api/client')

  await expect(login('admin@example.com', 'wrong-password')).rejects.toEqual(expect.objectContaining({
    name: 'OperationsAPIError', message: '邮箱或密码错误（错误码：61001）', status: 401,
  }))
  expect(expired).not.toHaveBeenCalled()
  window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
})

test('session checks only announce expiry after this app instance observed a valid session', async () => {
  const expired = vi.fn()
  window.addEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
  const unauthorized = () => Response.json({
    error: { code: 'UNAUTHORIZED', message: '请先登录', number: 61_001 },
  }, { status: 401 })
  const fetcher = vi.fn()
    .mockResolvedValueOnce(unauthorized())
    .mockResolvedValueOnce(Response.json({ operator: { email: 'admin@example.com' } }, { status: 200 }))
    .mockResolvedValueOnce(unauthorized())
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { getSession } = await import('../src/api/client')

  await expect(getSession()).rejects.toMatchObject({ name: 'OperationsAPIError', message: '请先登录（错误码：61001）' })
  expect(expired).not.toHaveBeenCalled()
  await expect(getSession()).resolves.toMatchObject({ operator: { email: 'admin@example.com' } })
  await expect(getSession()).rejects.toMatchObject({ name: 'OperationsSessionExpiredError', message: '' })
  expect(expired).toHaveBeenCalledTimes(1)
  window.removeEventListener(OPERATIONS_SESSION_EXPIRED_EVENT, expired)
})
