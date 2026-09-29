import { randomBytes } from 'node:crypto'
import { mkdir, readFile, rm } from 'node:fs/promises'
import { dirname, join, resolve } from 'node:path'
import { spawn } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createSystemCommandRunner } from './command-runner.mjs'
import { createTestCandidate } from './release-candidate.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..', '..')
const { bashPath, environment: commandEnvironment, executable, run, runNpm } = createSystemCommandRunner(root)
const version = JSON.parse(await readFile(join(root, 'package.json'), 'utf8')).version
const preparationID = `parallel-${Date.now()}-${randomBytes(4).toString('hex')}`
const preparationRoot = join(root, 'target', 'system-e2e', 'parallel-candidates', preparationID)
const v2SignersFile = join(preparationRoot, 'v2-signers.json')
const customerRoot = join(preparationRoot, 'customer-bundle')
const manifestFile = join(preparationRoot, 'candidate-manifest.json')

await mkdir(preparationRoot, { recursive: true })
try {
  run(executable('bash'), ['--noprofile', '--norc', 'scripts/ci/linux-lab.sh', 'quick', '--jobs', '2',
    '--export-v2-signers', bashPath(v2SignersFile), '--export-bundle', bashPath(customerRoot)])
  runNpm(['run', 'build:operations'])
  const commit = run('git', ['rev-parse', 'HEAD'], { capture: true }).trim()
  await createTestCandidate({ repositoryRoot: root, customerRoot, outputPath: manifestFile, version, commit, candidateID: `test-${preparationID}` })

  const childArgs = [join(root, 'tests', 'system-e2e', 'run.mjs'), `--candidate-manifest=${manifestFile}`]
  const environment = { ...commandEnvironment, ASTER_E2E_V2_SIGNERS_FILE: v2SignersFile }
  const children = [0, 1].map(index => spawn(process.execPath, childArgs, {
    cwd: root,
    stdio: ['ignore', 'inherit', 'inherit'],
    env: { ...environment, ASTER_E2E_PARALLEL_SLOT: String(index + 1) },
  }))
  const statuses = await Promise.all(children.map(child => new Promise((accept, reject) => {
    child.once('error', reject)
    child.once('exit', (code, signal) => accept({ code, signal }))
  })))
  if (statuses.some(status => status.code !== 0)) throw new Error(`parallel system E2E failed: ${JSON.stringify(statuses)}`)
  process.stdout.write(`Two isolated system E2E environments passed against ${manifestFile}\n`)
} finally {
  await rm(v2SignersFile, { force: true })
}
