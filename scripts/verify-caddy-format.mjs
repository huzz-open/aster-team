import {
  chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync,
} from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

import { prepareVerifiedDownload } from './release-download-cache.mjs'
import { verifyCaddyOnlineProxy } from './caddy-online-proxy-contract.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const runtime = JSON.parse(readFileSync(resolve(root, 'tools/caddy-runtime.json'), 'utf8'))
const caddySources = [
  'customer/deploy/install.sh',
  'customer/deploy/macos/install-macos.sh',
  'customer/deploy/windows/install.ps1',
  'customer/backend/cli/src/maintenance_executor.rs',
]
const caddyDirectives = '(?:admin|persist_config|auto_https|reverse_proxy|stream_close_delay|bind|tls|import)\\s'
const spaceIndentationPatterns = [
  new RegExp(`\\n {2}(?=${caddyDirectives})`, 'g'),
  new RegExp(`\\\\n {2}(?=${caddyDirectives})`, 'g'),
  new RegExp('`n {2}(?=' + caddyDirectives + ')', 'g'),
]

const globalOptions = `{
\tadmin 127.0.0.1:2019
\tpersist_config off
\tauto_https disable_redirects
}

import /opt/aster-team/config/caddy/upstreams.caddy

`

const site = ({ address, upstream, tls = '' }) => `${address} {
\tbind 127.0.0.1
${tls ? `\t${tls}\n` : ''}\timport ${upstream}
}
`

const samples = [
  ['HTTP IP Caddyfile', globalOptions
    + site({ address: 'http://127.0.0.1:11080', upstream: 'aster_api_upstream' })
    + '\n'
    + site({ address: 'http://127.0.0.1:11081', upstream: 'aster_member_upstream' })
    + '\n'
    + site({ address: 'http://127.0.0.1:11082', upstream: 'aster_admin_upstream' })],
  ['internal TLS domain Caddyfile', globalOptions
    + site({ address: 'https://api.team.example', upstream: 'aster_api_upstream', tls: 'tls internal' })
    + '\n'
    + site({ address: 'https://app.team.example', upstream: 'aster_member_upstream', tls: 'tls internal' })
    + '\n'
    + site({ address: 'https://admin.team.example', upstream: 'aster_admin_upstream', tls: 'tls internal' })],
  ['external TLS domain Caddyfile', globalOptions
    + site({
      address: 'https://api.team.example',
      upstream: 'aster_api_upstream',
      tls: 'tls /opt/aster-team/config/tls/server.crt /opt/aster-team/config/tls/server.key',
    })],
  ['Caddy upstream snippets', `(aster_api_upstream) {
\treverse_proxy 127.0.0.1:11380 {
\t\tstream_close_delay 15m
\t}
}

(aster_member_upstream) {
\treverse_proxy 127.0.0.1:11381 {
\t\tstream_close_delay 15m
\t}
}

(aster_admin_upstream) {
\treverse_proxy 127.0.0.1:11382 {
\t\tstream_close_delay 15m
\t}
}
`],
]

function run(command, args, options = {}) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: options.encoding,
    env: process.env,
    input: options.input,
    maxBuffer: 4 * 1024 * 1024,
    stdio: options.stdio,
  })
  if (result.error) throw result.error
  if (result.status !== 0) {
    const detail = result.stderr?.trim() || result.stdout?.trim() || `status ${result.status}`
    throw new Error(`${command} ${args.join(' ')} failed: ${detail}`)
  }
  return result
}

function lockedRuntime() {
  const platform = process.platform === 'win32'
    ? 'windows'
    : process.platform === 'darwin' ? 'macos' : process.platform === 'linux' ? 'linux' : ''
  const architecture = process.arch === 'x64' ? 'amd64' : process.arch === 'arm64' ? 'arm64' : ''
  if (!platform || !architecture) throw new Error(`Caddy format verification does not support ${process.platform}/${process.arch}`)

  const url = runtime[`${platform}_${architecture}_url`]
  const sha512 = runtime[`${platform}_${architecture}_sha512`]
  if (!/^https:\/\/github\.com\/caddyserver\/caddy\/releases\/download\//.test(url || '')
      || !/^[0-9a-f]{128}$/.test(sha512 || '')) {
    throw new Error(`tools/caddy-runtime.json has no valid Caddy lock for ${platform}/${architecture}`)
  }
  return { architecture, platform, sha512, url }
}

async function prepareCaddy() {
  if (process.env.ASTER_CADDY_BIN) return { binary: resolve(process.env.ASTER_CADDY_BIN), cleanup: () => {} }

  const locked = lockedRuntime()
  const windows = locked.platform === 'windows'
  const extension = windows ? 'zip' : 'tar.gz'
  const archive = await prepareVerifiedDownload({
    cacheDirectory: resolve(root, 'target/release-downloads'),
    fileName: `caddy-${runtime.version}-${locked.platform}-${locked.architecture}.${extension}`,
    algorithm: 'sha512',
    expectedDigest: locked.sha512,
    download: path => run(windows ? 'curl.exe' : 'curl', [
      '--fail', '--location', '--retry', '4', '--output', path, locked.url,
    ], { stdio: 'inherit' }),
  })
  const stage = mkdtempSync(join(tmpdir(), 'aster-caddy-format-'))
  const executable = windows ? 'caddy.exe' : 'caddy'
  run(windows ? 'tar.exe' : 'tar', [windows ? '-xf' : '-xzf', archive, '-C', stage, executable], { stdio: 'inherit' })
  const binary = resolve(stage, executable)
  if (!windows) chmodSync(binary, 0o755)
  // Windows can briefly retain the executable mapping after child exit.
  // Retry only filesystem cleanup; verification failures still propagate.
  return { binary, cleanup: () => rmSync(stage, { recursive: true, force: true, maxRetries: 10, retryDelay: 100 }) }
}

function verifySourceIndentation() {
  const failures = []
  for (const relativePath of caddySources) {
    const source = readFileSync(resolve(root, relativePath), 'utf8')
    for (const pattern of spaceIndentationPatterns) {
      for (const match of source.matchAll(pattern)) {
        const line = source.slice(0, match.index).split('\n').length
        failures.push(`${relativePath}:${line} emits a Caddy directive with two-space indentation`)
      }
    }
  }
  if (failures.length) throw new Error(failures.join('\n'))
}

function verifySamples(binary) {
  const version = run(binary, ['version'], { encoding: 'utf8' }).stdout.trim()
  if (!new RegExp(`^v?${runtime.version.replaceAll('.', '\\.')}\\b`).test(version)) {
    throw new Error(`expected Caddy ${runtime.version}, received ${version}`)
  }
  for (const [name, source] of samples) {
    const formatted = run(binary, ['fmt', '-'], { encoding: 'utf8', input: source }).stdout
    if (formatted !== source) {
      let index = 0
      while (source[index] === formatted[index] && index < source.length && index < formatted.length) index += 1
      const line = source.slice(0, index).split('\n').length
      throw new Error(`${name} is not canonical according to caddy fmt (first difference at line ${line})`)
    }
  }
}

if (!/^\d+\.\d+\.\d+$/.test(runtime.version)) throw new Error('tools/caddy-runtime.json has an invalid version')
for (const relativePath of caddySources) {
  if (!existsSync(resolve(root, relativePath))) throw new Error(`Caddy source is missing: ${relativePath}`)
}
mkdirSync(resolve(root, 'target'), { recursive: true })
verifySourceIndentation()
const caddy = await prepareCaddy()
try {
  verifySamples(caddy.binary)
  await verifyCaddyOnlineProxy(caddy.binary)
} finally {
  caddy.cleanup()
}
console.log(`Caddy ${runtime.version} format, atomic cutover and retained-stream contracts verified.`)
