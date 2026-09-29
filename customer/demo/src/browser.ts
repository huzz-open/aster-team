import { http, HttpResponse } from 'msw'
import { setupWorker } from 'msw/browser'
import { adminOverview, adminProfile, hourlyUsageSummary, memberDocs, memberKeys, memberModels, memberProfile, members, previousUsageSummary, syntheticDemoProfile, usageSummary } from './fixtures'

const handlers = [
  http.get('/api/public/license-state', () => HttpResponse.json({ state: 'active', available: true, features: ['gateway', 'member', 'runner'] })),
  http.get('/api/admin/me', () => HttpResponse.json(adminProfile)),
  http.get('/api/admin/overview', () => HttpResponse.json(adminOverview)),
  http.get('/api/admin/users', () => HttpResponse.json({ items: members })),
  http.get('/api/admin/vouchers/recipients', () => HttpResponse.json({ items: members.filter(member => member.status === 'active').map(({ id, email, display_name }) => ({ id, email, display_name })) })),
  http.get('/api/admin/consumption-logs/members', () => HttpResponse.json({ items: members.filter(member => member.status !== 'deleted').map(({ id, email, display_name }) => ({ id, email, display_name })) })),
  http.get('/api/admin/runners/connection', () => HttpResponse.json({ public_api_base_url: window.location.origin })),
  http.get('/api/member/me', () => HttpResponse.json(memberProfile)),
  http.get('/api/member/keys', () => HttpResponse.json({ items: memberKeys })),
  http.get('/api/member/models', () => HttpResponse.json({ items: memberModels })),
  http.get('/api/member/docs', () => HttpResponse.json(memberDocs)),
  http.get('/api/member/usage-summary', ({ request }) => {
    const searchParams = new URL(request.url).searchParams
    if (searchParams.get('period') === '1d') return HttpResponse.json(hourlyUsageSummary)
    const to = searchParams.get('to')
    return HttpResponse.json(to && to < '2026-08-28' ? previousUsageSummary : usageSummary)
  }),
]

const worker = setupWorker(...handlers)

export async function startDemoMock(): Promise<void> {
  document.documentElement.dataset.asterDataMode = 'mock'
  document.documentElement.dataset.asterDemoProfile = syntheticDemoProfile
  await worker.start({
    serviceWorker: { url: '/mockServiceWorker.js' },
    quiet: true,
    onUnhandledRequest(request, print) {
      if (new URL(request.url).pathname.startsWith('/api/')) print.warning()
    },
  })
}
