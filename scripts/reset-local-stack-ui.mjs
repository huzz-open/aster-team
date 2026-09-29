import { existsSync, readFileSync, rmSync } from 'node:fs'
import { isAbsolute, relative, resolve } from 'node:path'
import { parseEnv } from 'node:util'
import mysql from 'mysql2/promise'
import { dropLocalDatabase, localDatabasePlans } from './local-database-provision.mjs'

const root = resolve('.')
const confirmed = process.argv.slice(2)
if (process.env.ASTER_LOCAL_UI_CONFIRMED !== 'true' || confirmed.length !== 1 || confirmed[0] !== '--confirmed-by-local-ui') {
  throw new Error('此命令只能在本地开发 UI 明确确认后执行')
}
const envPath = resolve(root, '.env')
if (!existsSync(envPath)) throw new Error('缺少 .env，无法确认要清理的数据库')
const environment = parseEnv(readFileSync(envPath, 'utf8'))
const plans = localDatabasePlans(environment)
for (const plan of plans) {
  console.log(`正在删除 ${plan.label} 数据库 ${plan.host}:${plan.port}/${plan.database} 及专用账号 ${plan.serviceUser}@${plan.serviceHost}…`)
  await dropLocalDatabase(plan, mysql.createConnection)
}

function inside(base, target) {
  const value = relative(base, target)
  return value !== '' && !value.startsWith('..') && !isAbsolute(value)
}

for (const target of [resolve(root, 'data/local'), resolve(root, 'data/control/license'), resolve(root, 'data/runner')]) {
  if (!inside(root, target)) throw new Error(`拒绝清理工作区外路径: ${target}`)
  try {
    rmSync(target, { recursive: true, force: true, maxRetries: 12, retryDelay: 150 })
  } catch (error) {
    if (error?.code === 'EBUSY' || error?.code === 'EPERM') {
      throw new Error(`无法清理 ${error.path || target}：文件仍被后台进程占用。请停止相关 Aster/Node 进程后重试。`, { cause: error })
    }
    throw error
  }
  console.log(`已清理 ${target}`)
}
console.log('本地环境已恢复为未初始化状态；.env 数据库管理员配置已保留。')
