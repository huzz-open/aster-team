import test from 'node:test'
import assert from 'node:assert/strict'
import { createHash } from 'node:crypto'
import { mkdtempSync, mkdirSync, readFileSync, writeFileSync, rmSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { reusableAcceptance, preparedWindowsPackage } from './release-inputs.mjs'

const hash = bytes => createHash('sha256').update(bytes).digest('hex')
function fixture(t) {
  const root = mkdtempSync(join(tmpdir(), 'aster-acceptance-reuse-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const id = '20260908075939781-2e61cec9', version = '2.0.1-test.1'
  const dir = join(root, 'dist/website-release-validation', id)
  mkdirSync(dir, { recursive: true })
  const name = `aster-team-${version}-linux-amd64.tar.gz`, bytes = Buffer.from('explicit unit fixture')
  const path = join(dir, name)
  writeFileSync(path, bytes)
  const artifact = { name, sha256: hash(bytes), size_bytes: bytes.length }
  const inputs = { commit: 'a'.repeat(40), package_commit: 'b'.repeat(40), version, archive_path: path, artifact, tool_sha256: 'c'.repeat(64) }
  const encoded = JSON.stringify(inputs)
  writeFileSync(join(dir, 'inputs.json'), encoded)
  const result = { schema: 'aster.website-release-acceptance.v1', status: 'passed', trust: 'test-build', run_id: id,
    commit: inputs.commit, version, artifact, inputs_sha256: hash(encoded) }
  const writeResult = () => writeFileSync(join(dir, 'result.json'), JSON.stringify(result))
  writeResult()
  return { root, id, version, dir, path, result, writeResult }
}
test('reuse preserves original package source instead of relabelling it as the new frontend', t => {
  const f = fixture(t)
  assert.equal(reusableAcceptance(f.root, f.id, f.version).packageCommit, 'b'.repeat(40))
})
test('reuse rejects failed acceptance, altered archive and altered input receipt', t => {
  const f = fixture(t)
  f.result.status = 'failed'; f.writeResult()
  assert.throws(() => reusableAcceptance(f.root, f.id, f.version), /completed matching/)
  f.result.status = 'passed'; f.writeResult()
  writeFileSync(f.path, 'changed')
  assert.throws(() => reusableAcceptance(f.root, f.id, f.version), /Changed previously/)
  writeFileSync(join(f.dir, 'inputs.json'), '{}')
  assert.throws(() => reusableAcceptance(f.root, f.id, f.version), /completed matching/)
  assert.throws(() => reusableAcceptance(f.root, '../elsewhere', f.version), /Invalid acceptance/)
})

test('Windows handoff checks exact source, completion and immutable package bytes without signing inputs', t => {
  const root = mkdtempSync(join(tmpdir(), 'aster-windows-handoff-'))
  t.after(() => rmSync(root, { recursive: true, force: true }))
  const fingerprint = 'a'.repeat(64), id = fingerprint.slice(0, 12), version = '2.0.1-test.1', commit = 'b'.repeat(40)
  const dir = join(root, 'target/wp', id), packages = join(dir, 'packages/windows')
  mkdirSync(packages, { recursive: true })
  const archive = join(packages, `aster-team-${version}-windows-amd64.tar.gz`)
  writeFileSync(archive, 'explicit test package bytes')
  writeFileSync(`${archive}.sha256`, hash(readFileSync(archive)))
  const receipt = { schema: 'aster.windows-pipeline-fixture.v1', source_clean: true, fingerprint, inputs: { version, head: commit },
    steps: { base: Object.fromEntries([archive, `${archive}.sha256`].map(path => [path, hash(readFileSync(path))])), candidate: {} } }
  const save = () => writeFileSync(join(dir, 'build-receipt.json'), JSON.stringify(receipt))
  save()
  assert.equal(preparedWindowsPackage(root, id, version, commit).archive, archive)
  assert.throws(() => preparedWindowsPackage(root, '../unsafe', version, commit), /Invalid Windows/)
  assert.throws(() => preparedWindowsPackage(root, id, version, 'c'.repeat(40)), /current source/)
  delete receipt.source_clean; save()
  assert.throws(() => preparedWindowsPackage(root, id, version, commit), /completed Windows/)
  receipt.source_clean = true
  delete receipt.steps.candidate; save()
  assert.throws(() => preparedWindowsPackage(root, id, version, commit), /completed Windows/)
  receipt.steps.candidate = {}; save()
  writeFileSync(archive, 'modified')
  assert.throws(() => preparedWindowsPackage(root, id, version, commit), /Changed prepared/)
})
