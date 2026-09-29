import { spawn } from 'node:child_process'
import { readFile } from 'node:fs/promises'
import { fileURLToPath } from 'node:url'

const root = fileURLToPath(new URL('../../', import.meta.url))
const settings = Object.fromEntries((await readFile(new URL('../../dist/commercial-validation/operations-ui.env', import.meta.url), 'utf8')).trim().split(/\r?\n/).map(line => {
  const split = line.indexOf('='); return [line.slice(0, split), line.slice(split + 1)]
}))
for (const [key, expected] of Object.entries({ ASTER_OPERATIONS_ADDR: '127.0.0.1:26390', ASTER_OPERATIONS_DB_HOST: '127.0.0.1', ASTER_OPERATIONS_DB_PORT: '26216', ASTER_OPERATIONS_DB_NAME: 'aster_commercial_ui_test', ASTER_OPERATIONS_GITHUB_ENABLED: 'false', ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED: 'false' })) {
  if (settings[key] !== expected) throw new Error(`Disposable API configuration mismatch: ${key}`)
}
const env = Object.fromEntries(Object.entries(process.env).filter(([key]) => !key.startsWith('ASTER_OPERATIONS_')))
if (settings.ASTER_OPERATIONS_PUBLICATION_LOCAL_ORIGIN !== 'http://127.0.0.1:26394' || settings.ASTER_OPERATIONS_PUBLICATION_PRODUCTION_ORIGIN !== '') throw new Error('Publication verification must use only the isolated loopback target')
if (settings.ASTER_OPERATIONS_QUOTATION_ENVIRONMENT !== 'local') throw new Error('Quotation orders must use only the isolated local sales channel')
if (settings.ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT !== 'local') throw new Error('Paid fulfillment must use only the isolated local environment')
const binary = fileURLToPath(new URL('../../dist/commercial-validation/operations-api.exe', import.meta.url))
const child = spawn(binary, ['--env-file', 'dist/commercial-validation/operations-ui.env'], { cwd: root, env: { ...env, ...settings }, stdio: 'inherit', windowsHide: true })
for (const signal of ['SIGINT', 'SIGTERM']) process.on(signal, () => child.kill(signal))
child.once('error', error => { console.error(error.message); process.exitCode = 1 })
child.once('exit', code => { process.exitCode = code ?? 1 })
