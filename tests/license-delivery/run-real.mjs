import { spawnSync } from 'node:child_process'
import { createRequire } from 'node:module'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

const root = fileURLToPath(new URL('../../', import.meta.url))
const require = createRequire(import.meta.url)
function run(command, args, options = {}) {
  const result = spawnSync(command, args, { cwd: root, stdio: 'inherit', windowsHide: true, ...options })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status ?? result.signal}`)
  return result.stdout
}
const downloads = process.argv.includes('--downloads')
const playwrightArgs = process.argv.slice(2).filter(argument => argument !== '--downloads')
const artifact = join(root, 'dist/license-downloads/asterctl.exe')
if (downloads) {
  if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Download acceptance requires native Windows x64')
  run(process.execPath, [join(root, 'scripts/build-asterctl-windows.mjs'), `--output=${artifact}`])
}
for (const workspace of ['@aster/admin', '@aster/member']) {
  run(process.execPath, [process.env.npm_execpath, 'run', 'build', '--workspace', workspace])
}
run('cargo', ['build', '--locked', '-p', 'aster-control', '--no-default-features', '--features', 'local-demo,sqlite-dev', '--bin', 'license_browser_fixture', '-j', '1'])
const metadata = JSON.parse(run('cargo', ['metadata', '--locked', '--no-deps', '--format-version', '1'], { stdio: ['ignore', 'pipe', 'inherit'], encoding: 'utf8' }))
const binary = join(metadata.target_directory, 'debug', `license_browser_fixture${process.platform === 'win32' ? '.exe' : ''}`)
run(process.execPath, [require.resolve('@playwright/test/cli'), 'test', '--config', `./tests/license-delivery/${downloads ? 'downloads' : 'real'}.playwright.config.mjs`, ...playwrightArgs], {
  env: { ...process.env, ASTER_LICENSE_BROWSER_FIXTURE: binary, ...(downloads ? { ASTER_LICENSE_BROWSER_ASTERCTL: artifact } : {}) },
})
