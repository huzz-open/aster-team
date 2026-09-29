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

if (process.platform !== 'linux' || process.arch !== 'x64') {
  throw new Error('the Linux upgrade fixture must be built on Linux amd64')
}
withWorkspaceReleaseVersion({ root, cargo: 'cargo', baseVersion, candidateVersion }, version => {
  if (prepareOnly) return
  runCommand(root, process.execPath, [resolve(root, 'scripts/build-linux-bundle.mjs'), '--arch=amd64', `--version=${version}`], {
    env: { ASTER_OVERWRITE: 'true' },
  })
  const archive = resolve(root, 'dist', 'linux', `aster-team-${version}-linux-amd64.tar.gz`)
  if (!existsSync(archive)) throw new Error(`candidate archive was not created: ${archive}`)
  if (process.env.GITHUB_OUTPUT) {
    appendFileSync(process.env.GITHUB_OUTPUT, `candidate_version=${version}\ncandidate_archive=${archive}\n`)
  }
  console.log(`Linux upgrade fixture: ${archive}`)
})
if (prepareOnly) console.log(`Linux upgrade fixture metadata prepared for ${candidateVersion}`)
