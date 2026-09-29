import { spawnSync } from 'node:child_process'
import { resolve } from 'node:path'

const script = resolve('scripts/local_dev_manager.py')
const cliArguments = process.argv.slice(2)
const managerArguments = cliArguments.includes('--check')
  ? cliArguments
  : ['--select-environment', ...cliArguments]
const candidates = process.platform === 'win32'
  ? [['py', '-3'], ['python']]
  : [['python3'], ['python']]

for (const [command, ...prefix] of candidates) {
  const result = spawnSync(command, [...prefix, script, ...managerArguments], { cwd: process.cwd(), stdio: 'inherit' })
  if (result.error?.code === 'ENOENT') continue
  if (result.error) throw result.error
  process.exit(result.status ?? 1)
}

console.error('未找到 Python 3。请安装 Python 3（包含 tkinter）后重试。')
process.exit(1)
