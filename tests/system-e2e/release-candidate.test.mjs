import assert from 'node:assert/strict'
import { randomBytes } from 'node:crypto'
import { mkdir, readFile, rm, writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import test from 'node:test'
import { createTestCandidate, hashPath, verifyCandidateManifest, REQUIRED_RUNNER_PROTOCOL, TEST_MATRIX_VERSION } from './release-candidate.mjs'

test('candidate protocol agrees with the Runner wire protocol', async () => {
  const source = await readFile(new URL('../../customer/backend/crates/runner-protocol/src/lib.rs', import.meta.url), 'utf8')
  const declarations = [...source.matchAll(/^pub const RUNNER_PROTOCOL_VERSION: u32 = (\d+);$/gm)]
  assert.equal(declarations.length, 1)
  assert.equal(REQUIRED_RUNNER_PROTOCOL, Number(declarations[0][1]))
})

test('a local candidate uses this run exported bundle instead of a stale host dist directory', async () => {
  const root = join(process.cwd(), 'target', 'system-e2e-unit', randomBytes(6).toString('hex'))
  const customerRoot = join(root, 'target/run/customer-bundle')
  try {
    for (const path of ['bin', 'admin', 'member']) await mkdir(join(customerRoot, path), { recursive: true })
    await mkdir(join(root, 'dist/linux'), { recursive: true })
    await mkdir(join(root, 'dist/operations/aster-operations-0.1.0-linux-amd64'), { recursive: true })
    await writeFile(join(customerRoot, 'RELEASE.json'), JSON.stringify({ key_id: 'lab', signature: 'test-only' }))
    await writeFile(join(customerRoot, 'bin/aster-runner'), 'test-runner')
    await writeFile(join(root, 'dist/linux/aster-team-2.0.1-linux-amd64.tar.gz'), 'test-archive')
    const candidate = await createTestCandidate({ repositoryRoot: root, customerRoot,
      outputPath: join(root, 'candidate.json'), version: '2.0.1', commit: '1'.repeat(40), candidateID: 'test-export' })
    assert.equal(candidate.resolvedArtifacts.get('runner'), join(customerRoot, 'bin/aster-runner'))
    assert.equal(candidate.manifest.required_capabilities.runner_protocol, REQUIRED_RUNNER_PROTOCOL)
    await writeFile(join(root, 'candidate.json'), JSON.stringify({ ...candidate.manifest,
      required_capabilities: { ...candidate.manifest.required_capabilities, runner_protocol: 2 } }))
    await assert.rejects(verifyCandidateManifest(join(root, 'candidate.json'), root), new RegExp(`Runner protocol v${REQUIRED_RUNNER_PROTOCOL}`))
    await writeFile(join(root, 'candidate.json'), JSON.stringify(candidate.manifest))
    await writeFile(join(customerRoot, 'bin/aster-runner'), 'tampered')
    await assert.rejects(verifyCandidateManifest(join(root, 'candidate.json'), root), /digest mismatch/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})

test('candidate verification rejects an artifact changed after packaging', async () => {
  const root = join(process.cwd(), 'target', 'system-e2e-unit', randomBytes(6).toString('hex'))
  await mkdir(root, { recursive: true })
  const components = ['customer-package', 'customer-release-manifest', 'runner', 'admin-frontend', 'member-frontend', 'operations']
  try {
    const artifacts = []
    for (const component of components) {
      const path = join(root, component)
      await writeFile(path, component)
      artifacts.push({ component, path: component, sha256: await hashPath(path) })
    }
    const manifestPath = join(root, 'candidate.json')
    await writeFile(manifestPath, JSON.stringify({
      schema: 'aster.release-candidate.v1',
      candidate_id: 'candidate-test',
      trust: 'release-candidate',
      source: { commit: '1'.repeat(40) },
      built_at: new Date().toISOString(),
      test_matrix_version: TEST_MATRIX_VERSION,
      artifact_root: '.',
      artifacts,
      customer_release: { sha256: artifacts[1].sha256, key_id: 'test', signature: 'test' },
      required_capabilities: {
        schema: 'aster.required-capabilities.v1', providers: ['openai'], operations: ['text', 'image_generation', 'image_edit'], streaming: true, runner_protocol: REQUIRED_RUNNER_PROTOCOL,
      },
    }))
    await verifyCandidateManifest(manifestPath, root)
    const manifest = JSON.parse(await readFile(manifestPath, 'utf8'))
    for (const protocol of [2, REQUIRED_RUNNER_PROTOCOL + 1, String(REQUIRED_RUNNER_PROTOCOL), null]) {
      await writeFile(manifestPath, JSON.stringify({ ...manifest, required_capabilities: { ...manifest.required_capabilities, runner_protocol: protocol } }))
      await assert.rejects(verifyCandidateManifest(manifestPath, root), /Runner protocol/)
    }
    await writeFile(manifestPath, JSON.stringify({ ...manifest, test_matrix_version: 'upstream-provider-e2e.v2' }))
    await assert.rejects(verifyCandidateManifest(manifestPath, root), /test matrix/)
    await writeFile(manifestPath, JSON.stringify(manifest))
    await writeFile(join(root, artifacts[0].path), 'changed')
    await assert.rejects(verifyCandidateManifest(manifestPath, root), /digest mismatch/)
  } finally {
    await rm(root, { recursive: true, force: true })
  }
})
