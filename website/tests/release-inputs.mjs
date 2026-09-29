import { createHash } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { join, resolve } from 'node:path'

const sha256 = bytes => createHash('sha256').update(bytes).digest('hex')

// Reuse retains the package's own source identity. It never promotes a fixture,
// failed run, changed archive or changed handoff to installation evidence.
export function reusableAcceptance(root, id, version) {
  if (!/^\d{17}-[a-f0-9]{8}$/.test(id)) throw new Error('Invalid acceptance run ID')
  const runRoot = join(root, 'dist/website-release-validation', id)
  const result = JSON.parse(readFileSync(join(runRoot, 'result.json')))
  const bytes = readFileSync(join(runRoot, 'inputs.json'))
  const inputs = JSON.parse(bytes)
  if (result.schema !== 'aster.website-release-acceptance.v1' || result.trust !== 'test-build'
    || result.status !== 'passed' || result.run_id !== id || result.version !== version
    || inputs.commit !== result.commit || inputs.version !== version || sha256(bytes) !== result.inputs_sha256) {
    throw new Error('A completed matching test-package acceptance is required')
  }
  const archiveName = `aster-team-${version}-linux-amd64.tar.gz`
  const archive = join(runRoot, archiveName)
  if (resolve(inputs.archive_path) !== archive || inputs.artifact.name !== archiveName
    || JSON.stringify(inputs.artifact) !== JSON.stringify(result.artifact)) throw new Error('Inconsistent package handoff')
  const archiveBytes = readFileSync(archive)
  if (sha256(archiveBytes) !== result.artifact.sha256 || archiveBytes.length !== result.artifact.size_bytes) {
    throw new Error('Changed previously accepted package')
  }
  const packageCommit = inputs.package_commit ?? inputs.commit
  if (!/^[a-f0-9]{40}$/.test(packageCommit)) throw new Error('Missing package source identity')
  return { archive, packageCommit, toolSHA256: inputs.tool_sha256, sourceRun: id }
}

// A prepared Windows package is build/verification evidence, not administrator
// installation evidence. Never open its private signing inputs for a download check.
export function preparedWindowsPackage(root, id, version, commit) {
  if (!/^[a-f0-9]{12}$/.test(id)) throw new Error('Invalid Windows fixture ID')
  const runRoot = join(root, 'target/wp', id)
  const receipt = JSON.parse(readFileSync(join(runRoot, 'build-receipt.json')))
  if (receipt.schema !== 'aster.windows-pipeline-fixture.v1' || receipt.source_clean !== true
    || !/^[a-f0-9]{64}$/.test(receipt.fingerprint) || receipt.fingerprint.slice(0, 12) !== id
    || receipt.inputs?.version !== version || receipt.inputs?.head !== commit
    || !receipt.steps?.base || !receipt.steps?.candidate) throw new Error('A completed Windows preparation for the current source is required')
  const name = `aster-team-${version}-windows-amd64.tar.gz`
  const archive = join(runRoot, 'packages/windows', name)
  for (const path of [archive, `${archive}.sha256`]) {
    if (sha256(readFileSync(path)) !== receipt.steps.base[path]) throw new Error('Changed prepared Windows package')
  }
  return { archive, packageCommit: receipt.inputs.head, sourceRun: id }
}
