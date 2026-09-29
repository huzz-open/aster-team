import { randomBytes } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { mkdtempSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve, basename } from 'node:path'
import { setTimeout } from 'node:timers/promises'

const name = `aster-customer-db-test-${randomBytes(6).toString('hex')}`
const password = randomBytes(24).toString('hex')
const docker = (...args) => {
  const result = spawnSync('docker', args, { encoding: 'utf8', windowsHide: true })
  if (result.error || result.status !== 0) throw new Error(`Isolated MariaDB command failed: docker ${args[0]}`)
  return result.stdout.trim()
}
const certificates = mkdtempSync(join(tmpdir(), 'aster-customer-db-tls-'))
let created = false
try {
  docker('run', '--rm', '-v', `${certificates}:/certs`, '--entrypoint', 'openssl', 'mariadb:11.8.6',
    'req', '-x509', '-newkey', 'rsa:2048', '-nodes', '-days', '1', '-subj', '/CN=aster-test-ca',
    '-keyout', '/certs/ca.key', '-out', '/certs/ca.pem')
  docker('run', '--rm', '-v', `${certificates}:/certs`, '--entrypoint', 'openssl', 'mariadb:11.8.6',
    'req', '-newkey', 'rsa:2048', '-nodes', '-subj', '/CN=localhost', '-addext', 'subjectAltName=IP:127.0.0.1',
    '-keyout', '/certs/server.key', '-out', '/certs/server.csr')
  docker('run', '--rm', '-v', `${certificates}:/certs`, '--entrypoint', 'openssl', 'mariadb:11.8.6',
    'x509', '-req', '-in', '/certs/server.csr', '-CA', '/certs/ca.pem', '-CAkey', '/certs/ca.key',
    '-CAcreateserial', '-copy_extensions', 'copy', '-days', '1', '-out', '/certs/server.pem')
  docker('run', '--rm', '-v', `${certificates}:/certs`, '--entrypoint', 'chmod', 'mariadb:11.8.6', '644', '/certs/server.key')
  docker('run', '-d', '--name', name, '-p', '127.0.0.1::3306', '-e', `MARIADB_ROOT_PASSWORD=${password}`, '-e', 'MARIADB_DATABASE=aster_customer_fixture', '-e', 'MARIADB_USER=aster_team', '-e', `MARIADB_PASSWORD=${password}`, '-v', `${certificates}:/certs:ro`, 'mariadb:11.8.6', '--ssl-ca=/certs/ca.pem', '--ssl-cert=/certs/server.pem', '--ssl-key=/certs/server.key')
  created = true
  const port = docker('port', name, '3306/tcp').split(':').at(-1)
  let ready = false
  for (let attempt = 0; attempt < 40; attempt++) {
    const result = spawnSync('docker', ['exec', name, 'healthcheck.sh', '--connect', '--innodb_initialized'], { stdio: 'ignore', windowsHide: true })
    if (result.status === 0) { ready = true; break }
    await setTimeout(1000)
  }
  if (!ready) {
    const logs = spawnSync('docker', ['logs', '--tail', '30', name], { encoding: 'utf8', windowsHide: true })
    process.stderr.write(`${logs.stdout || ''}${logs.stderr || ''}`.split('\n').filter(line => /SSL|ERROR|certificate|Permission denied/.test(line)).join('\n') + '\n')
    throw new Error('Isolated MariaDB did not become ready')
  }
  docker('exec', '-e', `MYSQL_PWD=${password}`, name, 'mariadb', '-uroot', '-e', "REVOKE ALL PRIVILEGES, GRANT OPTION FROM 'aster_team'@'%'; GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX, REFERENCES ON aster_customer_fixture.* TO 'aster_team'@'%';")
  docker('exec', '-e', `MYSQL_PWD=${password}`, name, 'mariadb', '-uroot', '-e', `CREATE USER 'aster_readonly'@'%' IDENTIFIED BY '${password}'; GRANT SELECT ON aster_customer_fixture.* TO 'aster_readonly'@'%';`)
  docker('exec', '-e', `MYSQL_PWD=${password}`, name, 'mariadb', '-uroot', '-e', "CREATE DATABASE aster_migration_recovery_fixture; GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX, REFERENCES ON aster_migration_recovery_fixture.* TO 'aster_team'@'%';")
  docker('exec', '-e', `MYSQL_PWD=${password}`, name, 'mariadb', '-uroot', '-e', "CREATE DATABASE aster_online_migration_fixture; GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX, REFERENCES ON aster_online_migration_fixture.* TO 'aster_team'@'%';")
  const result = spawnSync('cargo', ['test', '--locked', '-p', 'aster-storage', '--no-default-features', '--features', 'mariadb', '--jobs', '1', 'installed_database_is_bound_and_migration_cancellation_releases_session_lock', '--', '--nocapture'], {
    stdio: 'inherit', windowsHide: true,
    env: { ...process.env, CARGO_PROFILE_TEST_DEBUG: '0', ASTER_CUSTOMER_TEST_DB_PORT: port, ASTER_CUSTOMER_TEST_DB_READONLY_USER: 'aster_readonly', ASTER_CUSTOMER_TEST_DB_PASSWORD: password, ASTER_CUSTOMER_TEST_DB_CA: join(certificates, 'ca.pem') },
  })
  if (result.error) throw new Error('Cannot launch Customer external database tests')
  process.exitCode = result.status ?? 1
  if (result.status === 0) {
    docker('exec', '-e', `MYSQL_PWD=${password}`, name, 'mariadb', '-uroot', '-e', "CREATE DATABASE aster_control_test_settlement; GRANT SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX, REFERENCES ON aster_control_test_settlement.* TO 'aster_team'@'%';")
    const settlement = spawnSync('cargo', ['test', '--locked', '-p', 'aster-control', '--features', 'sqlite-dev,mariadb', '--jobs', '1', '--lib', 'model_authorization::tests::durable_settlement::mariadb_settlement_recovers_failure_and_audit_across_two_controls', '--', '--exact', '--ignored'], {
      stdio: 'inherit', windowsHide: true,
      env: { ...process.env, CARGO_PROFILE_TEST_DEBUG: '0', ASTER_CUSTOMER_TEST_DB_PORT: port, ASTER_CUSTOMER_TEST_DB_PASSWORD: password, ASTER_CUSTOMER_TEST_DB_CA: join(certificates, 'ca.pem') },
    })
    if (settlement.error) throw new Error('Cannot launch Customer MariaDB settlement tests')
    process.exitCode = settlement.status ?? 1
  }
} finally {
  if (created) docker('rm', '-f', '-v', name)
  if (resolve(certificates).startsWith(`${resolve(tmpdir())}\\`) || resolve(certificates).startsWith(`${resolve(tmpdir())}/`)) {
    if (!basename(certificates).startsWith('aster-customer-db-tls-')) throw new Error('Unexpected certificate directory')
    rmSync(certificates, { recursive: true, force: true })
  } else throw new Error('Refusing cleanup outside the temporary directory')
}
