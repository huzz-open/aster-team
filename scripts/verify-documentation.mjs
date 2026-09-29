import { existsSync, readFileSync, readdirSync, statSync } from 'node:fs'
import { dirname, extname, resolve, sep } from 'node:path'
import { parse as parseYaml } from 'yaml'

const root = resolve('.')
const failures = []

function filesBelow(directory) {
  const output = []
  for (const entry of readdirSync(directory)) {
    if (['.aster-tools', '.git', 'dist', 'node_modules', 'target'].includes(entry)) continue
    const path = resolve(directory, entry)
    if (statSync(path).isDirectory()) output.push(...filesBelow(path))
    else output.push(path)
  }
  return output
}

const repositoryFiles = filesBelow(root)
const markdownFiles = repositoryFiles.filter(path => extname(path).toLowerCase() === '.md')
const packageManifest = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8'))
const packageScripts = packageManifest.scripts || {}
const brandSourcePath = resolve(root, 'docs/internal/assets/aster-team-product-icon-v1.svg')
const brandSource = readFileSync(brandSourcePath, 'utf8')
const documentationSiteRoot = resolve(root, 'website/docs')

function markdownLinkExists(file, target) {
  if (file.startsWith(`${documentationSiteRoot}${sep}`) && target.startsWith('/') && !target.startsWith('//')) {
    const source = resolve(documentationSiteRoot, `.${target}`)
    if (!source.startsWith(`${documentationSiteRoot}${sep}`)) return false
    const publicAsset = resolve(documentationSiteRoot, 'public', target.slice(1))
    return [source, `${source}.md`, resolve(source, 'index.md'), publicAsset]
      .some(path => existsSync(path) && statSync(path).isFile())
  }
  return existsSync(resolve(dirname(file), target))
}

for (const file of markdownFiles) {
  const source = readFileSync(file, 'utf8')
  for (const match of source.matchAll(/\[[^\]]*\]\(([^)]+)\)/g)) {
    const raw = match[1].trim().replace(/^<|>$/g, '')
    let target = raw.split('#')[0]
    if (!target || /^(?:https?:|mailto:)/.test(target)) continue
    try { target = decodeURIComponent(target) } catch {}
    if (!markdownLinkExists(file, target)) failures.push(`${file}: missing relative link ${raw}`)
  }
  for (const match of source.matchAll(/npm run ([A-Za-z0-9:_-]+)/g)) {
    if (!packageScripts[match[1]]) failures.push(`${file}: unknown root npm script ${match[1]}`)
  }
}

const contracts = [
  ['customer/admin/package.json', '11082'],
  ['customer/member/package.json', '11081'],
  ['operations/console/package.json', '12080'],
  ['website/package.json', '14080'],
  ['customer/admin/vite.config.ts', '127.0.0.1:11080'],
  ['customer/member/vite.config.ts', '127.0.0.1:11080'],
  ['operations/console/vite.config.ts', '127.0.0.1:12090'],
  ['scripts/setup-local-stack.mjs', 'ASTER_LICENSE_TRUSTED_KEYS_JSON'],
  ['scripts/setup-local-stack.mjs', 'ASTER_RELEASE_TRUSTED_KEYS_JSON'],
  ['scripts/setup-local-stack.mjs', 'ASTER_OPERATIONS_ADDR=${operationsAddress}'],
  ['scripts/setup-local-stack.mjs', 'ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT=5m'],
  ['scripts/setup-local-stack.mjs', 'ASTER_CONTROL_PORT=${controlPort}'],
  ['operations/backend/internal/config/config.go', 'ASTER_OPERATIONS_DB_DRIVER must be mysql'],
  ['package.json', 'scripts/run-rust-control.mjs'],
  ['package.json', 'scripts/run-rust-runner.mjs'],
  ['.env.example', 'ASTER_LOCAL_ADMIN_EMAIL='],
  ['.env.example', 'ASTER_OPERATIONS_DB_ADMIN_PASSWORD='],
  ['.env.example', 'ASTER_CUSTOMER_DB_ADMIN_PASSWORD='],
  ['.env.example', 'ASTER_OPERATIONS_DB_SERVICE_USER='],
  ['.env.example', 'ASTER_CUSTOMER_DB_SERVICE_USER='],
  ['customer/backend/control/src/main.rs', 'GenerateLicenseRequest'],
  ['customer/backend/control/src/main.rs', 'InstallLicense'],
  ['customer/backend/control/src/main.rs', 'VerifyRelease'],
  ['customer/backend/cli/src/main.rs', 'name = "aster-team-cli"'],
  ['customer/backend/cli/src/main.rs', 'verify_release_tree(root, &release)'],
  ['contracts/install-layout.json', 'aster.install-layout.v1'],
  ['contracts/install-layout.json', 'C:\\\\ProgramData\\\\Aster Team'],
  ['contracts/install-layout.json', '/Library/Application Support/Aster Team'],
  ['customer/deploy/init.sh', 'bin/aster-team-cli" bootstrap --release-root'],
  ['customer/deploy/install.sh', '--release-manifest-sha256'],
  ['customer/deploy/install.sh', '--runner-only'],
  ['customer/deploy/restore-backup.sh', '--confirm-restore'],
  ['.github/workflows/customer-release.yml', 'ASTER_RELEASE_SIGNING_SEED_BASE64'],
  ['.github/workflows/customer-release.yml', 'scripts/build-linux-amd64.sh'],
  ['scripts/build-linux-bundle.mjs', "'--features', 'sqlcipher,mariadb'"],
  ['scripts/build-linux-bundle.mjs', 'restore-backup.sh'],
  ['customer/deploy/systemd/aster-control@.service', '@ASTER_ROOT@/state/slots/%i-release/bin/aster-control serve'],
  ['customer/deploy/systemd/aster-runner.service', '@ASTER_ROOT@/config/runner/runner.env'],
  ['customer/deploy/systemd/aster-upgrade.service', '@ASTER_ROOT@/bin/aster-team-cli maintenance run-next'],
  ['docs/error-codes.md', '禁止随机生成'],
  ['operations/deploy/README.md', '--confirm-database aster_operations'],
  ['README-LINUX.md', 'sudo ./init.sh'],
  ['README-LINUX.md', 'aster-team-cli backup restore'],
  ['README-LINUX.md', 'npm run setup:linux-lab'],
  ['README-LOCAL.md', 'npm run setup:check'],
  ['README-LOCAL.md', 'npm run release:local'],
  ['README-LOCAL.md', '管理员 PowerShell'],
  ['docs/operations-guide.md', '--platform=all'],
  ['scripts/setup-windows-environment.mjs', 'Docker.DockerDesktop'],
]
for (const [file, expected] of contracts) {
  if (!readFileSync(resolve(root, file), 'utf8').includes(expected)) failures.push(`${file}: missing documented contract ${expected}`)
}

const operationsErrorImplementation = readFileSync(resolve(root, 'operations/backend/internal/apierrors/http.go'), 'utf8')
const operationsErrorDocumentation = readFileSync(resolve(root, 'docs/error-codes.md'), 'utf8')
if (/crypto\/rand|\bNewNumber\b|\bWriteNumber\b/.test(operationsErrorImplementation)) {
  failures.push('operations/backend/internal/apierrors/http.go: random or caller-supplied numeric error codes are forbidden')
}
const publishedErrors = parseYaml(readFileSync(resolve(root, 'contracts/catalogs/error-codes.yaml'), 'utf8')).errors
const registeredOperationCodes = new Map(publishedErrors
  .filter(entry => entry.target === 'operations')
  .map(entry => [entry.code, entry.number]))
const operationGoSources = repositoryFiles.filter(path => path.endsWith('.go') && !path.endsWith('_test.go') && path.startsWith(resolve(root, 'operations')))
const usedOperationCodes = new Set()
for (const file of operationGoSources) {
  const source = readFileSync(file, 'utf8')
  for (const match of source.matchAll(/\b(?:apierrors\.Write|writeError|api\.writeBusinessError)\s*\([^\n]*?"([A-Z][A-Z0-9_]+)"/g)) usedOperationCodes.add(match[1])
}
for (const code of usedOperationCodes) {
  if (!registeredOperationCodes.has(code)) failures.push(`operations/backend/internal/apierrors/http.go: missing fixed number for ${code}`)
}
for (const [code, number] of registeredOperationCodes) {
  if (!operationsErrorDocumentation.includes(`\`${code}\``)) failures.push(`docs/error-codes.md: missing Operations error ${code}`)
  if (!operationsErrorDocumentation.includes(`\`${number}\``)) failures.push(`docs/error-codes.md: missing Operations error number ${number}`)
}

const registeredCustomerCodes = new Map()
const allPublishedNumbers = new Map([...registeredOperationCodes].map(([code, number]) => [number, `Operations ${code}`]))
for (const entry of publishedErrors.filter(entry => entry.target === 'customer')) {
  const { code, number } = entry
  if (number < 10_000 || number > 99_999) failures.push(`rust error catalog: ${code} must use a five-digit number`)
  if (registeredCustomerCodes.has(code)) failures.push(`rust error catalog: duplicate string code ${code}`)
  if (allPublishedNumbers.has(number)) failures.push(`error catalogs: ${code} reuses ${number} from ${allPublishedNumbers.get(number)}`)
  registeredCustomerCodes.set(code, number)
  allPublishedNumbers.set(number, `Customer ${code}`)
  if (!operationsErrorDocumentation.includes(`\`${code}\``)) failures.push(`docs/error-codes.md: missing Customer error ${code}`)
  if (!operationsErrorDocumentation.includes(`\`${number}\``)) failures.push(`docs/error-codes.md: missing Customer error number ${number}`)
}
if (registeredCustomerCodes.size === 0) failures.push('rust error catalog: no fixed Customer errors were discovered')

for (const file of [
  'customer/admin/public/favicon.svg',
  'customer/member/public/favicon.svg',
  'operations/console/public/favicon.svg',
  'website/public/favicon.svg',
]) {
  if (readFileSync(resolve(root, file), 'utf8') !== brandSource) failures.push(`${file}: must match the canonical Aster Team brand icon`)
}

const customerReleaseWorkflow = readFileSync(resolve(root, '.github/workflows/customer-release.yml'), 'utf8')
for (const obsolete of [
  'offline-runtime', 'MANIFEST.json', 'mysql-existing', '--clobber',
  'ASTER_LICENSE_PUBLIC_KEY_SPKI', 'aster-runner-*-linux-amd64',
]) {
  if (customerReleaseWorkflow.includes(obsolete)) failures.push(`customer-release.yml: obsolete release contract ${obsolete}`)
}

for (const [file, obsolete] of [
  ['customer/deploy/README.md', 'sudo ./install.sh'],
  ['README-LINUX.md', 'sudo ./install.sh'],
  ['docs/customer-guide.md', 'install.sh --runner-only'],
  ['docs/user-manual.md', 'sudo ./install.sh'],
  ['operations/console/src/views/ReleaseArtifactsView.vue', 'sudo ./install.sh'],
  ['customer/admin/src/views/RunnerView.vue', '/opt/aster-team/current/bin/aster-runner enroll'],
]) {
  if (readFileSync(resolve(root, file), 'utf8').includes(obsolete)) failures.push(`${file}: obsolete public Customer command ${obsolete}`)
}

const localDocumentation = readFileSync(resolve(root, 'README-LOCAL.md'), 'utf8')
for (const obsolete of [
  '127.0.0.1:5173', '127.0.0.1:5174', '127.0.0.1:5180', '127.0.0.1:8080',
  '--operations-db-password', '--operations-env-file', 'mariadb-password.txt',
  'License Guard', 'build:local:linux', 'build:local:offline:linux',
]) {
  if (localDocumentation.includes(obsolete)) failures.push(`README-LOCAL.md: obsolete local instruction ${obsolete}`)
}

if (failures.length) {
  console.error(failures.join('\n'))
  process.exit(1)
}
console.log(`Documentation contracts verified across ${markdownFiles.length} Markdown files.`)
