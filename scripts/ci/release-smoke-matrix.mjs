#!/usr/bin/env node

import { readFile } from 'node:fs/promises'

const platformId = process.argv[2] || 'linux-amd64'
let onlyTarget = ''
const excludedTargets = new Set()
for (let index = 3; index < process.argv.length; index += 1) {
  const argument = process.argv[index]
  if (argument === '--only') {
    onlyTarget = process.argv[++index] || ''
    if (!onlyTarget) throw new Error('--only requires a smoke target id')
  } else if (argument === '--exclude') {
    const target = process.argv[++index] || ''
    if (!target) throw new Error('--exclude requires a smoke target id')
    excludedTargets.add(target)
  } else {
    throw new Error(`Unknown argument: ${argument}`)
  }
}
if (onlyTarget && excludedTargets.size > 0) {
  throw new Error('--only and --exclude cannot be combined')
}
const manifestUrl = new URL('../../contracts/release-platforms.json', import.meta.url)
const manifest = JSON.parse(await readFile(manifestUrl, 'utf8'))
const platform = manifest.platforms?.find(candidate => candidate.id === platformId)

if (!platform) {
  throw new Error(`Unknown release platform: ${platformId}`)
}
if (!Array.isArray(platform.smoke_targets) || platform.smoke_targets.length === 0) {
  throw new Error(`Release platform has no smoke targets: ${platformId}`)
}

const ids = new Set()
for (const target of platform.smoke_targets) {
  if (!target || typeof target !== 'object') throw new Error('Smoke target must be an object')
  if (!/^[a-z0-9][a-z0-9.-]*$/.test(target.id || '')) throw new Error(`Invalid smoke target id: ${target.id}`)
  if (ids.has(target.id)) throw new Error(`Duplicate smoke target id: ${target.id}`)
  ids.add(target.id)
  if (!/^ubuntu-[0-9]{2}\.[0-9]{2}$/.test(target.runner || '')) {
    throw new Error(`Invalid GitHub runner for ${target.id}: ${target.runner}`)
  }
  if (!['host', 'container'].includes(target.mode)) throw new Error(`Invalid smoke mode for ${target.id}: ${target.mode}`)
  if (target.mode === 'container') {
    if (!target.image || !['apt', 'dnf'].includes(target.package_family)) {
      throw new Error(`Container smoke target ${target.id} requires image and package_family`)
    }
  } else if (target.image || target.package_family) {
    throw new Error(`Host smoke target ${target.id} must not define container settings`)
  }
}

if (onlyTarget && !ids.has(onlyTarget)) throw new Error(`Unknown smoke target: ${onlyTarget}`)
for (const target of excludedTargets) {
  if (!ids.has(target)) throw new Error(`Unknown smoke target: ${target}`)
}
const selectedTargets = platform.smoke_targets.filter(target => {
  if (onlyTarget) return target.id === onlyTarget
  return !excludedTargets.has(target.id)
})
if (selectedTargets.length === 0) throw new Error('Smoke target selection is empty')

process.stdout.write(JSON.stringify(selectedTargets))
