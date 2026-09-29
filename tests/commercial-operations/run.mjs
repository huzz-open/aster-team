import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { join } from 'node:path'

const root = fileURLToPath(new URL('../../', import.meta.url))
const target = join(root, 'target')
const built = spawnSync('cargo', ['build', '--locked', '-p', 'aster-license-core', '--example', 'verify_issued_license', '-j', '1'], {
  cwd: root, stdio: 'inherit', windowsHide: true, timeout: 180_000,
  env: { ...process.env, CARGO_TARGET_DIR: target },
})
if (built.error || built.status !== 0) {
  if (built.error) console.error(built.error.message)
  process.exit(built.status ?? 1)
}
const result = spawnSync(process.execPath, [join(root, 'node_modules/@playwright/test/cli.js'), 'test', '--config', './tests/commercial-operations/playwright.config.mjs', ...process.argv.slice(2)], {
  cwd: root, stdio: 'inherit', windowsHide: true,
  env: { ...process.env, ASTER_COMMERCIAL_RUST_VERIFIER: join(target, 'debug/examples', `verify_issued_license${process.platform === 'win32' ? '.exe' : ''}`) },
})
if (result.error) console.error(result.error.message)
process.exitCode = result.status ?? 1
