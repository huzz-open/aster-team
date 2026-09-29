import { handleInquiryPost } from '../../server/inquiry-handler'

export const onRequest: PagesFunction<Env> = async (context) => {
  if (context.request.method === 'POST') return handleInquiryPost(context)
  return new Response(null, { status: 405, headers: { Allow: 'POST', 'Cache-Control': 'no-store', 'X-Content-Type-Options': 'nosniff' } })
}
