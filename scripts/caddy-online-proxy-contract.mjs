import assert from 'node:assert/strict'
import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { mkdtempSync, rmSync, writeFileSync } from 'node:fs'
import { createServer } from 'node:http'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { setTimeout as sleep } from 'node:timers/promises'

async function within(promise, label, timeoutMs = 10000) {
  let timer
  try {
    return await Promise.race([
      promise,
      new Promise((_, reject) => { timer = setTimeout(() => reject(new Error(`${label} timed out`)), timeoutMs) }),
    ])
  } finally {
    clearTimeout(timer)
  }
}

async function listen(server) {
  await new Promise((resolve, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', resolve)
  })
  return server.address().port
}

function message(socket) {
  return within(new Promise((resolve, reject) => {
    const cleanup = () => {
      socket.removeEventListener('message', onMessage)
      socket.removeEventListener('close', onClose)
      socket.removeEventListener('error', onClose)
    }
    const onMessage = event => { cleanup(); resolve(String(event.data)) }
    const onClose = () => { cleanup(); reject(new Error('WebSocket closed before the expected message')) }
    socket.addEventListener('message', onMessage, { once: true })
    socket.addEventListener('close', onClose, { once: true })
    socket.addEventListener('error', onClose, { once: true })
  }), 'WebSocket message')
}

function textFrame(text) {
  const payload = Buffer.from(text)
  assert.ok(payload.length < 126)
  return Buffer.concat([Buffer.from([0x81, payload.length]), payload])
}

/** Real locked Caddy + deterministic local upstreams. No Customer runtime, paid
 * model, external network, installation service or TLS identity is involved. */
export async function verifyCaddyOnlineProxy(binary) {
  const stage = mkdtempSync(join(tmpdir(), 'aster-caddy-online-'))
  const servers = []
  const sockets = new Set()
  let child
  let childExited
  let websocket
  let streamResponse
  let oldWebSocket
  let logs = ''
  let phase = 'startup'
  try {
    const upstreams = {}
    for (const slot of ['blue', 'green']) {
      upstreams[slot] = {}
      for (const role of ['api', 'member', 'admin']) {
        const label = `${slot}-${role}`
        const server = createServer((request, response) => {
          if (request.url === '/api/stream' && slot === 'blue') {
            response.writeHead(200, { 'Content-Type': 'text/event-stream', 'Cache-Control': 'no-cache' })
            response.write(`data: ${label}\n\n`)
            streamResponse = response
          } else response.end(label)
        })
        server.on('upgrade', (request, socket) => {
          sockets.add(socket)
          socket.on('close', () => sockets.delete(socket))
          socket.on('error', () => {})
          const accept = createHash('sha1').update(`${request.headers['sec-websocket-key']}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64')
          socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`)
          socket.write(textFrame(label))
          // This fixture receives only the client's close frame.
          socket.on('data', () => socket.end(Buffer.from([0x88, 0x00])))
          if (slot === 'blue' && role === 'api') oldWebSocket = socket
        })
        servers.push(server)
        upstreams[slot][role] = await listen(server)
      }
    }
    // Hold both reservations until their distinct addresses are selected.
    const reservations = [createServer(), createServer()]
    servers.push(...reservations)
    const [adminPort, frontPort] = await Promise.all(reservations.map(listen))
    await Promise.all(reservations.map(server => new Promise(resolve => server.close(resolve))))
    const config = slot => ({
      admin: { listen: `127.0.0.1:${adminPort}`, config: { persist: false } },
      apps: { http: { servers: { business: {
        listen: [`127.0.0.1:${frontPort}`],
        routes: ['api', 'member', 'admin'].map(role => ({
          match: [{ path: [`/${role}/*`] }],
          handle: [{ handler: 'reverse_proxy', upstreams: [{ dial: `127.0.0.1:${upstreams[slot][role]}` }], stream_close_delay: 900000000000 }],
        })),
      } } } },
    })
    const configPath = join(stage, 'caddy.json')
    writeFileSync(configPath, JSON.stringify(config('blue')))
    child = spawn(binary, ['run', '--config', configPath], {
      cwd: stage, windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
      env: { ...process.env, XDG_DATA_HOME: join(stage, 'data'), XDG_CONFIG_HOME: join(stage, 'config'), HOME: stage },
    })
    const capture = data => { logs = (logs + String(data)).slice(-16384) }
    child.stdout.on('data', capture)
    child.stderr.on('data', capture)
    let startupError
    childExited = new Promise(resolve => {
      child.once('exit', resolve)
      child.once('error', error => { startupError = error; resolve() })
    })
    const endpoint = `http://127.0.0.1:${adminPort}/config/`
    const front = `http://127.0.0.1:${frontPort}`
    const adminOrigin = new URL(endpoint).origin
    // Each config replacement starts a new admin server and asynchronously
    // shuts down the old one (Caddy 2.11.3 admin.go:replaceLocalAdminServer).
    // Do not reuse an idle socket belonging to the retired server. This changes
    // transport ownership, not assertions or mutation/readback retry policy.
    const adminHeaders = { Origin: adminOrigin, Connection: 'close' }
    const getConfig = () => fetch(endpoint, { headers: adminHeaders, signal: AbortSignal.timeout(2000) })
    const startupDeadline = Date.now() + 10000
    let initial
    while (!initial) {
      if (startupError || child.exitCode !== null || child.signalCode !== null) throw startupError || new Error(`Caddy exited during startup: ${logs}`)
      try {
        const response = await getConfig()
        if (response.ok) initial = response
      } catch { /* Startup only: no mutation is replayed. */ }
      if (!initial) {
        if (Date.now() >= startupDeadline) throw new Error(`Caddy startup timed out: ${logs}`)
        await sleep(50)
      }
    }
    const denied = await fetch(endpoint, { headers: { ...adminHeaders, Origin: 'https://untrusted.example' }, signal: AbortSignal.timeout(2000) })
    assert.equal(denied.status, 403)
    await denied.text()
    const original = await initial.json()
    assert.deepEqual(original, config('blue'))
    const etag = initial.headers.get('etag')
    assert.match(etag, /^"\/config\/ [a-f0-9]+"$/)
    const stream = await fetch(`${front}/api/stream`, { signal: AbortSignal.timeout(60000) })
    const reader = stream.body.getReader()
    const first = await within(reader.read(), 'initial stream response')
    assert.match(Buffer.from(first.value).toString(), /blue-api/)
    websocket = new WebSocket(`ws://127.0.0.1:${frontPort}/api/ws`)
    assert.equal(await message(websocket), 'blue-api')

    phase = 'conditional cutover'
    const switched = await fetch(endpoint, {
      method: 'POST', headers: { ...adminHeaders, 'Content-Type': 'application/json', 'If-Match': etag },
      body: JSON.stringify(config('green')), signal: AbortSignal.timeout(5000),
    })
    assert.equal(switched.status, 200)
    await switched.text()
    phase = 'cutover readback'
    const current = await getConfig()
    assert.deepEqual(await current.json(), config('green'))
    for (const role of ['api', 'member', 'admin']) {
      const response = await fetch(`${front}/${role}/check`, { signal: AbortSignal.timeout(2000) })
      assert.equal(await response.text(), `green-${role}`)
    }
    phase = 'stale-writer rejection'
    // A stale config writer must not revert the successful cutover.
    const conflict = await fetch(endpoint, {
      method: 'POST', headers: { ...adminHeaders, 'Content-Type': 'application/json', 'If-Match': etag },
      body: JSON.stringify(config('blue')), signal: AbortSignal.timeout(5000),
    })
    assert.equal(conflict.status, 412)
    await conflict.text()
    assert.deepEqual(await (await getConfig()).json(), config('green'))

    phase = 'old stream and WebSocket completion'
    // Complete work on the old upstream after observing new traffic on all
    // entries. Reconnection cannot satisfy either of these assertions.
    const oldMessage = message(websocket)
    oldWebSocket.write(textFrame('blue-after-cutover'))
    assert.equal(await oldMessage, 'blue-after-cutover')
    streamResponse.end('data: blue-finished\n\n')
    let remaining = ''
    for (;;) {
      const chunk = await within(reader.read(), 'old stream completion')
      if (chunk.done) break
      remaining += Buffer.from(chunk.value).toString()
    }
    assert.match(remaining, /blue-finished/)
    websocket.close()
  } catch (error) {
    // Only deterministic local fixture configuration is logged by this child.
    // Keep its bounded diagnostics with the original error/cause before cleanup.
    throw new Error(`Caddy online contract failed during ${phase}; exit=${child?.exitCode}; signal=${child?.signalCode}\n${logs}`, { cause: error })
  } finally {
    websocket?.close()
    for (const socket of sockets) socket.destroy()
    if (child?.pid && child.exitCode === null && child.signalCode === null) {
      child.kill('SIGTERM')
      try { await within(childExited, 'Caddy shutdown', 5000) } catch {
        child.kill('SIGKILL')
        await within(childExited, 'Caddy forced shutdown', 5000)
      }
    }
    await Promise.all(servers.map(async server => {
      server.closeAllConnections()
      if (server.listening) await new Promise(resolve => server.close(resolve))
    }))
    rmSync(stage, { recursive: true, force: true })
  }
}
