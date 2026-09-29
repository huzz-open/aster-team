#!/usr/bin/env node
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { parseReleaseVersionArguments } from './prepare-release-version.mjs'
import { releaseGithub } from './release-github.mjs'

export function parseTagArguments(args) {
  if (args.includes('--help') || args.includes('-h')) return { help: true }
  const { version } = parseReleaseVersionArguments(args.filter(arg => arg !== '--tag-only'))
  return { version, tag: true, dispatchOnly: args.includes('--tag-only') }
}

const invoked = process.argv[1] ? resolve(process.argv[1]) : ''
const current = fileURLToPath(import.meta.url)
if (process.platform === 'win32' ? invoked.toLowerCase() === current.toLowerCase() : invoked === current) {
  try {
    const options = parseTagArguments(process.argv.slice(2))
    if (options.help) {
      console.log(`Usage:
  npm run release:tag -- --version VERSION
  npm run release:tag -- --version VERSION --tag-only

Synchronize clean main, prepare and commit the four version files, push main,
then create and push annotated vVERSION. The tag triggers public GitHub Actions,
which publishes signed Linux and Windows packages to huzz-open/aster-team after
all release checks succeed. Prefer --tag-only after the version PR has merged.
Requires Git, Cargo and authenticated GitHub CLI. No local verification suites.
Git hooks and remote protection remain in effect. Existing tags are never moved.
--tag-only uses the already-committed version without preparing another commit.
If a push failed, push the retained main commit before using --tag-only.
An existing remote tag is not pushed again; rerun its Actions run on failure.`)
    } else releaseGithub({ options })
  } catch (error) {
    console.error(`error: ${error.message}`)
    process.exitCode = 1
  }
}
