import { createHash } from 'node:crypto'
import { readFile } from 'node:fs/promises'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseEnv } from 'node:util'
import type { Plugin } from 'vite'

type LocalAccountTarget = 'customer' | 'operations' | 'member'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const credentialsPath = resolve(root, 'data/local/local-admin-credentials.env')
const endpoint = '/__aster_local_admin_credentials'

export type LocalAdminCredentials = { email: string; password: string; sha256: string }

function isLoopback(address: string | undefined): boolean {
  return address === '127.0.0.1' || address === '::1' || address === '::ffff:127.0.0.1'
}

export function localAdminCredentials(target: LocalAccountTarget): Plugin {
  return {
    name: `aster-local-${target}-admin-credentials`,
    apply: 'serve',
    configureServer(server) {
      server.middlewares.use(async (request, response, next) => {
        const pathname = new URL(request.url || '/', 'http://127.0.0.1').pathname
        if (pathname !== endpoint) return next()
        if (request.method !== 'GET') {
          response.statusCode = 405
          response.end()
          return
        }
        if (!isLoopback(request.socket.remoteAddress)) {
          response.statusCode = 403
          response.end()
          return
        }
        try {
          const body = JSON.stringify(await readLocalAdminCredentials(target))
          response.statusCode = 200
          response.setHeader('Content-Type', 'application/json; charset=utf-8')
          response.setHeader('Cache-Control', 'no-store')
          response.setHeader('Content-Length', Buffer.byteLength(body))
          response.end(body)
        } catch {
          const body = JSON.stringify({ error: '本地账号凭据不完整，请检查 data/local/local-admin-credentials.env。' })
          response.statusCode = 503
          response.setHeader('Content-Type', 'application/json; charset=utf-8')
          response.setHeader('Cache-Control', 'no-store')
          response.setHeader('Content-Length', Buffer.byteLength(body))
          response.end(body)
        }
      })
    },
  }
}

export async function readLocalAdminCredentials(
  target: LocalAccountTarget,
  path = credentialsPath,
): Promise<LocalAdminCredentials> {
  const prefix = target === 'customer'
    ? 'ASTER_LOCAL_CUSTOMER'
    : target === 'operations' ? 'ASTER_LOCAL_OPERATIONS' : 'ASTER_LOCAL_MEMBER'
  const source = await readFile(path)
  const values = parseEnv(source.toString('utf8'))
  const email = values[`${prefix}_EMAIL`]
  const password = values[`${prefix}_PASSWORD`]
  if (!email || !password) throw new Error(`missing ${prefix} credentials`)
  return { email, password, sha256: createHash('sha256').update(source).digest('hex') }
}
