import { spawnSync } from 'node:child_process'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { resolveGoCommand } from '../tools/toolchains/go-toolchain.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const args = process.argv.slice(2)
if (!args.length) throw new Error('Usage: node scripts/run-go.mjs <go arguments...>')

const result = spawnSync(resolveGoCommand('go', root), args, {
  cwd: root,
  env: process.env,
  stdio: 'inherit',
})
if (result.error?.code === 'ENOENT') {
  console.error('Missing required command: go. From the repository directory, run: npm run setup')
  process.exit(1)
}
if (result.error) throw result.error
process.exit(result.status ?? 1)
