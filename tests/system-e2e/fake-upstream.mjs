import { createHash, randomBytes, timingSafeEqual } from 'node:crypto'
import { createServer } from 'node:https'
import { readFileSync } from 'node:fs'

const port = Number(process.env.ASTER_E2E_FAKE_UPSTREAM_PORT || 443)
const expectedEmail = process.env.ASTER_E2E_UPSTREAM_EMAIL || 'upstream@example.test'
const expectedPassword = process.env.ASTER_E2E_UPSTREAM_PASSWORD || 'Upstream-E2E-Password-2026!'
const controlToken = process.env.ASTER_E2E_FAKE_CONTROL_TOKEN
const diagnosticCanary = process.env.ASTER_E2E_DIAGNOSTIC_CANARY || ''
if (!controlToken || controlToken.length < 32) throw new Error('ASTER_E2E_FAKE_CONTROL_TOKEN is required')

const oauthClientID = 'app_EMoamEEZ73f0CkXaXp7hrann'
const oauthRedirectURI = 'http://localhost:1455/auth/callback'
const accountID = 'aster-e2e-upstream-account'
const requests = []
const authorizations = new Map()
const authorizationCodes = new Map()
const refreshTokens = new Set()
const accessTokens = new Set()
const cases = new Map()
const png = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAusB9Y9Z7V8AAAAASUVORK5CYII='

function base64url(value) {
  return Buffer.from(JSON.stringify(value)).toString('base64url')
}

function token() {
  const value = `${base64url({ alg: 'none', typ: 'JWT' })}.${base64url({
    email: expectedEmail,
    'https://api.openai.com/auth': { chatgpt_account_id: accountID, chatgpt_plan_type: 'team' },
    'https://api.openai.com/profile': { email: expectedEmail },
  })}.${randomBytes(12).toString('base64url')}`
  accessTokens.add(value)
  return value
}

function respond(response, status, contentType, payload, headers = {}) {
  response.writeHead(status, { 'content-type': contentType, 'cache-control': 'no-store', ...headers })
  response.end(payload)
}

function json(response, status, payload) {
  respond(response, status, 'application/json; charset=utf-8', JSON.stringify(payload))
}

function error(response, status, code, detail) {
  return json(response, status, { error: { code, message: detail || code, type: 'aster_e2e_mock_error' } })
}

async function body(request) {
  const chunks = []
  for await (const chunk of request) chunks.push(chunk)
  return Buffer.concat(chunks)
}

function secureEqual(left, right) {
  const first = Buffer.from(left || '')
  const second = Buffer.from(right || '')
  return first.length === second.length && timingSafeEqual(first, second)
}

function controlAuthorized(request) {
  const authorization = request.headers.authorization || ''
  return authorization.startsWith('Bearer ') && secureEqual(authorization.slice(7), controlToken)
}

function requireHeader(request, name, expected) {
  const value = request.headers[name]
  if (!value || (expected && value !== expected)) throw new Error(`required ${name} header is invalid`)
  return value
}

function requireAccess(request) {
  const authorization = requireHeader(request, 'authorization')
  if (!authorization.startsWith('Bearer ') || !accessTokens.has(authorization.slice(7))) throw new Error('access token is unknown')
  if (requireHeader(request, 'chatgpt-account-id') !== accountID) throw new Error('ChatGPT account ID is invalid')
  requireHeader(request, 'originator', 'Aster Team')
}

function escapeHTML(value) {
  return value.replaceAll('&', '&amp;').replaceAll('"', '&quot;').replaceAll('<', '&lt;').replaceAll('>', '&gt;')
}

function loginPage(state) {
  return `<!doctype html><html><body><main><h1>Aster E2E upstream login</h1><form method="post" action="/login"><input type="hidden" name="state" value="${escapeHTML(state)}"><label>Email<input name="email" type="email" autocomplete="username"></label><label>Password<input name="password" type="password" autocomplete="current-password"></label><button type="submit">Sign in</button></form></main></body></html>`
}

function validateAuthorize(url) {
  const required = {
    response_type: 'code', client_id: oauthClientID, redirect_uri: oauthRedirectURI,
    scope: 'openid profile email offline_access api.connectors.read api.connectors.invoke', code_challenge_method: 'S256',
    id_token_add_organizations: 'true', codex_cli_simplified_flow: 'true', originator: 'codex_cli_rs',
  }
  for (const [name, expected] of Object.entries(required)) {
    if (url.searchParams.get(name) !== expected) throw new Error(`OAuth authorize ${name} is invalid`)
  }
  const state = url.searchParams.get('state') || ''
  const challenge = url.searchParams.get('code_challenge') || ''
  if (!/^[A-Za-z0-9_-]{22,}$/.test(state)) throw new Error('OAuth state has insufficient entropy')
  if (!/^[A-Za-z0-9_-]{43}$/.test(challenge)) throw new Error('OAuth PKCE S256 challenge is invalid')
  authorizations.set(state, { challenge, createdAt: Date.now() })
  return state
}

function issueTokens(response) {
  const refreshToken = `refresh_${randomBytes(24).toString('base64url')}`
  refreshTokens.add(refreshToken)
  return json(response, 200, { access_token: token(), refresh_token: refreshToken, id_token: token(), token_type: 'Bearer', expires_in: 3600 })
}

function validateTokenRequest(request, encoded) {
  if (!(request.headers['content-type'] || '').startsWith('application/x-www-form-urlencoded')) throw new Error('OAuth token content type is invalid')
  const form = new URLSearchParams(encoded.toString('utf8'))
  if (form.get('client_id') !== oauthClientID) throw new Error('OAuth client ID is invalid')
  const grant = form.get('grant_type')
  if (grant === 'authorization_code') {
    if (form.get('redirect_uri') !== oauthRedirectURI) throw new Error('OAuth redirect URI is invalid')
    const code = form.get('code') || ''
    const record = authorizationCodes.get(code)
    if (!record) throw new Error('OAuth authorization code is invalid or already consumed')
    authorizationCodes.delete(code)
    const verifier = form.get('code_verifier') || ''
    const challenge = createHash('sha256').update(verifier).digest('base64url')
    if (!secureEqual(challenge, record.challenge)) throw new Error('OAuth PKCE verifier does not match')
    return
  }
  if (grant === 'refresh_token') {
    const refreshToken = form.get('refresh_token') || ''
    if (!refreshTokens.has(refreshToken)) throw new Error('OAuth refresh token is unknown')
    return
  }
  throw new Error('OAuth grant type is unsupported')
}

function requestCase(input) {
  const marker = JSON.stringify(input).match(/__aster_e2e_case:([A-Za-z0-9_-]{4,64})/)
  return marker?.[1] || ''
}

function completedResponse(input) {
  const image = Array.isArray(input.tools) && input.tools.some(item => item?.type === 'image_generation')
  const output = image
    ? [{ id: 'image_aster_e2e', type: 'image_generation_call', result: png, revised_prompt: 'Aster E2E image' }]
    : [{ id: 'message_aster_e2e', type: 'message', role: 'assistant', content: [{ type: 'output_text', text: 'Aster E2E response' }] }]
  return {
    id: `response_${randomBytes(8).toString('hex')}`,
    object: 'response', model: input.model, output,
    usage: { input_tokens: 12, output_tokens: 4, input_tokens_details: { cached_tokens: 2, cache_write_tokens: 1 } },
  }
}

function sse(response, completed, mode = 'ok') {
  if (mode === 'invalid_json') return respond(response, 200, 'text/event-stream; charset=utf-8', 'event: response.completed\ndata: {broken\n\n')
  if (mode === 'invalid_sse') return respond(response, 200, 'text/event-stream; charset=utf-8', 'this is not an SSE frame')
  const item = completed.output[0]
  const events = [`event: response.output_item.added\ndata: ${JSON.stringify({ type: 'response.output_item.added', output_index: 0, item })}\n\n`]
  if (item.type === 'message') events.push(`event: response.output_text.delta\ndata: ${JSON.stringify({ type: 'response.output_text.delta', output_index: 0, content_index: 0, delta: 'Aster E2E response' })}\n\n`)
  if (mode === 'interrupt') {
    response.writeHead(200, { 'content-type': 'text/event-stream; charset=utf-8', 'cache-control': 'no-store' })
    response.write(events.join(''))
    return response.destroy()
  }
  events.push(`event: response.completed\ndata: ${JSON.stringify({ type: 'response.completed', response: completed })}\n\n`)
  return respond(response, 200, 'text/event-stream; charset=utf-8', events.join(''))
}

function validateResponses(request, encoded) {
  requireAccess(request)
  requireHeader(request, 'accept', 'text/event-stream')
  requireHeader(request, 'openai-beta', 'responses_websockets=2026-02-06')
  if (!(request.headers['content-type'] || '').startsWith('application/json')) throw new Error('Responses content type is invalid')
  if (!request.headers['x-client-request-id']) throw new Error('logical request ID is missing')
  let input
  try { input = JSON.parse(encoded.toString('utf8')) } catch { throw new Error('Responses body is invalid JSON') }
  if (!input || typeof input !== 'object' || typeof input.model !== 'string' || !Array.isArray(input.input)) throw new Error('Responses model and normalized input are required')
  if (input.stream !== true || input.store !== false) throw new Error('Codex transport must stream and disable storage')
  if (input.tools !== undefined && !Array.isArray(input.tools)) throw new Error('Responses tools must be an array')
  for (const tool of input.tools || []) {
    if (tool?.type !== 'image_generation' || !['generate', 'edit'].includes(tool.action)) throw new Error('image tool contract is invalid')
    if (tool.action === 'edit' && !JSON.stringify(input.input).includes('input_image')) throw new Error('image edit is missing input images')
  }
  return input
}

function recordRequest(request, url, encoded, caseID = '', details = {}) {
  requests.push({
    method: request.method, host: request.headers.host, path: url.pathname, body_bytes: encoded.length,
    case_id: caseID || undefined, has_authorization: Boolean(request.headers.authorization),
    client_request_id: request.headers['x-client-request-id'] || undefined,
    ...details,
  })
}

const server = createServer({ key: readFileSync('/certs/server.key'), cert: readFileSync('/certs/server.crt') }, async (request, response) => {
  const url = new URL(request.url || '/', `https://${request.headers.host || 'chatgpt.com'}`)
  const encoded = await body(request)
  try {
    if (url.pathname.startsWith('/__e2e/')) {
      if (!controlAuthorized(request)) return error(response, 401, 'control_unauthorized')
      if (request.method === 'GET' && url.pathname === '/__e2e/requests') return json(response, 200, { items: requests })
      if (request.method === 'POST' && /^\/__e2e\/cases\/[A-Za-z0-9_-]{4,64}$/.test(url.pathname)) {
        const caseID = url.pathname.split('/').at(-1)
        const scenario = JSON.parse(encoded.toString('utf8'))
        if (!['status', 'invalid_json', 'invalid_sse', 'interrupt', 'timeout'].includes(scenario.mode)) return error(response, 400, 'invalid_scenario')
        if (scenario.mode === 'status' && ![400, 401, 403, 404, 409, 429, 500, 502, 503].includes(scenario.status)) return error(response, 400, 'invalid_status')
        cases.set(caseID, scenario)
        return json(response, 201, { case_id: caseID })
      }
      return error(response, 404, 'control_not_found')
    }
    if (request.method === 'GET' && url.pathname === '/healthz') return json(response, 200, { status: 'ok' })
    if (request.method === 'GET' && url.pathname === '/oauth/authorize') {
      const state = validateAuthorize(url)
      recordRequest(request, url, encoded)
      return respond(response, 200, 'text/html; charset=utf-8', loginPage(state))
    }
    if (request.method === 'POST' && url.pathname === '/login') {
      const form = new URLSearchParams(encoded.toString('utf8'))
      const authorization = authorizations.get(form.get('state') || '')
      if (!authorization || form.get('email') !== expectedEmail || form.get('password') !== expectedPassword) return error(response, 401, 'invalid_test_credentials')
      authorizations.delete(form.get('state'))
      const code = `code_${randomBytes(24).toString('base64url')}`
      authorizationCodes.set(code, authorization)
      const callback = new URL(oauthRedirectURI)
      callback.searchParams.set('code', code)
      callback.searchParams.set('state', form.get('state'))
      recordRequest(request, url, encoded)
      response.writeHead(302, { location: callback.toString(), 'cache-control': 'no-store' })
      return response.end()
    }
    if (request.method === 'POST' && url.pathname === '/oauth/token') {
      validateTokenRequest(request, encoded)
      recordRequest(request, url, encoded)
      return issueTokens(response)
    }
    if (request.method === 'GET' && url.pathname === '/backend-api/codex/models') {
      requireAccess(request)
      requireHeader(request, 'accept', 'application/json')
      recordRequest(request, url, encoded)
      return json(response, 200, { models: [{ slug: 'gpt-5.6-e2e', display_name: 'GPT 5.6 E2E' }] })
    }
    if (request.method === 'POST' && url.pathname === '/backend-api/codex/responses') {
      const input = validateResponses(request, encoded)
      const caseID = requestCase(input)
      recordRequest(request, url, encoded, caseID, {
        model: input.model,
        service_tier: input.service_tier,
        reasoning_effort: input.reasoning?.effort,
      })
      const scenario = caseID ? cases.get(caseID) : undefined
      if (caseID) cases.delete(caseID)
      if (scenario?.mode === 'status') return error(response, scenario.status, `mock_${scenario.status}`)
      if (scenario?.mode === 'timeout') return setTimeout(() => response.destroy(), Number(scenario.delay_ms || 35_000))
      return sse(response, completedResponse(input), scenario?.mode)
    }
    return error(response, 404, 'not_found', `${request.method} ${url.pathname}`)
  } catch (caught) {
    const message = caught instanceof Error ? caught.message : String(caught)
    if (diagnosticCanary && message.includes(diagnosticCanary)) return error(response, 500, 'canary_rejected')
    return error(response, 400, 'strict_mock_contract_violation', message)
  }
})

server.listen(port, '0.0.0.0', () => {
  process.stdout.write(`Aster strict deterministic upstream listening on https://0.0.0.0:${port}\n`)
})
