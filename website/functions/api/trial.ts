import { handleTrialPost } from '../../server/trial-handler'

export const onRequest: PagesFunction<Env> = async (context) => {
  if (context.request.method === 'POST') return handleTrialPost(context)
  return Response.json({ ok: false, error: 'method_not_allowed' }, {
    status: 405,
    headers: {
      'Allow': 'POST',
      'Cache-Control': 'no-store',
      'X-Content-Type-Options': 'nosniff',
    },
  })
}
