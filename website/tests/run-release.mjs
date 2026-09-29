import { createHash, randomBytes } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { copyFileSync, existsSync, mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { build } from 'vite'
import { createSystemCommandRunner } from '../../tests/system-e2e/command-runner.mjs'
import { buildPublicSupportBundle } from '../../scripts/build-public-support-bundle.mjs'
import { canonicalJson } from '../build/public-catalog.mjs'
import { reusableAcceptance, preparedWindowsPackage } from './release-inputs.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')
const commands = createSystemCommandRunner(root)
const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')
function run(command, args, capture = false, input) {
  const result = spawnSync(command, args, { cwd: root, env: commands.environment, windowsHide: true,
    stdio: capture ? [input === undefined ? 'ignore' : 'pipe', 'pipe', 'pipe'] : 'inherit', encoding: capture ? 'utf8' : undefined, input })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} exited with ${result.status}: ${capture ? result.stderr : 'see preceding output'}`)
  return result.stdout
}
if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Full website release acceptance requires native Windows x64 and Docker Linux containers')
if (!process.env.npm_execpath) throw new Error('Run npm run test:website:release:windows')
const args = process.argv.slice(2)
const options = {}
for (let index = 0; index < args.length; index += 2) {
  const key = args[index]
  if (!['--reuse-acceptance', '--windows-fixture'].includes(key) || !args[index + 1] || options[key]) throw new Error('Usage: npm run test:website:release:windows -- [--reuse-acceptance <run-id>] [--windows-fixture <id>]')
  options[key] = args[index + 1]
}
const commit = run('git', ['rev-parse', 'HEAD'], true).trim()
function assertSource() {
  if (run('git', ['rev-parse', 'HEAD'], true).trim() !== commit
    || run('git', ['status', '--porcelain', '--untracked-files=all'], true).trim()) {
    throw new Error('Website package acceptance requires one unchanged committed source tree')
  }
}
assertSource()
const { version } = JSON.parse(readFileSync(join(root, 'package.json'), 'utf8'))
if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:[-+][0-9A-Za-z.-]+)?$/.test(version)) throw new Error('Invalid product version')
const runID = `${new Date().toISOString().replace(/[-:.TZ]/g, '')}-${randomBytes(4).toString('hex')}`
const runRoot = join(root, 'dist/website-release-validation', runID)
mkdirSync(runRoot, { recursive: true })
const state = { schema: 'aster.website-release-acceptance.v1', trust: 'test-build', commit, version, run_id: runID, started_at: new Date().toISOString(), status: 'running' }
const writeState = () => writeFileSync(join(runRoot, 'result.json'), `${JSON.stringify(state, null, 2)}\n`)
writeState()
process.stdout.write(`Website release acceptance: ${runRoot}\n`)
try {
  const archiveName = `aster-team-${version}-linux-amd64.tar.gz`
  const archive = join(runRoot, archiveName)
  let toolSHA256
  let packageCommit = commit
  if (options['--reuse-acceptance']) {
    const prior = reusableAcceptance(root, options['--reuse-acceptance'], version)
    copyFileSync(prior.archive, archive)
    writeFileSync(`${archive}.sha256`, `${sha256(readFileSync(archive))}  ${archiveName}\n`)
    toolSHA256 = prior.toolSHA256
    packageCommit = prior.packageCommit
    state.package_acceptance_run = prior.sourceRun
    console.log(`Reusing accepted package from ${packageCommit}; only website/download acceptance runs again.`)
  } else {
    const tool = join(runRoot, 'asterctl.exe')
    run(process.execPath, [join(root, 'scripts/build-asterctl-windows.mjs'), `--output=${tool}`])
    const originalArchive = join(root, 'dist/linux', archiveName)
    // Preserve previously reported bytes before the lab updates its conventional output.
    if (existsSync(originalArchive)) {
      const previous = join(runRoot, 'previous-package')
      mkdirSync(previous)
      copyFileSync(originalArchive, join(previous, archiveName))
      if (existsSync(`${originalArchive}.sha256`)) copyFileSync(`${originalArchive}.sha256`, join(previous, `${archiveName}.sha256`))
    }
    const bundle = join(runRoot, 'customer-bundle')
    run(commands.executable('bash'), ['--noprofile', '--norc', 'scripts/ci/linux-lab.sh', 'quick', '--jobs', '2',
      '--asterctl-windows-x64', commands.bashPath(tool), '--export-bundle', commands.bashPath(bundle)])
    assertSource()
    copyFileSync(originalArchive, archive)
    copyFileSync(`${originalArchive}.sha256`, `${archive}.sha256`)
    toolSHA256 = sha256(readFileSync(tool))
    if (sha256(readFileSync(join(bundle, 'client-tools/asterctl/windows-x86_64/asterctl.exe'))) !== toolSHA256) throw new Error('Linux bundle does not contain the actual native tool')
  }
  state.package_commit = packageCommit
  const supportRoot = join(root, 'dist/public-support', `website-${runID}`)
  const support = buildPublicSupportBundle({ version, archivePath: archive, checksumPath: `${archive}.sha256`, outputPath: supportRoot, environment: 'local' })
  const supportManifest = join(supportRoot, `releases/${version}/manifest.json`)
  const license = support.bundled_license
  // Review values use the same Go freeze, calculation and public projection as Operations.
  // No database approval, customer order, License issuance or production offer is created.
  const review = JSON.parse(run('go', ['run', './operations/backend/cmd/labwebsitecatalog'], true, JSON.stringify({
    plan_id: license.plan_id, version: license.plan_version,
    definition: { product: 'aster-team', code: 'local_review_free', name: '免费版',
      description: '3 席位体验基础功能 本地方案预览 非正式发行', edition: license.edition,
      entitlements: license.entitlements, quota_policy_version: license.quota_policy_version,
      minimum_version: license.minimum_version, transfer_limit: 0, support_terms_version: 'local-review-support-pending',
      offer: { kind: 'free', expiry: license.expiry } },
  })))
  const catalog = review.catalog
  if (catalog.environment !== 'local' || sha256(canonicalJson(catalog)) !== review.sha256) throw new Error('Invalid local Operations projection')
  writeFileSync(join(runRoot, 'review-plan-snapshots.json'), `${JSON.stringify(review.plans, null, 2)}\n`)
  const catalogPath = join(runRoot, 'plans.json')
  const catalogBytes = canonicalJson(catalog)
  writeFileSync(catalogPath, catalogBytes)
  for (const key of Object.keys(process.env)) if (key.startsWith('ASTER_WEBSITE_')) delete process.env[key]
  Object.assign(process.env, {
    ASTER_WEBSITE_CATALOG_PATH: catalogPath, ASTER_WEBSITE_CATALOG_SHA256: sha256(catalogBytes),
    ASTER_WEBSITE_CATALOG_REVISION: catalog.revision, ASTER_WEBSITE_CATALOG_ENVIRONMENT: 'local',
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_PATH: supportManifest,
    ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256: sha256(readFileSync(supportManifest)),
    ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT: 'local', ASTER_WEBSITE_PRODUCT_RELEASE_ARCHIVE_PATH: archive,
    VITE_ASTER_TURNSTILE_SITE_KEY: '', VITE_ASTER_CONTACT_EMAIL: 'local-acceptance@example.invalid', VITE_ASTER_LEAD_ENDPOINT: '/api/trial',
  })
  let windows
  if (options['--windows-fixture']) {
    const prepared = preparedWindowsPackage(root, options['--windows-fixture'], version, commit)
    const windowsArchive = join(runRoot, `aster-team-${version}-windows-amd64.tar.gz`)
    copyFileSync(prepared.archive, windowsArchive)
    copyFileSync(`${prepared.archive}.sha256`, `${windowsArchive}.sha256`)
    const windowsSupportRoot = `${supportRoot}-windows`
    const manifest = buildPublicSupportBundle({ version, archivePath: windowsArchive, checksumPath: `${windowsArchive}.sha256`,
      outputPath: windowsSupportRoot, environment: 'local', platform: 'windows' })
    const manifestPath = join(windowsSupportRoot, `releases/${version}/manifest.json`)
    Object.assign(process.env, {
      ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_PATH: manifestPath,
      ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_SHA256: sha256(readFileSync(manifestPath)),
      ASTER_WEBSITE_WINDOWS_RELEASE_ENVIRONMENT: 'local', ASTER_WEBSITE_WINDOWS_RELEASE_ARCHIVE_PATH: windowsArchive,
    })
    windows = { package_commit: prepared.packageCommit, preparation_id: prepared.sourceRun, archive_path: windowsArchive,
      support_root: windowsSupportRoot, artifact: manifest.artifact, release_manifest: manifest.release_manifest,
      bundled_license: manifest.bundled_license, installation_status: 'not_run_requires_administrator' }
  }
  const siteRoot = join(runRoot, 'site')
  run(process.execPath, [process.env.npm_execpath, 'run', 'typecheck', '--workspace', '@aster/website'])
  await build({ root: join(root, 'website'), configFile: join(root, 'website/vite.config.ts'), envDir: false,
    build: { outDir: siteRoot, emptyOutDir: true } })
  const inputs = { commit, package_commit: packageCommit, package_acceptance_run: state.package_acceptance_run, version, site_root: siteRoot, archive_path: archive, support_root: supportRoot,
    catalog_path: catalogPath, artifact: support.artifact, release_manifest: support.release_manifest,
    bundled_license: support.bundled_license, tool_sha256: toolSHA256, ...(windows ? { windows } : {}) }
  const inputsPath = join(runRoot, 'inputs.json')
  writeFileSync(inputsPath, `${JSON.stringify(inputs, null, 2)}\n`)
  Object.assign(commands.environment, { ASTER_WEBSITE_RELEASE_TEST_INPUTS: inputsPath })
  run(process.execPath, [join(root, 'node_modules/@playwright/test/cli.js'), 'test', '--config', 'website/tests/release.playwright.config.mjs'])
  assertSource()
  Object.assign(state, { status: 'passed', artifact: support.artifact, tool_sha256: toolSHA256,
    release_manifest: support.release_manifest, ...(windows ? { windows } : {}), inputs_sha256: sha256(readFileSync(inputsPath)) })
} catch (error) {
  state.status = 'failed'
  state.error = error.message
  throw error
} finally {
  state.finished_at = new Date().toISOString()
  writeState()
}
