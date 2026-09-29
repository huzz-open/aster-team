import { afterEach, expect, test, vi } from 'vitest'

afterEach(() => { document.cookie = 'aster_operations_csrf=; Max-Age=0; Path=/'; vi.unstubAllGlobals() })

test('API client maps structured errors and sends the CSRF cookie on mutations', async () => {
  document.cookie = 'aster_operations_csrf=csrf%20value; Path=/'
  const fetcher = vi.fn().mockResolvedValueOnce(Response.json({
    error: { code: 'VALIDATION_FAILED', message: 'invalid customer', number: 69_006 },
  }, { status: 400 })).mockResolvedValueOnce(Response.json({ id: 'customer_1', name: 'Customer' }, { status: 201 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { createCustomer } = await import('../src/api/client')

  await expect(createCustomer({ name: 'Customer' } as never)).rejects.toEqual(expect.objectContaining({
    code: 'VALIDATION_FAILED', message: 'invalid customer（错误码：69006）',
    status: 400, number: 69_006,
  }))
  await createCustomer({ name: 'Customer' } as never)
  const request = fetcher.mock.calls[1][0] as Request
  expect(request.headers.get('X-CSRF-Token')).toBe('csrf value')
  expect(new URL(request.url).pathname).toBe('/api/operations/v1/customers')
})

test('API client falls back to a stable error when a proxy returns no API error envelope', async () => {
  const fetcher = vi.fn().mockResolvedValue(new Response(null, { status: 500 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { login } = await import('../src/api/client')

  await expect(login('admin@example.com', 'password')).rejects.toEqual(expect.objectContaining({
    code: 'REQUEST_FAILED', message: '请求失败', status: 500,
  }))
})

test('high-risk payment confirmation forwards current password only in the request body', async () => {
  document.cookie = 'aster_operations_csrf=csrf-token; Path=/'
  const fetcher = vi.fn().mockResolvedValue(Response.json({ id: 'order_1', status: 'fulfillment_pending' }, { status: 200 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { confirmCommercialPayment } = await import('../src/api/client')
  await confirmCommercialPayment('order_1', { operation_id: 'payment-test', expected_order_sha256: 'a'.repeat(64), payment_reference: 'bank-reference', received_at: '2026-09-08T00:00:00.000Z', notes: 'reviewed' }, 'current-password')
  const request = fetcher.mock.calls[0][0] as Request
  const body = await request.clone().json()
  expect(body).toMatchObject({ payment_reference: 'bank-reference', notes: 'reviewed', current_password: 'current-password' })
  expect(request.headers.get('X-CSRF-Token')).toBe('csrf-token')
  expect(request.headers.get('Authorization')).toBeNull()
  expect(request.url).not.toContain('current-password')
})

test('release center reads task list and hierarchy without mutation headers', async () => {
  const fetcher = vi.fn()
    .mockResolvedValueOnce(Response.json({ items: [{ id: 'release_task_1' }] }, { status: 200 }))
    .mockResolvedValueOnce(Response.json({ task: { id: 'release_task_1' }, runs: [], artifacts: [] }, { status: 200 }))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { listReleaseTasks, getReleaseTask } = await import('../src/api/client')
  await expect(listReleaseTasks()).resolves.toEqual([{ id: 'release_task_1' }])
  await expect(getReleaseTask('release_task_1')).resolves.toMatchObject({ task: { id: 'release_task_1' }, runs: [], artifacts: [] })
  const requests = fetcher.mock.calls.map(call => call[0] as Request)
  expect(requests.map(request => new URL(request.url).pathname)).toEqual([
    '/api/operations/v1/release-tasks', '/api/operations/v1/release-tasks/release_task_1',
  ])
  expect(requests.every(request => request.method === 'GET' && !request.headers.has('X-CSRF-Token'))).toBe(true)
})

test('release workflow mutations are fixed API calls with CSRF and structured inputs', async () => {
  document.cookie = 'aster_operations_csrf=release-csrf; Path=/'
  const body = { task: { id: 'release_task_1' }, runs: [], artifacts: [] }
  const fetcher = vi.fn().mockImplementation(() => Promise.resolve(Response.json(body, { status: 201 })))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { createReleaseTask, syncReleaseTask, retryReleaseTask, reverifyReleaseTask } = await import('../src/api/client')
  await createReleaseTask('2.0.0', 'main', 'dist_free_1')
  await syncReleaseTask('release_task_1')
  await retryReleaseTask('release_task_1')
  await reverifyReleaseTask('release_task_1', 'artifact_windows')
  const requests = fetcher.mock.calls.map(call => call[0] as Request)
  expect(requests.map(request => new URL(request.url).pathname)).toEqual([
    '/api/operations/v1/release-tasks',
    '/api/operations/v1/release-tasks/release_task_1/sync',
    '/api/operations/v1/release-tasks/release_task_1/retry',
    '/api/operations/v1/release-tasks/release_task_1/artifacts/artifact_windows/reverify',
  ])
  expect(requests.every(request => request.method === 'POST' && request.headers.get('X-CSRF-Token') === 'release-csrf')).toBe(true)
  await expect(requests[0].clone().json()).resolves.toEqual({ version: '2.0.0', source_ref: 'main', free_distribution_id: 'dist_free_1' })
  expect((await requests[1].clone().text())).toBe('')
})

test('release mutations reject invalid versions before making a request', async () => {
  const fetcher = vi.fn()
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { createReleaseTask, importReleaseArtifact } = await import('../src/api/client')

  await expect(createReleaseTask('v2.0.0', 'main', 'dist_free_1')).rejects.toThrow('不能使用 v 前缀或前导零')
  await expect(importReleaseArtifact({ version: '02.0.0' } as never)).rejects.toThrow('不能使用 v 前缀或前导零')
  expect(fetcher).not.toHaveBeenCalled()
})

test('formal publish approval and execution keep reauthentication in structured request bodies', async () => {
  document.cookie = 'aster_operations_csrf=publish-csrf; Path=/'
  const responseBody = { id: 'publish_1', status: 'approved' }
  const fetcher = vi.fn().mockImplementation(() => Promise.resolve(Response.json(responseBody, { status: 200 })))
  vi.stubGlobal('fetch', fetcher)
  vi.resetModules()
  const { requestReleasePublish, decideReleasePublish, executeReleasePublish } = await import('../src/api/client')
  await requestReleasePublish('release_1')
  await decideReleasePublish('publish_1', 'approved', 'independent review complete', 'approval-password')
  await executeReleasePublish('publish_1', 'execution-password')
  const requests = fetcher.mock.calls.map(call => call[0] as Request)
  expect(requests.map(request => new URL(request.url).pathname)).toEqual([
    '/api/operations/v1/release-artifacts/release_1/publish-requests',
    '/api/operations/v1/release-publish-requests/publish_1/decision',
    '/api/operations/v1/release-publish-requests/publish_1/execute',
  ])
  expect(requests.every(request => request.headers.get('X-CSRF-Token') === 'publish-csrf')).toBe(true)
  await expect(requests[1].clone().json()).resolves.toEqual({ decision: 'approved', comment: 'independent review complete', current_password: 'approval-password' })
  await expect(requests[2].clone().json()).resolves.toEqual({ current_password: 'execution-password' })
  expect(requests.every(request => !request.url.includes('password'))).toBe(true)
})
