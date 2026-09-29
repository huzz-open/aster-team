import { randomBytes } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { createServer as createNetServer } from 'node:net'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { createSystemCommandRunner } from './command-runner.mjs'
import { createTestCandidate, verifyCandidateManifest } from './release-candidate.mjs'
import { redactDiagnostic, scanDiagnosticDirectory, writeDiagnostic } from './sanitize-diagnostics.mjs'
import { resolveCiImage } from '../../scripts/ci/ci-images.mjs'
import { verifyArchive } from '../../scripts/ci/verify-static-archive.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const runtimeImage = resolveCiImage('ubuntu-22.04', 'runtime').image
const { bashPath, environment: commandEnvironment, executable, run, runNpm } = createSystemCommandRunner(root)
commandEnvironment.ASTER_E2E_RUNTIME_IMAGE = runtimeImage
const composeFile = join(root, 'tests', 'system-e2e', 'docker-compose.yml')
const version = JSON.parse(await readFile(join(root, 'package.json'), 'utf8')).version
const args = new Set(process.argv.slice(2))
const keep = args.has('--keep')
const headed = args.has('--headed')
const reuseRunID = [...args].find(value => value.startsWith('--reuse-run='))?.slice(12) || ''
const upstreamMode = [...args].find(value => value.startsWith('--upstream='))?.slice(11) || 'fake'
const candidateManifestArgument = [...args].find(value => value.startsWith('--candidate-manifest='))?.slice(21) || ''
if (!['fake', 'live'].includes(upstreamMode)) throw new Error('--upstream must be fake or live')
if (upstreamMode === 'live' && !headed) throw new Error('live upstream authorization requires --headed')
if (upstreamMode === 'live' && keep) throw new Error('live upstream authorization forbids --keep so credentials and browser state are always destroyed')
if (reuseRunID && !/^\d{14}-[a-f0-9]{8}$/.test(reuseRunID)) throw new Error('--reuse-run must be a retained run ID created by this runner')
if (reuseRunID && candidateManifestArgument) throw new Error('--reuse-run and --candidate-manifest are mutually exclusive')
if (keep && candidateManifestArgument) throw new Error('--candidate-manifest does not support --keep; external candidate inputs are not retained for reuse')

const runID = reuseRunID || `${new Date().toISOString().replaceAll(/[-:.TZ]/g, '').slice(0, 14)}-${randomBytes(4).toString('hex')}`
const stateRoot = join(root, 'target', 'system-e2e')
const runRoot = join(stateRoot, 'runs', runID)
const liveTestStatusFile = join(runRoot, 'live-test-status.json')
const v2SignersFile = join(runRoot, 'v2-signers.json')
const exportedCustomerRoot = join(runRoot, 'customer-bundle')
const composeEnvFile = join(runRoot, 'compose.env')
const operationsEnvFile = join(runRoot, 'operations.env')
const runtimeFile = join(runRoot, 'runtime.json')
const generatedCandidateManifestFile = join(runRoot, 'candidate-manifest.json')
const diagnosticsRoot = join(runRoot, 'diagnostics')
const projectName = `aster-e2e-${runID.toLowerCase()}`
let operationsEmail = `operations-${runID}@example.test`
let operationsPassword = `Operations-${randomBytes(18).toString('base64url')}!`
const ownerEmail = `owner-${runID}@example.test`
const ownerPassword = `Owner-${randomBytes(18).toString('base64url')}!`
let upstreamEmail = `upstream-${runID}@example.test`
let upstreamPassword = `Upstream-${randomBytes(18).toString('base64url')}!`

const variants = [
  { id: 'json', service: 'customer-json' },
  { id: 'qr-png', service: 'customer-qr' },
  { id: 'terminal-screenshot', service: 'customer-terminal' },
]
let operationsPort
let upstreamPort
let candidateInfo
let fakeUpstreamControlToken
let paidSignerID
let diagnosticCanary = `ASTER_E2E_CANARY_${randomBytes(18).toString('base64url')}`
const heldPortServers = []

const docker = executable('docker')

function runPlaywright(commandArgs, options = {}) {
  return run(process.execPath, [join(root, 'node_modules', '@playwright', 'test', 'cli.js'), ...commandArgs], options)
}

async function holdAvailablePort() {
  const server = createNetServer()
  await new Promise((accept, reject) => {
    server.once('error', reject)
    server.listen(0, '127.0.0.1', accept)
  })
  const address = server.address()
  if (!address || typeof address === 'string') throw new Error('failed to reserve an isolated host port')
  heldPortServers.push(server)
  return address.port
}

async function allocatePorts() {
  operationsPort = await holdAvailablePort()
  upstreamPort = await holdAvailablePort()
  for (const variant of variants) {
    variant.api = await holdAvailablePort()
    variant.member = await holdAvailablePort()
    variant.admin = await holdAvailablePort()
  }
}

async function releaseHeldPorts() {
  await Promise.all(heldPortServers.splice(0).map(server => new Promise(resolvePromise => server.close(resolvePromise))))
}

async function loadRetainedPorts() {
  const requiredString = (values, key) => {
    const value = values.get(key)
    if (typeof value !== 'string' || !value) throw new Error(`retained system E2E value is missing: ${key}`)
    return value
  }
  const source = await readFile(composeEnvFile, 'utf8')
  const values = new Map(source.split(/\r?\n/).filter(Boolean).map(line => {
    const separator = line.indexOf('=')
    return [line.slice(0, separator), JSON.parse(line.slice(separator + 1))]
  }))
  operationsPort = Number(values.get('ASTER_E2E_OPERATIONS_PORT'))
  upstreamPort = Number(values.get('ASTER_E2E_UPSTREAM_PORT'))
  for (const variant of variants) {
    const prefix = variant.id === 'qr-png' ? 'QR' : variant.id === 'terminal-screenshot' ? 'TERMINAL' : 'JSON'
    variant.api = Number(values.get(`ASTER_E2E_${prefix}_API_PORT`))
    variant.member = Number(values.get(`ASTER_E2E_${prefix}_MEMBER_PORT`))
    variant.admin = Number(values.get(`ASTER_E2E_${prefix}_ADMIN_PORT`))
  }
  fakeUpstreamControlToken = values.get('ASTER_E2E_FAKE_CONTROL_TOKEN')
  upstreamEmail = requiredString(values, 'ASTER_E2E_UPSTREAM_EMAIL')
  upstreamPassword = requiredString(values, 'ASTER_E2E_UPSTREAM_PASSWORD')
  diagnosticCanary = requiredString(values, 'ASTER_E2E_DIAGNOSTIC_CANARY')

  const operationsSource = await readFile(operationsEnvFile, 'utf8')
  const operationsValues = new Map(operationsSource.split(/\r?\n/).filter(Boolean).map(line => {
    const separator = line.indexOf('=')
    return [line.slice(0, separator), line.slice(separator + 1)]
  }))
  operationsEmail = requiredString(operationsValues, 'ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL')
  operationsPassword = requiredString(operationsValues, 'ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD')
  paidSignerID = paidSignerProfile(requiredString(operationsValues, 'ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON')).key_id
}

function paidSignerProfile(raw) {
  const signers = JSON.parse(raw)
  if (!Array.isArray(signers) || signers.length !== 1 || !signers[0].key_id || !signers[0].private_key_pkcs8) throw new Error('system E2E requires exactly one isolated paid v2 signer')
  return signers[0]
}

function compose(...commandArgs) {
  return run(docker, ['compose', '--env-file', composeEnvFile, '--file', composeFile, '--project-name', projectName, ...commandArgs])
}

function composeCapture(...commandArgs) {
  return run(docker, ['compose', '--env-file', composeEnvFile, '--file', composeFile, '--project-name', projectName, ...commandArgs], { capture: true })
}

async function waitFor(description, probe, timeout = 180_000) {
  const deadline = Date.now() + timeout
  let lastError
  while (Date.now() < deadline) {
    try {
      if (await probe()) return
    } catch (error) { lastError = error }
    await new Promise(resolvePromise => setTimeout(resolvePromise, 1_000))
  }
  throw new Error(`${description} did not become ready${lastError ? `: ${lastError.message}` : ''}`)
}

async function waitHTTP(url) {
  await waitFor(url, async () => {
    const response = await fetch(url, { signal: AbortSignal.timeout(3_000) })
    return response.ok
  })
}

function customerInstallScript() {
  return String.raw`
service="$1"
upstream_mode="$4"
machine_hex="$(printf '%s' "$service" | sha256sum | awk '{print substr($1,1,32)}')"
machine_uuid="$(printf '%s' "$machine_hex" | sed -E 's/(.{8})(.{4})(.{4})(.{4})(.{12})/\1-\2-\3-\4-\5/')"
printf '%s\n' "$machine_hex" >/etc/machine-id
mkdir -p /sys/class/dmi/id
printf '%s\n' "$machine_uuid" >/sys/class/dmi/id/product_uuid
lan_ip="$(ip -4 route get 192.0.2.1 | awk '{for(i=1;i<=NF;i++) if($i=="src"){print $(i+1);exit}}')"
test -n "$lan_ip"
printf '%s\n' "$lan_ip" >/e2e/access-host
printf '%s' "$2" >/e2e/owner.password
chmod 0600 /e2e/owner.password
trap 'rm -f /e2e/owner.password' EXIT
install_source="$(mktemp -d /tmp/aster-customer.XXXXXX)"
tar -xzf /workspace/customer-bundle.tar.gz -C "$install_source" --strip-components=1
"$install_source/init.sh" --install-root /srv/aster-team
/usr/local/bin/aster-team-cli install --unattended \
  --owner-email "$3" \
  --owner-password-file /e2e/owner.password \
  --install-local-runner \
  --access-host "$lan_ip" \
  --bind-address "$lan_ip"
if [ "$upstream_mode" = fake ]; then
  install -o root -g aster-runner -m 0640 /e2e-certs/ca.crt /srv/aster-team/config/runner/upstream-ca.pem
  printf '%s\n' 'ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE=/srv/aster-team/config/runner/upstream-ca.pem' >>/srv/aster-team/config/runner/runner.env
  systemctl restart aster-runner.service
  systemctl is-active --quiet aster-runner.service
  curl --fail --silent --show-error --cacert /srv/aster-team/config/runner/upstream-ca.pem https://auth.openai.com/healthz >/dev/null
fi
rm -f /e2e/request.json /e2e/request.qr.png /e2e/terminal.txt
cd /e2e
/usr/local/bin/aster-team-cli license request --output /e2e/request.json > /e2e/terminal.txt
# Only the disposable authorization request and its QR representation cross
# the container boundary to the unprivileged host Playwright process.
chmod 0644 /e2e/request.json /e2e/request.qr.png
/usr/local/bin/aster-team-cli doctor
`
}

async function prepareDirectories() {
  await mkdir(runRoot, { recursive: true })
  await mkdir(diagnosticsRoot, { recursive: true })
  const certificateRoot = join(runRoot, 'certs')
  await mkdir(certificateRoot, { recursive: true })
  await mkdir(join(runRoot, 'operations-artifacts'), { recursive: true })
  for (const variant of variants) await mkdir(join(runRoot, `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`), { recursive: true })
  if (upstreamMode !== 'fake') return
  const openssl = executable('openssl')
  const caKey = join(certificateRoot, 'ca.key')
  const caCertificate = join(certificateRoot, 'ca.crt')
  const serverKey = join(certificateRoot, 'server.key')
  const serverRequest = join(certificateRoot, 'server.csr')
  const serverExtension = join(certificateRoot, 'server.ext')
  const serverCertificate = join(certificateRoot, 'server.crt')
  await writeFile(serverExtension, 'subjectAltName=DNS:auth.openai.com,DNS:chatgpt.com,DNS:api.openai.com,IP:127.0.0.1\nextendedKeyUsage=serverAuth\n')
  run(openssl, ['req', '-x509', '-newkey', 'rsa:2048', '-sha256', '-nodes', '-days', '2', '-subj', '/CN=Aster E2E Test CA', '-keyout', caKey, '-out', caCertificate], { capture: true })
  run(openssl, ['req', '-newkey', 'rsa:2048', '-nodes', '-sha256', '-subj', '/CN=auth.openai.com', '-keyout', serverKey, '-out', serverRequest], { capture: true })
  run(openssl, ['x509', '-req', '-sha256', '-days', '2', '-in', serverRequest, '-CA', caCertificate, '-CAkey', caKey, '-CAcreateserial', '-extfile', serverExtension, '-out', serverCertificate], { capture: true })
}

async function buildArtifacts() {
  process.stdout.write('Building a newly test-signed Customer Linux package and validating it in the Linux lab...\n')
  run(executable('bash'), ['--noprofile', '--norc', 'scripts/ci/linux-lab.sh', 'quick', '--jobs', '2',
    '--export-v2-signers', bashPath(v2SignersFile), '--export-bundle', bashPath(exportedCustomerRoot)])
  process.stdout.write('Building the Operations Linux bundle...\n')
  runNpm(['run', 'build:operations'])
}

async function prepareCandidate() {
  if (candidateManifestArgument) {
    const manifestPath = resolve(root, candidateManifestArgument)
    candidateInfo = await verifyCandidateManifest(manifestPath, root)
    const v2Signers = process.env.ASTER_E2E_V2_SIGNERS_FILE
    if (!v2Signers || !existsSync(resolve(v2Signers))) throw new Error('candidate testing requires ASTER_E2E_V2_SIGNERS_FILE matching the candidate v2 trust scope')
    return {
      customerArchive: candidateInfo.resolvedArtifacts.get('customer-package'),
      operationsBundle: candidateInfo.resolvedArtifacts.get('operations'),
      v2SignersFile: resolve(v2Signers),
    }
  }
  await buildArtifacts()
  const commit = run('git', ['rev-parse', 'HEAD'], { capture: true }).trim()
  candidateInfo = await createTestCandidate({
    repositoryRoot: root,
    customerRoot: exportedCustomerRoot,
    outputPath: generatedCandidateManifestFile,
    version,
    commit,
    candidateID: `test-${runID}`,
  })
  return {
    customerArchive: candidateInfo.resolvedArtifacts.get('customer-package'),
    operationsBundle: candidateInfo.resolvedArtifacts.get('operations'),
    v2SignersFile,
  }
}

async function writeEnvironment({ customerArchive, operationsBundle, v2SignersFile: signerFile }) {
  const signers = JSON.stringify(JSON.parse(await readFile(signerFile, 'utf8')))
  paidSignerID = paidSignerProfile(signers).key_id
  const databasePassword = randomBytes(32).toString('base64url')
  const databaseRootPassword = randomBytes(32).toString('base64url')
  const customerReferenceSecret = randomBytes(32).toString('base64url')
  for (const required of [customerArchive, operationsBundle]) {
    if (!existsSync(required)) throw new Error(`candidate bundle is missing: ${required}`)
  }
  fakeUpstreamControlToken = randomBytes(32).toString('base64url')
  const env = {
    ASTER_E2E_RUNTIME_IMAGE: runtimeImage,
    ASTER_E2E_PROJECT_NAME: projectName,
    ASTER_E2E_RUN_ROOT: runRoot.replaceAll('\\', '/'),
    ASTER_E2E_CUSTOMER_ARCHIVE: customerArchive.replaceAll('\\', '/'),
    ASTER_E2E_OPERATIONS_BUNDLE: operationsBundle.replaceAll('\\', '/'),
    ASTER_E2E_DB_PASSWORD: databasePassword,
    ASTER_E2E_DB_ROOT_PASSWORD: databaseRootPassword,
    ASTER_E2E_UPSTREAM_EMAIL: upstreamEmail,
    ASTER_E2E_UPSTREAM_PASSWORD: upstreamPassword,
    ASTER_E2E_FAKE_CONTROL_TOKEN: fakeUpstreamControlToken,
    ASTER_E2E_DIAGNOSTIC_CANARY: diagnosticCanary,
    ASTER_E2E_OPERATIONS_PORT: operationsPort,
    ASTER_E2E_UPSTREAM_PORT: upstreamPort,
    ASTER_E2E_JSON_API_PORT: variants[0].api,
    ASTER_E2E_JSON_MEMBER_PORT: variants[0].member,
    ASTER_E2E_JSON_ADMIN_PORT: variants[0].admin,
    ASTER_E2E_QR_API_PORT: variants[1].api,
    ASTER_E2E_QR_MEMBER_PORT: variants[1].member,
    ASTER_E2E_QR_ADMIN_PORT: variants[1].admin,
    ASTER_E2E_TERMINAL_API_PORT: variants[2].api,
    ASTER_E2E_TERMINAL_MEMBER_PORT: variants[2].member,
    ASTER_E2E_TERMINAL_ADMIN_PORT: variants[2].admin,
  }
  await writeFile(composeEnvFile, `${Object.entries(env).map(([key, value]) => `${key}=${JSON.stringify(value)}`).join('\n')}\n`, { mode: 0o600 })
  const operations = [
    'ASTER_OPERATIONS_DB_DRIVER=mysql', 'ASTER_OPERATIONS_DB_HOST=mariadb', 'ASTER_OPERATIONS_DB_PORT=3306',
    'ASTER_OPERATIONS_DB_NAME=aster_operations', 'ASTER_OPERATIONS_DB_USER=aster_operations', `ASTER_OPERATIONS_DB_PASSWORD=${databasePassword}`,
    'ASTER_OPERATIONS_DB_TLS=false', 'ASTER_OPERATIONS_DB_CHARSET=utf8mb4', 'ASTER_OPERATIONS_DB_LOCATION=UTC',
    'ASTER_OPERATIONS_ADDR=0.0.0.0:12090', 'ASTER_OPERATIONS_AUTO_MIGRATE=false', 'ASTER_OPERATIONS_CREATE_DATABASE=false',
    `ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL=${operationsEmail}`, `ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD=${operationsPassword}`,
    'ASTER_OPERATIONS_SESSION_SECURE=false', 'ASTER_OPERATIONS_SESSION_TTL=12h',
    `ASTER_OPERATIONS_TRUSTED_ORIGINS=http://127.0.0.1:${operationsPort},http://localhost:${operationsPort}`,
    `ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON=${signers}`,
    'ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT=local',
    `ASTER_OPERATIONS_CUSTOMER_REF_SECRET=${customerReferenceSecret}`, 'ASTER_OPERATIONS_ARTIFACT_ROOT=/e2e/operations-artifacts',
    'ASTER_OPERATIONS_GITHUB_ENABLED=false', 'ASTER_OPERATIONS_GITHUB_PUBLISH_ENABLED=false',
  ]
  await writeFile(operationsEnvFile, `${operations.join('\n')}\n`, { mode: 0o600 })
  return { customerArchive, operationsBundle }
}

async function installCustomers() {
  if (upstreamMode === 'fake') await waitFor('fake upstream certificate', () => existsSync(join(runRoot, 'certs', 'ca.crt')))
  for (const variant of variants) {
    process.stdout.write(`Installing clean Customer environment for ${variant.id}...\n`)
    compose('exec', '-T', variant.service, 'bash', '-Eeuo', 'pipefail', '-c', customerInstallScript(), 'bash', variant.service, ownerPassword, ownerEmail, upstreamMode)
  }
}

async function writeRuntime() {
  const customerRelease = JSON.parse(await readFile(candidateInfo.resolvedArtifacts.get('customer-release-manifest'), 'utf8'))
  const runtime = {
    run_id: runID,
    upstream_mode: upstreamMode,
    paid_signer_id: paidSignerID,
    candidate: {
      id: candidateInfo.manifest.candidate_id,
      version: customerRelease.version,
      trust: candidateInfo.manifest.trust,
      manifest_sha256: candidateInfo.digest,
      test_matrix_version: candidateInfo.manifest.test_matrix_version,
    },
    operations: { url: `http://127.0.0.1:${operationsPort}`, email: operationsEmail, password: operationsPassword },
    owner: { email: ownerEmail, password: ownerPassword },
    upstream: {
      url: `https://127.0.0.1:${upstreamPort}`,
      email: upstreamEmail,
      password: upstreamPassword,
      control_token: upstreamMode === 'fake' ? fakeUpstreamControlToken : undefined,
    },
    customers: await Promise.all(variants.map(async variant => ({
      ...variant,
      access_host: (await readFile(join(runRoot, `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`, 'access-host'), 'utf8')).trim(),
      api_url: `http://127.0.0.1:${variant.api}`,
      member_url: `http://127.0.0.1:${variant.member}`,
      admin_url: `http://127.0.0.1:${variant.admin}`,
      request_json: join(runRoot, `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`, 'request.json'),
      request_qr_png: join(runRoot, `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`, 'request.qr.png'),
      terminal_output: join(runRoot, `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`, 'terminal.txt'),
    }))),
  }
  await writeFile(runtimeFile, `${JSON.stringify(runtime, null, 2)}\n`)
}

async function collectLogs() {
  try {
    const output = composeCapture('logs', '--no-color')
    await writeDiagnostic(join(diagnosticsRoot, 'docker.log'), output, { canary: diagnosticCanary })
  } catch {}
  for (const variant of variants) {
    try {
      const output = composeCapture('exec', '-T', variant.service, 'journalctl', '--no-pager', '-n', '500', '-u', 'aster-control@blue.service', '-u', 'aster-runner.service')
      await writeDiagnostic(join(diagnosticsRoot, `${variant.service}.journal.log`), output, { canary: diagnosticCanary })
    } catch {}
  }
  await scanDiagnosticDirectory(diagnosticsRoot, { canary: diagnosticCanary })
}

async function writeLiveResult(status, detail = '') {
  if (upstreamMode !== 'live' || !candidateInfo) return
  const safeDetail = redactDiagnostic(detail, { canary: diagnosticCanary })
  const result = {
    schema: 'aster.live-e2e-result.v1',
    status,
    candidate_id: candidateInfo.manifest.candidate_id,
    candidate_manifest_sha256: candidateInfo.digest,
    test_matrix_version: candidateInfo.manifest.test_matrix_version,
    completed_at: new Date().toISOString(),
    detail: safeDetail,
  }
  await writeFile(join(runRoot, 'live-result.json'), `${JSON.stringify(result, null, 2)}\n`)
  await writeFile(join(runRoot, 'live-checklist.md'), [
    '# Live OpenAI acceptance', '',
    `- Status: ${status}`,
    `- Candidate: ${result.candidate_id}`,
    `- Candidate manifest SHA-256: ${result.candidate_manifest_sha256}`,
    '- Human boundary: login, MFA, and risk-control only.',
    '- Automated checks: member, quota, Aster API key, text, streaming, image generation, image edit, usage, and audit.',
    safeDetail ? `- Detail: ${safeDetail}` : '',
  ].filter(Boolean).join('\n') + '\n')
}

async function liveFailureStatus() {
  if (upstreamMode !== 'live') return 'FAILED'
  try {
    const parsed = JSON.parse(await readFile(liveTestStatusFile, 'utf8'))
    if (['BLOCKED', 'FAILED', 'INCONCLUSIVE'].includes(parsed.status)) return parsed.status
  } catch {}
  return 'FAILED'
}

let environmentStarted = false
try {
  if (reuseRunID) {
    for (const required of [v2SignersFile, composeEnvFile, operationsEnvFile, generatedCandidateManifestFile]) {
      if (!existsSync(required)) throw new Error(`retained system E2E input is missing: ${required}`)
    }
    if (upstreamMode === 'fake' && !existsSync(join(runRoot, 'certs', 'ca.crt'))) throw new Error('retained fake-upstream CA is missing')
    await loadRetainedPorts()
    candidateInfo = await verifyCandidateManifest(generatedCandidateManifestFile, root)
    process.stdout.write(`Reusing packaged artifacts and ephemeral keys from retained run ${reuseRunID}; deployment state will still be rebuilt from scratch.\n`)
  } else {
    await prepareDirectories()
    await allocatePorts()
  }
  run(docker, ['version', '--format', 'Client={{.Client.Version}} Server={{.Server.Version}} OS={{.Server.Os}}'])
  if (!reuseRunID) {
    const artifacts = await prepareCandidate()
    await writeEnvironment(artifacts)
  }
  verifyArchive(candidateInfo.resolvedArtifacts.get('customer-package'))
  const baselineScript = (await readFile(join(root, 'scripts/ci/check-runtime-baseline.sh'), 'utf8')).replaceAll('\r\n', '\n')
  const checkBaselines = () => {
    for (const variant of variants) compose('exec', '-T', variant.service, 'sh', '-ec', baselineScript, 'baseline', 'ubuntu', '22.04')
  }
  await releaseHeldPorts()
  try { compose('down', '--volumes', '--remove-orphans') } catch {}
  if (upstreamMode === 'fake') {
    compose('up', '--detach', '--build')
  } else {
    compose('up', '--detach', '--build', 'mariadb', 'operations-api', 'operations-web', ...variants.map(variant => variant.service))
  }
  environmentStarted = true
  checkBaselines()
  await waitHTTP(`http://127.0.0.1:${operationsPort}/health`)
  await installCustomers()
  for (const variant of variants) {
    await waitHTTP(`http://127.0.0.1:${variant.admin}/`)
    await waitHTTP(`http://127.0.0.1:${variant.api}/healthz`)
  }
  await writeRuntime()
  runPlaywright(['test', '--config', './tests/system-e2e/playwright.config.mjs'], {
    env: {
      ASTER_SYSTEM_E2E_RUNTIME: runtimeFile,
      ASTER_SYSTEM_E2E_HEADED: headed ? 'true' : 'false',
      ASTER_SYSTEM_E2E_RUN_ROOT: runRoot,
    },
  })
  checkBaselines()
  await writeLiveResult('PASSED')
  process.stdout.write(`System E2E passed. Artifacts: ${runRoot}\n`)
} catch (error) {
  let failure = error
  try { await collectLogs() } catch (diagnosticError) {
    failure = new AggregateError([error, diagnosticError], 'system E2E failed and diagnostic safety scanning also failed')
  }
  await writeLiveResult(await liveFailureStatus(), error instanceof Error ? error.message : String(error))
  throw failure
} finally {
  await releaseHeldPorts()
  if (environmentStarted && !keep) {
    try { compose('down', '--volumes', '--remove-orphans') } catch {}
  }
  if (!keep) {
    await rm(v2SignersFile, { force: true })
    await rm(operationsEnvFile, { force: true })
    await rm(composeEnvFile, { force: true })
    await rm(runtimeFile, { force: true })
    await rm(liveTestStatusFile, { force: true })
    await rm(join(runRoot, 'certs', 'ca.key'), { force: true })
    await rm(join(runRoot, 'certs', 'server.key'), { force: true })
    for (const variant of variants) {
      const directory = `customer-${variant.id === 'qr-png' ? 'qr' : variant.id === 'terminal-screenshot' ? 'terminal' : 'json'}`
      await rm(join(runRoot, directory, 'owner.password'), { force: true })
    }
    if (upstreamMode === 'live') {
      await rm(join(runRoot, 'browser-results'), { recursive: true, force: true })
      await rm(join(runRoot, 'browser-report'), { recursive: true, force: true })
      await rm(join(runRoot, 'diagnostics'), { recursive: true, force: true })
    }
  } else {
    process.stdout.write(`Environment retained with --keep. Runtime: ${runtimeFile}\n`)
  }
}
