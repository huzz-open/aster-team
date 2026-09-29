import { execFileSync } from 'node:child_process'

const LOCAL_PROXY_BYPASS = ['127.0.0.1', 'localhost', '::1']

function proxyUrl(host, port) {
  const normalizedHost = host.includes(':') && !host.startsWith('[') ? `[${host}]` : host
  try {
    const url = new URL(`http://${normalizedHost}:${port}`)
    if (url.username || url.password || url.pathname !== '/' || url.search || url.hash) return null
    return url.href.slice(0, -1)
  } catch {
    return null
  }
}

export function parseMacosSystemProxies(output) {
  const values = new Map()
  for (const line of output.split(/\r?\n/u)) {
    const match = line.match(/^\s*([A-Za-z]+)\s*:\s*(.*?)\s*$/u)
    if (match) values.set(match[1], match[2])
  }

  const result = {}
  for (const [scheme, prefix] of [['http', 'HTTP'], ['https', 'HTTPS']]) {
    if (values.get(`${prefix}Enable`) !== '1') continue
    const host = values.get(`${prefix}Proxy`)
    const port = values.get(`${prefix}Port`)
    if (!host || !port || !/^\d{1,5}$/u.test(port) || Number(port) > 65_535) continue
    const url = proxyUrl(host, port)
    if (url) result[scheme] = url
  }
  return result
}

function hasProxy(environment, scheme) {
  const upper = scheme.toUpperCase()
  return Boolean(
    environment[`${upper}_PROXY`]
    || environment[`${scheme}_proxy`]
    || environment.ALL_PROXY
    || environment.all_proxy,
  )
}

function withLocalProxyBypass(environment) {
  const key = Object.hasOwn(environment, 'NO_PROXY') ? 'NO_PROXY' : 'no_proxy'
  const entries = (environment[key] || '').split(',').map(value => value.trim()).filter(Boolean)
  for (const host of LOCAL_PROXY_BYPASS) {
    if (!entries.includes(host)) entries.push(host)
  }
  environment[key] = entries.join(',')
}

export function localRunnerProxyEnvironment(
  baseEnvironment,
  { platform = process.platform, systemProxyOutput } = {},
) {
  const environment = { ...baseEnvironment }
  if (platform !== 'darwin') return environment
  const proxyAlreadyConfigured = hasProxy(environment, 'http') || hasProxy(environment, 'https')

  let output = systemProxyOutput
  if (output === undefined) {
    try {
      output = execFileSync('/usr/sbin/scutil', ['--proxy'], {
        encoding: 'utf8',
        stdio: ['ignore', 'pipe', 'ignore'],
        timeout: 2_000,
      })
    } catch {
      if (proxyAlreadyConfigured) withLocalProxyBypass(environment)
      return environment
    }
  }

  const proxies = parseMacosSystemProxies(output)
  let added = false
  if (proxies.http && !hasProxy(environment, 'http')) {
    environment.HTTP_PROXY = proxies.http
    added = true
  }
  if (proxies.https && !hasProxy(environment, 'https')) {
    environment.HTTPS_PROXY = proxies.https
    added = true
  }
  if (added || proxyAlreadyConfigured) withLocalProxyBypass(environment)
  return environment
}
