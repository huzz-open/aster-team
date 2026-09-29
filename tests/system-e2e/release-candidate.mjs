import { createHash } from 'node:crypto'
import { lstat, readFile, readdir, writeFile } from 'node:fs/promises'
import { dirname, isAbsolute, relative, resolve } from 'node:path'
import { parse as parseYaml } from 'yaml'

export const RELEASE_CANDIDATE_SCHEMA = 'aster.release-candidate.v1'
export const TEST_MATRIX_VERSION = 'upstream-provider-e2e.v3'
const candidateSchema = parseYaml(await readFile(new URL('../../contracts/schemas/release-candidate.v1.schema.yaml', import.meta.url), 'utf8'))
export const REQUIRED_RUNNER_PROTOCOL = candidateSchema.properties.required_capabilities.properties.runner_protocol.const
if (!Number.isInteger(REQUIRED_RUNNER_PROTOCOL) || REQUIRED_RUNNER_PROTOCOL < 1) throw new Error('candidate schema must declare the required Runner protocol')

function sha256(value) {
  return createHash('sha256').update(value).digest('hex')
}

export async function hashPath(path) {
  const metadata = await lstat(path)
  if (metadata.isSymbolicLink()) throw new Error(`candidate artifact cannot be a symbolic link: ${path}`)
  if (metadata.isFile()) return sha256(await readFile(path))
  if (!metadata.isDirectory()) throw new Error(`candidate artifact is neither a file nor a directory: ${path}`)
  const entries = []
  async function walk(directory) {
    for (const entry of await readdir(directory, { withFileTypes: true })) {
      const current = resolve(directory, entry.name)
      if (entry.isSymbolicLink()) throw new Error(`candidate artifact tree cannot contain a symbolic link: ${current}`)
      if (entry.isDirectory()) await walk(current)
      else if (entry.isFile()) entries.push(current)
    }
  }
  await walk(path)
  entries.sort((left, right) => left.localeCompare(right, 'en'))
  const digest = createHash('sha256')
  for (const entry of entries) {
    const payload = await readFile(entry)
    digest.update(`${relative(path, entry).replaceAll('\\', '/')}\0${payload.length}\0`)
    digest.update(sha256(payload))
    digest.update('\n')
  }
  return digest.digest('hex')
}

function artifactPath(root, path) {
  return isAbsolute(path) ? path : resolve(root, path)
}

export async function verifyCandidateManifest(manifestPath, repositoryRoot) {
  const manifest = JSON.parse(await readFile(manifestPath, 'utf8'))
  if (manifest.schema !== RELEASE_CANDIDATE_SCHEMA) throw new Error(`unsupported candidate schema: ${manifest.schema}`)
  if (typeof manifest.candidate_id !== 'string' || !(3 <= manifest.candidate_id.length && manifest.candidate_id.length <= 128)) throw new Error('candidate ID is invalid')
  if (!['release-candidate', 'test-build'].includes(manifest.trust)) throw new Error('candidate trust classification is invalid')
  if (!/^[0-9a-f]{40}$/.test(manifest.source?.commit || '')) throw new Error('candidate source commit is invalid')
  if (typeof manifest.built_at !== 'string' || !Number.isFinite(Date.parse(manifest.built_at))) throw new Error('candidate build time is invalid')
  if (manifest.test_matrix_version !== TEST_MATRIX_VERSION) {
    throw new Error(`candidate test matrix ${manifest.test_matrix_version} does not match ${TEST_MATRIX_VERSION}`)
  }
  if (!Array.isArray(manifest.artifacts) || manifest.artifacts.length === 0) throw new Error('candidate contains no artifacts')
  if (!manifest.artifact_root || isAbsolute(manifest.artifact_root)) throw new Error('candidate artifact_root must be relative to the manifest')
  const manifestRoot = artifactPath(dirname(manifestPath), manifest.artifact_root)
  const resolvedArtifacts = new Map()
  for (const artifact of manifest.artifacts) {
    if (!artifact.component || !artifact.path || !/^[a-f0-9]{64}$/.test(artifact.sha256 || '')) {
      throw new Error('candidate artifact component, path, and SHA-256 are required')
    }
    if (resolvedArtifacts.has(artifact.component)) throw new Error(`candidate artifact component is duplicated: ${artifact.component}`)
    if (isAbsolute(artifact.path)) throw new Error(`candidate artifact path must be relative to artifact_root: ${artifact.component}`)
    const path = resolve(manifestRoot, artifact.path)
    const containment = relative(manifestRoot, path)
    if (containment === '..' || containment.startsWith(`..${process.platform === 'win32' ? '\\' : '/'}`)) {
      throw new Error(`candidate artifact escapes artifact_root: ${artifact.component}`)
    }
    const actual = await hashPath(path)
    if (actual !== artifact.sha256) throw new Error(`candidate artifact digest mismatch for ${artifact.component}: expected ${artifact.sha256}, got ${actual}`)
    resolvedArtifacts.set(artifact.component, path)
  }
  for (const component of ['customer-package', 'customer-release-manifest', 'runner', 'admin-frontend', 'member-frontend', 'operations']) {
    if (!resolvedArtifacts.has(component)) throw new Error(`candidate artifact is missing required component: ${component}`)
  }
  const releaseArtifact = manifest.artifacts.find(artifact => artifact.component === 'customer-release-manifest')
  if (manifest.customer_release?.sha256 !== releaseArtifact?.sha256 || !manifest.customer_release?.key_id || !manifest.customer_release?.signature) {
    throw new Error('candidate customer release signature metadata is missing or does not match the release manifest artifact')
  }
  const required = manifest.required_capabilities
  if (required?.schema !== 'aster.required-capabilities.v1' || !required.providers?.includes('openai')) {
    throw new Error('candidate RequiredCapabilityManifest must require the OpenAI provider')
  }
  for (const operation of ['text', 'image_generation', 'image_edit']) {
    if (!required.operations?.includes(operation)) throw new Error(`candidate RequiredCapabilityManifest is missing ${operation}`)
  }
  if (required.streaming !== true || required.runner_protocol !== REQUIRED_RUNNER_PROTOCOL) {
    throw new Error(`candidate RequiredCapabilityManifest must require streaming and Runner protocol v${REQUIRED_RUNNER_PROTOCOL}`)
  }
  return { manifest, resolvedArtifacts, digest: sha256(await readFile(manifestPath)) }
}

export async function createTestCandidate({ repositoryRoot, outputPath, version, commit, candidateID,
  customerRoot = resolve(repositoryRoot, 'dist', 'linux', `aster-team-${version}-linux-amd64`) }) {
  const releasePath = resolve(customerRoot, 'RELEASE.json')
  const release = JSON.parse(await readFile(releasePath, 'utf8'))
  const artifacts = [
    ['customer-package', resolve(repositoryRoot, 'dist', 'linux', `aster-team-${version}-linux-amd64.tar.gz`)],
    ['customer-release-manifest', releasePath],
    ['runner', resolve(customerRoot, 'bin', 'aster-runner')],
    ['admin-frontend', resolve(customerRoot, 'admin')],
    ['member-frontend', resolve(customerRoot, 'member')],
    ['operations', resolve(repositoryRoot, 'dist', 'operations', 'aster-operations-0.1.0-linux-amd64')],
  ]
  const manifest = {
    schema: RELEASE_CANDIDATE_SCHEMA,
    candidate_id: candidateID,
    trust: 'test-build',
    source: { commit },
    built_at: new Date().toISOString(),
    test_matrix_version: TEST_MATRIX_VERSION,
    artifact_root: relative(dirname(outputPath), repositoryRoot).replaceAll('\\', '/') || '.',
    artifacts: await Promise.all(artifacts.map(async ([component, path]) => ({
      component,
      path: relative(repositoryRoot, path).replaceAll('\\', '/'),
      sha256: await hashPath(path),
    }))),
    customer_release: {
      sha256: await hashPath(releasePath),
      key_id: release.key_id,
      signature: release.signature,
    },
    required_capabilities: {
      schema: 'aster.required-capabilities.v1',
      providers: ['openai'],
      operations: ['text', 'image_generation', 'image_edit'],
      streaming: true,
      runner_protocol: REQUIRED_RUNNER_PROTOCOL,
    },
  }
  await writeFile(outputPath, `${JSON.stringify(manifest, null, 2)}\n`)
  return verifyCandidateManifest(outputPath, repositoryRoot)
}
