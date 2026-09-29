import { execFileSync } from 'node:child_process'
import { readFileSync } from 'node:fs'
import { extname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import { npmCommand, repositoryRoot, runCommands } from './verification-runner.mjs'

const manifestPath = resolve(repositoryRoot, 'tools/verification-map.json')

export const SUITE_ORDER = [
  'documentation',
  'common-policy',
  'tooling-tests',
  'ui-policy',
  'customer-domain',
  'operations-domain',
  'customer-admin',
  'customer-member',
  'customer-sdk',
  'operations-console',
  'website',
  'rust-workspace',
  'operations-go',
]

export const SUITE_COMMANDS = {
  documentation: [
    npmCommand('documentation', 'run', 'verify:docs'),
  ],
  'common-policy': [
    npmCommand('release-boundaries', 'run', 'verify:boundaries'),
    npmCommand('contracts', 'run', 'verify:contracts'),
  ],
  'tooling-tests': [
    npmCommand('tooling-tests', 'run', 'test:boundaries'),
    npmCommand('system-e2e-unit-tests', 'run', 'test:system-e2e:unit'),
  ],
  'ui-policy': [
    npmCommand('table-layout', 'run', 'verify:table-layout'),
    npmCommand('typography', 'run', 'verify:typography'),
  ],
  'customer-domain': [
    npmCommand('customer-domain', 'run', 'verify:customer'),
  ],
  'operations-domain': [
    npmCommand('operations-domain', 'run', 'verify:operations'),
  ],
  'customer-admin': [
    npmCommand('customer-admin-tests', 'test', '--workspace', '@aster/admin'),
    npmCommand('customer-admin-build', 'run', 'build', '--workspace', '@aster/admin'),
    npmCommand('customer-admin-assets', 'run', 'verify:customer-admin-assets'),
  ],
  'customer-member': [
    npmCommand('customer-member-build', 'run', 'build', '--workspace', '@aster/member'),
    npmCommand('customer-member-assets', 'run', 'verify:customer-member-assets'),
  ],
  'customer-sdk': [
    npmCommand('customer-sdk-tests', 'test', '--workspace', '@aster/sdk'),
  ],
  'operations-console': [
    npmCommand('operations-console-tests', 'test', '--workspace', '@aster/operations-console'),
    npmCommand('operations-console-build', 'run', 'build', '--workspace', '@aster/operations-console'),
  ],
  website: [
    npmCommand('website-tests', 'test', '--workspace', '@aster/website'),
    npmCommand('website-build', 'run', 'build', '--workspace', '@aster/website'),
  ],
  'rust-workspace': [
    npmCommand('rust-format-and-lint', 'run', 'check:rust'),
    npmCommand('rust-tests', 'run', 'test:rust'),
    npmCommand('customer-logging', 'run', 'verify:customer-logging'),
  ],
  'operations-go': [
    npmCommand('operations-go-tests', 'run', 'test:go'),
    npmCommand('operations-go-build', 'run', 'build:go'),
  ],
}

const RISK_LEVELS = new Map([
  ['affected', 0],
  ['domain', 1],
  ['repo-full', 2],
])

const SUITE_SUPERSEDES = {
  'customer-domain': [
    'common-policy',
    'ui-policy',
    'customer-admin',
    'customer-member',
    'customer-sdk',
    'rust-workspace',
  ],
  'operations-domain': [
    'common-policy',
    'ui-policy',
    'operations-console',
    'operations-go',
  ],
}

function normalizePath(path) {
  return path.replaceAll('\\', '/').replace(/^\.\//, '')
}

function unique(values) {
  return [...new Set(values)]
}

function ruleMatches(path, rule) {
  return (rule.paths || []).includes(path)
    || (rule.prefixes || []).some(prefix => path.startsWith(prefix))
}

function validateManifest(manifest) {
  if (manifest?.schemaVersion !== 2 || !Array.isArray(manifest.rules) || !Array.isArray(manifest.extensionRules)) {
    throw new Error('tools/verification-map.json must use schemaVersion 2 and define rules and extensionRules')
  }
  for (const rule of [...manifest.rules, ...manifest.extensionRules]) {
    const level = rule.level || 'affected'
    if (!rule.reason || !RISK_LEVELS.has(level)
        || (level !== 'repo-full' && !Array.isArray(rule.suites))) {
      throw new Error(`Invalid verification rule: ${rule.id || rule.reason || 'unknown'}`)
    }
    for (const suite of rule.suites || []) {
      if (!SUITE_COMMANDS[suite]) throw new Error(`Unknown verification suite: ${suite}`)
    }
  }
}

export function resolveVerificationPlan(changedPaths, manifest) {
  validateManifest(manifest)
  const paths = unique(changedPaths.map(normalizePath).filter(Boolean)).sort()
  const suites = new Set()
  const reasons = []
  const unknownPaths = []
  let riskLevel = 0

  for (const path of paths) {
    const matchingRules = manifest.rules.filter(rule => ruleMatches(path, rule))
    const matchedSuites = new Set(matchingRules.flatMap(rule => rule.suites || []))
    const matchingExtensionRules = manifest.extensionRules.filter(rule => (
      (!matchingRules.length || (rule.always && rule.suites.some(suite => !matchedSuites.has(suite))))
      && rule.extensions.includes(extname(path).toLowerCase())
    ))
    const candidateMatches = [...matchingRules, ...matchingExtensionRules]

    if (!candidateMatches.length) {
      unknownPaths.push(path)
      riskLevel = RISK_LEVELS.get('repo-full')
      reasons.push(`${path}: no verification rule matched`)
      continue
    }

    const pathRiskLevel = Math.max(...candidateMatches.map(rule => RISK_LEVELS.get(rule.level || 'affected')))
    const matches = pathRiskLevel === RISK_LEVELS.get('repo-full')
      ? candidateMatches.filter(rule => rule.level === 'repo-full')
      : pathRiskLevel === RISK_LEVELS.get('domain')
        ? candidateMatches.filter(rule => rule.level === 'domain' || rule.always)
        : candidateMatches
    riskLevel = Math.max(riskLevel, pathRiskLevel)

    for (const rule of matches) {
      reasons.push(`${path}: ${rule.reason}`)
      for (const suite of rule.suites || []) suites.add(suite)
    }
  }

  if (riskLevel === RISK_LEVELS.get('repo-full')) {
    return {
      changedPaths: paths,
      commands: [npmCommand('full-verification', 'run', 'verify')],
      level: 'REPO_FULL',
      reasons: unique(reasons),
      suites: ['full'],
      unknownPaths,
    }
  }

  for (const suite of [...suites]) {
    for (const superseded of SUITE_SUPERSEDES[suite] || []) suites.delete(superseded)
  }

  const orderedSuites = SUITE_ORDER.filter(suite => suites.has(suite))
  const commands = []
  const commandIDs = new Set()
  for (const suite of orderedSuites) {
    for (const command of SUITE_COMMANDS[suite]) {
      if (commandIDs.has(command.id)) continue
      commandIDs.add(command.id)
      commands.push(command)
    }
  }

  return {
    changedPaths: paths,
    commands,
    level: paths.length
      ? riskLevel === RISK_LEVELS.get('domain') ? 'DOMAIN' : 'AFFECTED'
      : 'NONE',
    reasons: unique(reasons),
    suites: orderedSuites,
    unknownPaths,
  }
}

export function parseNameStatus(output) {
  const tokens = output.split('\0')
  const paths = []
  for (let index = 0; index < tokens.length;) {
    const status = tokens[index++]
    if (!status) continue
    const firstPath = tokens[index++]
    if (!firstPath) throw new Error(`git diff returned a pathless ${status} entry`)
    paths.push(firstPath)
    if (/^[RC]/.test(status)) {
      const secondPath = tokens[index++]
      if (!secondPath) throw new Error(`git diff returned an incomplete ${status} entry`)
      paths.push(secondPath)
    }
  }
  return paths
}

function git(arguments_, cwd = repositoryRoot) {
  return execFileSync('git', arguments_, { cwd, encoding: 'utf8' }).trim()
}

function diffPaths(arguments_, cwd) {
  const output = execFileSync('git', ['diff', '--name-status', '-z', '--find-renames', ...arguments_], {
    cwd,
    encoding: 'utf8',
  })
  return parseNameStatus(output)
}

export function changedPathsFromGit({ base = 'origin/main', cwd = repositoryRoot, head = 'HEAD' } = {}) {
  const mergeBase = git(['merge-base', base, head], cwd)
  const paths = diffPaths([mergeBase, head], cwd)

  if (head === 'HEAD') {
    paths.push(...diffPaths(['--cached'], cwd))
    paths.push(...diffPaths([], cwd))
    const untracked = execFileSync('git', ['ls-files', '--others', '--exclude-standard', '-z'], {
      cwd,
      encoding: 'utf8',
    }).split('\0').filter(Boolean)
    paths.push(...untracked)
  }

  return { mergeBase, paths: unique(paths.map(normalizePath)) }
}

function argument(name) {
  const prefix = `${name}=`
  return process.argv.find(value => value.startsWith(prefix))?.slice(prefix.length)
}

function displayPlan(plan, { base, head, mergeBase }) {
  console.log(`Verification level: ${plan.level}`)
  console.log(`Comparison: ${base} (${mergeBase || 'unavailable'}) ... ${head}`)
  console.log(`Changed files: ${plan.changedPaths.length}`)
  for (const path of plan.changedPaths) console.log(`- ${path}`)
  console.log(`Suites: ${plan.suites.length ? plan.suites.join(', ') : 'none'}`)
  console.log('Reasons:')
  for (const reason of plan.reasons) console.log(`- ${reason}`)
  if (!plan.reasons.length) console.log('- no changes detected')
  console.log('Commands:')
  for (const command of plan.commands) console.log(`- ${command.executable} ${command.arguments.join(' ')}`)
  if (!plan.commands.length) console.log('- none')
}

function main() {
  const base = argument('--base') || 'origin/main'
  const head = argument('--head') || 'HEAD'
  let changed
  let plan

  try {
    const manifest = JSON.parse(readFileSync(manifestPath, 'utf8'))
    changed = changedPathsFromGit({ base, head })
    plan = resolveVerificationPlan(changed.paths, manifest)
  } catch (error) {
    const reason = error instanceof Error ? error.message : String(error)
    plan = {
      changedPaths: [],
      commands: [npmCommand('full-verification', 'run', 'verify')],
      level: 'REPO_FULL',
      reasons: [`verification planning failed closed: ${reason}`],
      suites: ['full'],
      unknownPaths: [],
    }
    changed = { mergeBase: '' }
  }

  displayPlan(plan, { base, head, mergeBase: changed.mergeBase })
  if (process.argv.includes('--explain') || plan.level === 'NONE') return

  try {
    runCommands(plan.commands)
  } catch (error) {
    console.error(error instanceof Error ? error.message : error)
    process.exitCode = error?.exitCode || 1
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) main()
