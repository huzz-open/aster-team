import type { Plugin } from 'vite'
import { createReadStream, existsSync, statSync } from 'node:fs'
import { extname, isAbsolute, relative, resolve } from 'node:path'

const publicContentTypes: Record<string, string> = {
  '.png': 'image/png',
  '.ps1': 'text/plain; charset=utf-8',
  '.sh': 'text/plain; charset=utf-8',
  '.svg': 'image/svg+xml',
}

export function localPublicAssets(appRoot: string): Plugin {
  const publicAssets = resolve(appRoot, 'public')
  return {
    name: 'aster-local-public-assets',
    apply: 'serve' as const,
    configureServer(server) {
      server.middlewares.use((request, response, next) => {
        if (!request.url || (request.method !== 'GET' && request.method !== 'HEAD')) return next()
        let pathname: string
        try {
          pathname = decodeURIComponent(new URL(request.url, 'http://localhost').pathname)
        } catch {
          return next()
        }
        const asset = resolve(publicAssets, `.${pathname}`)
        const assetRelative = relative(publicAssets, asset)
        if (!assetRelative || assetRelative.startsWith('..') || isAbsolute(assetRelative) || !existsSync(asset)) return next()
        const details = statSync(asset)
        if (!details.isFile()) return next()
        response.setHeader('Content-Type', publicContentTypes[extname(asset)] || 'application/octet-stream')
        response.setHeader('Content-Length', details.size)
        response.setHeader('X-Content-Type-Options', 'nosniff')
        if (request.method === 'HEAD') {
          response.end()
          return
        }
        const stream = createReadStream(asset)
        stream.on('error', () => response.destroy())
        stream.pipe(response)
      })
    },
  }
}

export function localDocumentationRoutes(appRoot: string, publicDirectory = '.docs-public'): Plugin {
  const documentationAssets = resolve(appRoot, publicDirectory)
  return {
    name: 'aster-local-documentation',
    apply: 'serve' as const,
    configureServer(server) {
      server.middlewares.use((request, _response, next) => {
        if (!request.url) return next()
        const [pathname = '', query = ''] = request.url.split('?', 2)
        if (!pathname.startsWith('/docs/') || extname(pathname)) return next()
        const relativePath = pathname.slice(1)
        const htmlFile = pathname.endsWith('/')
          ? resolve(documentationAssets, relativePath, 'index.html')
          : resolve(documentationAssets, `${relativePath}.html`)
        const htmlRelative = relative(documentationAssets, htmlFile)
        if (htmlRelative && !htmlRelative.startsWith('..') && !isAbsolute(htmlRelative) && existsSync(htmlFile)) {
          request.url = `${pathname.endsWith('/') ? `${pathname}index.html` : `${pathname}.html`}${query ? `?${query}` : ''}`
        }
        next()
      })
    },
  }
}
