import { beforeEach, describe, expect, it, vi } from 'vitest'
import { request } from '@aster/sdk'
import { adminPageAccess, adminPageAllowed } from '../src/page-access'
import { adminProfile, clearAdminProfile, loadAdminProfile, type AdminProfile } from '../src/license-status'
import router from '../src/router'

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

const profile = (features: string[]): AdminProfile => ({
  email: 'owner@example.test', password_change_required: false, license_state: 'active',
  license: { state: 'active', available: true, features },
})

describe('Admin page license requirements', () => {
  it('requires an explicit policy for every business shell route', () => {
    const shell = router.options.routes.find(route => route.path === '/')!
    const pages = shell.children!.filter(route => !route.redirect).map(route => {
      const path = `/${route.path}`
      expect(route.meta?.adminPage, path).toBe(path)
      expect(Object.hasOwn(adminPageAccess, path), path).toBe(true)
      return path
    })
    expect(pages.sort()).toEqual(Object.keys(adminPageAccess).sort())
  })
  it('uses the explicit minimal capability for each business page', () => {
    const expected = {
      member: ['/users', '/consumption-logs'],
      gateway: ['/models', '/upstream-accounts'],
      runner: ['/runners'],
    }
    for (const [feature, allowed] of Object.entries(expected)) {
      for (const page of Object.values(expected).flat()) {
        expect(adminPageAllowed(page, profile([feature]).license), `${feature}: ${page}`).toBe(allowed.includes(page))
      }
    }
  })

  it('keeps recovery pages available while rejecting missing, stale and unknown business claims', () => {
    const support = ['/overview', '/audit-events', '/license', '/maintenance', '/account/password', '/settings']
    for (const license of [undefined, { state: 'active', available: true }, { state: 'unavailable', available: false, features: ['runner'] }]) {
      for (const page of support) expect(adminPageAllowed(page, license)).toBe(true)
      for (const page of Object.keys(adminPageAccess).filter(page => !support.includes(page))) {
        expect(adminPageAllowed(page, license), page).toBe(false)
      }
    }
    expect(adminPageAllowed('/future-unclassified', profile(['gateway', 'member', 'runner']).license)).toBe(false)
    expect(adminPageAllowed('__proto__', profile(['runner']).license)).toBe(false)
    expect(adminPageAllowed('/models', profile(['unknown']).license)).toBe(false)
  })
})

it('retains only previously licensed business pages after expiry', () => {
  const expired = { state: 'expired', available: false, features: ['member'] }
  expect(adminPageAllowed('/users', expired)).toBe(true)
  expect(adminPageAllowed('/consumption-logs', expired)).toBe(true)
  expect(adminPageAllowed('/runners', expired)).toBe(false)
  expect(adminPageAllowed('/users', { ...expired, features: [] })).toBe(false)
})

describe('Admin profile refresh ordering', () => {
  beforeEach(() => { clearAdminProfile(); vi.mocked(request).mockReset() })

  it('never restores an older feature set after a newer response', async () => {
    let finishOld!: (value: AdminProfile) => void
    vi.mocked(request).mockReturnValueOnce(new Promise(resolve => { finishOld = resolve }))
    vi.mocked(request).mockResolvedValueOnce(profile(['runner']))
    const old = loadAdminProfile()
    await loadAdminProfile()
    finishOld(profile(['gateway', 'member', 'runner']))
    await old
    expect(adminProfile.value?.license?.features).toEqual(['runner'])
  })

  it('clears features on the newest failure and ignores an older success', async () => {
    let finishOld!: (value: AdminProfile) => void
    vi.mocked(request).mockReturnValueOnce(new Promise(resolve => { finishOld = resolve }))
    vi.mocked(request).mockRejectedValueOnce(new Error('unavailable'))
    const old = loadAdminProfile()
    await expect(loadAdminProfile()).rejects.toThrow('unavailable')
    finishOld(profile(['gateway']))
    await old
    expect(adminProfile.value).toBeNull()
  })

  it('cannot restore an old session after logout', async () => {
    let finish!: (value: AdminProfile) => void
    vi.mocked(request).mockReturnValueOnce(new Promise(resolve => { finish = resolve }))
    const pending = loadAdminProfile()
    clearAdminProfile()
    finish(profile(['gateway']))
    await pending
    expect(adminProfile.value).toBeNull()
  })
})
