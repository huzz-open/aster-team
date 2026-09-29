import { appendFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { command } from '../release-github.mjs'
import { parseReleaseVersionArguments } from '../prepare-release-version.mjs'

const repository = 'huzz-open/aster-team'
const requiredJobs = [
  'Audit release dependencies',
  'Build downloadable asterctl (Windows x64)',
  'Build and verify Windows amd64 package',
  'Build signed Linux amd64 package',
  'Verify Linux primary (ubuntu-20.04)',
  'Require Linux verification and Windows package build',
]

export function verifyReleaseRun({ env = process.env, run = (exe, args) => command(process.cwd(), exe, args) } = {}) {
  const tag = env.RELEASE_TAG
  const runId = env.SOURCE_RUN_ID
  if (env.GITHUB_EVENT_NAME !== 'workflow_dispatch' || env.GITHUB_REF !== 'refs/heads/main'
    || env.GITHUB_REPOSITORY !== repository || !/^v[0-9]+\.[0-9]+\.[0-9]+(?:[-+][A-Za-z0-9.-]+)?$/.test(tag || '')
    || !/^[1-9][0-9]*$/.test(runId || '')) throw new Error('Recovery requires a version tag and an existing run dispatched from main')
  const version = parseReleaseVersionArguments(['--version', tag.slice(1)]).version
  if (tag !== `v${version}`) throw new Error('Invalid release version tag')
  const refs = run('git', ['ls-remote', 'origin', `refs/tags/${tag}`, `refs/tags/${tag}^{}`])
    .split('\n').filter(Boolean).map(line => line.split(/\s+/))
  const sha = refs.find(([, ref]) => ref === `refs/tags/${tag}^{}`)?.[0]
  if (!/^[a-f0-9]{40}$/.test(sha || '')) throw new Error('Immutable annotated release tag not found')
  run('git', ['merge-base', '--is-ancestor', sha, 'origin/main'])
  const api = path => JSON.parse(run('gh', ['api', `repos/${repository}/${path}`]))
  const source = api(`actions/runs/${runId}`)
  if (source.id !== Number(runId) || source.event !== 'push' || source.head_branch !== tag
    || source.head_sha !== sha || source.path !== '.github/workflows/customer-release.yml'
    || source.status !== 'completed' || source.conclusion !== 'success') {
    throw new Error('Source run is not a successful release of the exact immutable tag')
  }
  const jobs = api(`actions/runs/${runId}/jobs?per_page=100`)
  if (jobs.total_count !== jobs.jobs?.length || jobs.total_count >= 100) throw new Error('Source run job list is incomplete')
  for (const name of requiredJobs) {
    if (jobs.jobs.filter(job => job.name === name && job.conclusion === 'success').length !== 1) {
      throw new Error(`Source release gate is missing successful job: ${name}`)
    }
  }
  for (const pattern of [/^Verify customer install \(/, /^Verify dedicated Runner install \((?!windows-2025\))/]) {
    const matches = jobs.jobs.filter(job => pattern.test(job.name))
    if (matches.length !== 5 || matches.some(job => job.conclusion !== 'success')) {
      throw new Error('Source Linux installation matrix is incomplete')
    }
  }
  const artifacts = api(`actions/runs/${runId}/artifacts?per_page=100`)
  for (const name of [`customer-linux-amd64-${version}`, `customer-windows-amd64-${version}`]) {
    if (artifacts.artifacts?.filter(item => item.name === name && !item.expired && item.workflow_run?.head_sha === sha).length !== 1) {
      throw new Error(`Verified release artifact is unavailable: ${name}`)
    }
  }
  const result = { tag, version, sha, runId }
  if (env.GITHUB_OUTPUT) appendFileSync(env.GITHUB_OUTPUT,
    `tag=${tag}\nversion=${version}\nsha=${sha}\nrun_id=${runId}\n`)
  return result
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  try { console.log(verifyReleaseRun()) } catch (error) { console.error(error.message); process.exitCode = 1 }
}
