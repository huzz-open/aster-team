import { existsSync, readFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { parseEnv } from 'node:util'
import { fileURLToPath } from 'node:url'
import { localRunnerProxyEnvironment } from './local-system-proxy.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const envFile = resolve(root, 'data/local/customer.env')
if (!existsSync(envFile)) throw new Error('缺少 data/local/customer.env，请先执行 npm run setup:local')
const environment = localRunnerProxyEnvironment({
  ...process.env,
  ...parseEnv(readFileSync(envFile, 'utf8')),
})
if (process.env.ASTER_RUNNER_CONTROL_WSS) environment.ASTER_RUNNER_CONTROL_WSS = process.env.ASTER_RUNNER_CONTROL_WSS
const identity = resolve(root, environment.ASTER_RUNNER_IDENTITY_PATH || 'data/runner/identity.json')
const taskKeys = resolve(root, environment.ASTER_RUNNER_TASK_KEYS_PATH || 'data/runner/task-keys.json')
if (!existsSync(identity) || !existsSync(taskKeys)) throw new Error('Runner 尚未注册；请先在本地开发控制台完成“本地快速授权”，或在 Customer Admin 手工注册')

const result = spawnSync('cargo', [
  'run', '-p', 'aster-runner', '--features', 'local-demo', '--', 'serve',
  '--control-wss', environment.ASTER_RUNNER_CONTROL_WSS || 'ws://127.0.0.1:11080/api/runner/channel',
  '--identity-file', identity, '--task-keys-file', taskKeys,
  '--allowed-upstream-host', 'api.openai.com', '--allowed-upstream-host', 'auth.openai.com',
  '--allowed-upstream-host', 'api.anthropic.com', '--allowed-upstream-host', 'chatgpt.com',
], { cwd: root, env: environment, stdio: 'inherit', shell: process.platform === 'win32' })
if (result.error) throw result.error
process.exit(result.status ?? 1)
