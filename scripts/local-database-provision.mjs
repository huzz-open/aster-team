const databasePrivileges = [
  'SELECT', 'INSERT', 'UPDATE', 'DELETE',
  'CREATE', 'ALTER', 'DROP', 'INDEX', 'REFERENCES',
  'CREATE VIEW', 'SHOW VIEW', 'TRIGGER',
]

function required(value, name) {
  const result = String(value || '').trim()
  if (!result || /[\r\n]/.test(result)) throw new Error(`${name} 不能为空或包含换行符`)
  return result
}

function port(value, name) {
  const result = Number(required(value, name))
  if (!Number.isSafeInteger(result) || result < 1 || result > 65_535) throw new Error(`${name} 无效`)
  return result
}

function identifier(value, name) {
  const result = required(value, name)
  if (!/^[A-Za-z0-9_]+$/.test(result)) throw new Error(`${name} 只能包含字母、数字和下划线`)
  return result
}

function accountHost(value, name) {
  const result = required(value, name)
  if (!/^[A-Za-z0-9_.:%-]+$/.test(result)) throw new Error(`${name} 包含不支持的字符`)
  return result
}

function boolean(value, name) {
  const result = String(value || 'false').trim().toLowerCase()
  if (!['true', 'false'].includes(result)) throw new Error(`${name} 必须是 true 或 false`)
  return result === 'true'
}

function safeError(error, secrets = []) {
  let message = error instanceof Error ? error.message : String(error)
  for (const secret of secrets.filter(Boolean)) message = message.replaceAll(secret, '[REDACTED]')
  return message
}

function plan(environment, scope, runtimePrefix, label, defaults) {
  return {
    label,
    runtimePrefix,
    host: required(environment[`${scope}_HOST`], `${scope}_HOST`),
    port: port(environment[`${scope}_PORT`], `${scope}_PORT`),
    database: identifier(environment[`${scope}_NAME`], `${scope}_NAME`),
    adminUser: required(environment[`${scope}_ADMIN_USER`], `${scope}_ADMIN_USER`),
    adminPassword: required(environment[`${scope}_ADMIN_PASSWORD`], `${scope}_ADMIN_PASSWORD`),
    adminTLS: boolean(environment[`${scope}_ADMIN_TLS`], `${scope}_ADMIN_TLS`),
    serviceUser: identifier(environment[`${scope}_SERVICE_USER`] || defaults.user, `${scope}_SERVICE_USER`),
    serviceHost: accountHost(environment[`${scope}_SERVICE_HOST`] || defaults.host, `${scope}_SERVICE_HOST`),
  }
}

export function localDatabasePlans(environment) {
  for (const scope of ['ASTER_CUSTOMER_DB', 'ASTER_OPERATIONS_DB']) {
    if (!environment[`${scope}_ADMIN_USER`] && environment[`${scope}_USER`]) {
      throw new Error(`检测到旧版 ${scope}_USER/PASSWORD 配置；请按 .env.example 改为 ${scope}_ADMIN_USER/ADMIN_PASSWORD`)
    }
  }
  const plans = [
    plan(environment, 'ASTER_CUSTOMER_DB', 'ASTER_CONTROL_DB', 'Customer', { user: 'aster_customer', host: '127.0.0.1' }),
    plan(environment, 'ASTER_OPERATIONS_DB', 'ASTER_OPERATIONS_DB', 'Operations', { user: 'aster_operations', host: '127.0.0.1' }),
  ]

  const databases = new Set()
  const accounts = new Set()
  for (const item of plans) {
    const server = `${item.host.toLowerCase()}:${item.port}`
    const databaseKey = `${server}/${item.database.toLowerCase()}`
    if (databases.has(databaseKey)) throw new Error('Customer 与 Operations 必须使用不同数据库')
    databases.add(databaseKey)

    const accountKey = `${server}/${item.serviceUser.toLowerCase()}@${item.serviceHost.toLowerCase()}`
    if (accounts.has(accountKey)) throw new Error('Customer 与 Operations 不能复用同一个数据库服务账号')
    accounts.add(accountKey)
  }
  return plans
}

async function adminConnection(plan, connect) {
  try {
    return await connect({
      host: plan.host,
      port: plan.port,
      user: plan.adminUser,
      password: plan.adminPassword,
      ssl: plan.adminTLS ? {} : undefined,
      multipleStatements: false,
    })
  } catch (error) {
    throw new Error(`${plan.label} 数据库管理员连接失败: ${safeError(error, [plan.adminPassword])}`)
  }
}

export async function inspectLocalDatabases(plans, connect) {
  const results = []
  for (const plan of plans) {
    const connection = await adminConnection(plan, connect)
    try {
      const [rows] = await connection.query(
        'SELECT SCHEMA_NAME FROM information_schema.SCHEMATA WHERE SCHEMA_NAME = ?',
        [plan.database],
      )
      results.push({ plan, exists: rows.length > 0 })
    } finally {
      await connection.end()
    }
  }
  return results
}

export async function provisionLocalDatabase(plan, servicePassword, connect) {
  required(servicePassword, `${plan.label} 随机数据库密码`)
  const connection = await adminConnection(plan, connect)
  try {
    const database = connection.escapeId(plan.database)
    const account = `${connection.escape(plan.serviceUser)}@${connection.escape(plan.serviceHost)}`
    const password = connection.escape(servicePassword)

    await connection.query(`DROP DATABASE IF EXISTS ${database}`)
    await connection.query(`CREATE DATABASE ${database} CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci`)
    await connection.query(`CREATE USER IF NOT EXISTS ${account} IDENTIFIED BY ${password}`)
    await connection.query(`ALTER USER ${account} IDENTIFIED BY ${password}`)
    await connection.query(`GRANT ${databasePrivileges.join(', ')} ON ${database}.* TO ${account}`)
  } catch (error) {
    throw new Error(`${plan.label} 数据库或服务账号创建失败: ${safeError(error, [plan.adminPassword, servicePassword])}`)
  } finally {
    await connection.end()
  }
}

export async function dropLocalDatabase(plan, connect) {
  const connection = await adminConnection(plan, connect)
  try {
    const database = connection.escapeId(plan.database)
    const account = `${connection.escape(plan.serviceUser)}@${connection.escape(plan.serviceHost)}`
    await connection.query(`DROP DATABASE IF EXISTS ${database}`)
    await connection.query(`DROP USER IF EXISTS ${account}`)
  } catch (error) {
    throw new Error(`${plan.label} 数据库或服务账号删除失败: ${safeError(error, [plan.adminPassword])}`)
  } finally {
    await connection.end()
  }
}

export function runtimeDatabaseEnvironment(plan, password) {
  return {
    [`${plan.runtimePrefix}_HOST`]: plan.host,
    [`${plan.runtimePrefix}_PORT`]: String(plan.port),
    [`${plan.runtimePrefix}_NAME`]: plan.database,
    [`${plan.runtimePrefix}_USER`]: plan.serviceUser,
    [`${plan.runtimePrefix}_PASSWORD`]: required(password, `${plan.label} 随机数据库密码`),
  }
}
