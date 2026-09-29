import { expect, test } from 'vitest'
import router, { requireOperationsSession } from '../src/router'

const target = (path: string, publicRoute = false) => ({ path, fullPath: path, meta: { public: publicRoute } }) as never

test('session guard forces password rotation before operations pages', async () => {
  const load = async () => ({ operator: { password_change_required: true } }) as never
  await expect(requireOperationsSession(target('/commercial/orders'), load)).resolves.toBe('/change-password')
  await expect(requireOperationsSession(target('/change-password'), load)).resolves.toBe(true)
})

test('session guard returns to the requested page after login', async () => {
  const load = async () => { throw new Error('unauthorized') }
  await expect(requireOperationsSession(target('/commercial/orders'), load)).resolves.toEqual({ path: '/login', query: { redirect: '/commercial/orders' } })
  await expect(requireOperationsSession(target('/login', true), load)).resolves.toBe(true)
})

test('release center has stable list and task detail routes', () => {
  expect(router.resolve('/release-center').matched).toHaveLength(2)
  expect(router.resolve('/release-center/release_task_1').params.taskID).toBe('release_task_1')
})

test('workflow and base-data routes form the primary operations navigation', () => {
  for (const path of ['/workflows/business', '/workflows/release', '/base/customers', '/base/plans', '/base/signing', '/system/audit']) {
    expect(router.resolve(path).matched).toHaveLength(2)
  }
})


test('license issuance is available only through commercial lifecycle routes', () => {
  expect(router.getRoutes().some(route => ['/licenses', '/trials', '/plans', '/orders', '/deliveries'].includes(route.path))).toBe(false)
  expect(router.resolve('/commercial/orders').matched).toHaveLength(2)
  expect(router.resolve('/commercial/fulfillments').matched).toHaveLength(2)
  expect(router.resolve('/release-artifacts').matched).toHaveLength(2)
  expect(router.resolve('/commercial/distributions').matched).toHaveLength(2)
})
