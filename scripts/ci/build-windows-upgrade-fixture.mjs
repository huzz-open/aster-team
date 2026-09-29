import { appendFileSync, existsSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import {
  nextPatchFixtureVersion, runCommand, withWorkspaceReleaseVersion,
} from './workspace-release-fixture.mjs'

const root = resolve(fileURLToPath(new URL('../..', import.meta.url)))
const argument = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
const baseVersion = argument('--base-version').replace(/^v/, '')
const candidateVersion = nextPatchFixtureVersion(baseVersion)
const prepareOnly = process.argv.includes('--prepare-only')

if (process.platform !== 'win32') throw new Error('the Windows upgrade fixture must be built on Windows')
withWorkspaceReleaseVersion({ root, cargo: 'cargo.exe', baseVersion, candidateVersion }, version => {
  if (prepareOnly) return
  runCommand(root, process.execPath, [resolve(root, 'scripts/build-windows-bundle.mjs'), `--version=${version}`])
  const outputRoot = resolve(root, process.env.ASTER_RELEASE_OUTPUT_ROOT || 'dist')
  const archive = resolve(outputRoot, 'windows', `aster-team-${version}-windows-amd64.tar.gz`)
  if (!existsSync(archive)) throw new Error(`candidate archive was not created: ${archive}`)
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `candidate_version=${version}\ncandidate_archive=${archive}\n`)
  }
  console.log(`Windows upgrade fixture: ${archive}`)
})
if (prepareOnly) console.log(`Windows upgrade fixture metadata prepared for ${candidateVersion}`)
