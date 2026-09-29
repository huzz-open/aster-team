// @vitest-environment node
import { mkdtemp, mkdir, writeFile, rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { createServer } from 'vite'
import { expect, it } from 'vitest'
import { localDocumentationRoutes, localPublicAssets } from '../../../scripts/local-documentation-vite'

it.each(['.docs-public', 'public'])('serves deep links and preserves other public assets from %s', async publicDirectory => {
  const root = await mkdtemp(join(tmpdir(), 'aster-docs-routes-'))
  const docs = join(root, publicDirectory, 'docs/zh-cn/administration')
  await mkdir(docs, { recursive: true })
  await writeFile(join(root, 'index.html'), '<html>console shell</html>')
  await writeFile(join(docs, 'index.html'), '<html>deployment guide</html>')
  await writeFile(join(docs, 'licensing.html'), '<html>license guide</html>')
  await writeFile(join(root, publicDirectory, 'site-icon.svg'), '<svg>website asset</svg>')
  await mkdir(join(root, 'public'), { recursive: true })
  await writeFile(join(root, 'public/install-runner.ps1'), '[CmdletBinding()]\nparam()')
  await writeFile(join(root, 'public/install-runner.sh'), '#!/usr/bin/env bash\nset -Eeuo pipefail')
  const server = await createServer({
    configFile: false, root, publicDir: publicDirectory,
    plugins: [localPublicAssets(root), localDocumentationRoutes(root, publicDirectory)],
    server: { host: '127.0.0.1', port: 0 },
  })
  try {
    await server.listen()
    const address = server.httpServer!.address()
    if (!address || typeof address === 'string') throw new Error('Missing test server address')
    const base = `http://127.0.0.1:${address.port}`
    for (const [path, content] of [
      ['/docs/zh-cn/administration/', 'deployment guide'],
      ['/docs/zh-cn/administration/licensing?from=admin', 'license guide'],
      ['/docs/zh-cn/administration/licensing.html', 'license guide'],
      ['/site-icon.svg', 'website asset'],
      ['/install-runner.ps1', '[CmdletBinding()]'],
      ['/install-runner.sh', '#!/usr/bin/env bash'],
      ['/overview', 'console shell'],
    ]) {
      const response = await fetch(`${base}${path}`)
      expect(response.status).toBe(200)
      expect(await response.text()).toContain(content)
    }
    const installer = await fetch(`${base}/install-runner.ps1`)
    expect(installer.headers.get('content-type')).toBe('text/plain; charset=utf-8')
    expect(installer.headers.get('x-content-type-options')).toBe('nosniff')
  } finally {
    await server.close()
    await rm(root, { recursive: true, force: true })
  }
})
