import { randomBytes } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { setTimeout } from 'node:timers/promises'

const name = `aster-upgrade-test-${randomBytes(6).toString('hex')}`
const password = randomBytes(24).toString('hex')
const docker = (...args) => {
  const result = spawnSync('docker', args, { encoding: 'utf8', windowsHide: true })
  if (result.error || result.status !== 0) throw new Error(`Isolated MariaDB command failed: docker ${args[0]}`)
  return result.stdout.trim()
}
let created = false
try {
  docker('run', '-d', '--name', name, '-p', '127.0.0.1::3306', '-e', `MARIADB_ROOT_PASSWORD=${password}`, '-e', 'MARIADB_DATABASE=aster_upgrade_fixture', 'mariadb:11.8.6')
  created = true
  const port = docker('port', name, '3306/tcp').split(':').at(-1)
  let ready = false
  for (let attempt = 0; attempt < 40; attempt++) {
    const result = spawnSync('docker', ['exec', name, 'healthcheck.sh', '--connect', '--innodb_initialized'], { stdio: 'ignore', windowsHide: true })
    if (result.status === 0) { ready = true; break }
    await setTimeout(1000)
  }
  if (!ready) throw new Error('Isolated MariaDB did not become ready')
  const result = spawnSync(process.execPath, ['scripts/run-go.mjs', 'test', './operations/backend/internal/adapters/mariadb', '-run', 'TestEnvironmentUpgradeDatabaseFencesRecoveryAndPreservesEvidence', '-count=1'], {
    stdio: 'inherit', windowsHide: true,
    env: { ...process.env, ASTER_OPERATIONS_TEST_DB_DSN: `root:${password}@tcp(127.0.0.1:${port})/aster_upgrade_fixture?parseTime=true` },
  })
  if (result.error) throw new Error('Cannot launch environment upgrade database tests')
  process.exitCode = result.status ?? 1
} finally {
  if (created) docker('rm', '-f', name)
}
