#!/usr/bin/env node
import { spawnSync } from 'node:child_process'
import { createHash, randomUUID } from 'node:crypto'
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { resolveWindowsDockerRuntime } from '../windows-docker-runtime.mjs'

const root = fileURLToPath(new URL('../../', import.meta.url))
const receiptDirectory = resolve(process.env.ASTER_CI_IMAGES_RECEIPT_DIR || resolve(root, 'target/ci-base-images'))
const receiptPath = resolve(receiptDirectory, 'validated.json')
const lockPath = resolve(root, 'tools/ci-base-images.lock.json')
const source = 'https://github.com/huzz-open/aster-team'
const prefix = 'ghcr.io/huzz-open/aster-team-ci-'
const digestPattern = /^sha256:[a-f0-9]{64}$/

export function parseArguments(args) {
  if (args.length === 0) return 'plan'
  if (args.length !== 1 || !['--plan', '--build', '--resume', '--publish', '--update', '--help'].includes(args[0])) {
    throw new Error('Use one of --plan, --build, --resume, --publish, --update, --help')
  }
  return args[0].slice(2)
}

export function imageTargets(manifest) {
  const targets = manifest.platforms?.find(item => item.id === 'linux-amd64')?.smoke_targets
    ?.map(item => ({ ...item, image: item.image || `ubuntu:${item.id.replace('ubuntu-', '')}`,
      package_family: item.package_family || 'apt' }))
  if (!targets?.length) throw new Error('No Linux container smoke targets configured')
  const ids = new Set()
  return targets.map(target => {
    if (!/^(ubuntu-\d{2}\.\d{2}|debian-\d+|rocky-linux-\d+)$/.test(target.id)
      || ids.has(target.id) || !['apt', 'dnf'].includes(target.package_family)
      || !/^[a-z][a-z0-9/-]*:[a-z0-9.-]+$/.test(target.image)) {
      throw new Error(`Invalid or duplicate container target: ${target.id}`)
    }
    ids.add(target.id)
    const [os, version] = target.id.replace('rocky-linux-', 'rocky-').split('-')
    if ((os === 'rocky' ? 'dnf' : 'apt') !== target.package_family) throw new Error('OS/package family mismatch')
    return { id: target.id, base: target.image, family: target.package_family, os, version,
      kind: 'runtime', repository: `${prefix}${target.id}` }
  })
}

export function maintenanceTargets(manifest) {
  return [...imageTargets(manifest), { id: 'linux-builder', kind: 'builder', family: 'toolchain',
    os: 'debian', version: '12', base: 'rust:1.95.0-slim-bookworm@sha256:d7482085ff5b415f84dba5647ae71606650bdef00db7aeb69f4b3d170c3e4082',
    repository: `${prefix}linux-builder` }]
}

export function recipeHash(target, dockerfile, validator, maintainer) {
  return createHash('sha256').update(JSON.stringify(target)).update(dockerfile.replace(/\r\n/g, '\n'))
    .update(validator.replace(/\r\n/g, '\n')).update(maintainer.replace(/\r\n/g, '\n')).digest('hex')
}

export function validateImage(info, target, hash) {
  if (info.Os !== 'linux' || info.Architecture !== 'amd64' || !digestPattern.test(info.Id || '')
    || info.Config?.Labels?.['io.aster.ci.recipe'] !== hash
    || info.Config?.Labels?.['io.aster.ci.target'] !== target.id
    || info.Config?.Labels?.['org.opencontainers.image.source'] !== source
    || JSON.stringify(info.Config?.Cmd) !== JSON.stringify(target.kind === 'builder' ? ['bash'] : ['/sbin/init'])) {
    throw new Error(`Image identity/architecture/recipe mismatch: ${target.id}`)
  }
}

export function publishedReference(info, repository) {
  const references = (info.RepoDigests || []).filter(value => value.startsWith(`${repository}@`)
    && digestPattern.test(value.slice(repository.length + 1)))
  if (references.length !== 1) throw new Error(`Missing or ambiguous published digest: ${repository}`)
  return references[0]
}

export function validateReceipt(receipt, targets, hashes, inspect) {
  if (receipt.schema_version !== 1 || !Array.isArray(receipt.images)
    || receipt.images.length !== targets.length) throw new Error('Build all configured images first: npm run ci:images:build')
  return targets.map(target => {
    const matches = receipt.images.filter(item => item.target === target.id)
    const item = matches[0]
    if (matches.length !== 1 || item.recipe !== hashes[target.id]
      || !item.tag?.startsWith(`${target.repository}:`)
      || !/^[a-z0-9][a-z0-9.-]+$/.test(item.tag.slice(target.repository.length + 1))
      || !digestPattern.test(item.image_id || '') || !item.base?.includes('@sha256:')) {
      throw new Error(`Missing/stale validation receipt: ${target.id}; run npm run ci:images:build`)
    }
    const info = inspect(item.tag)
    validateImage(info, target, hashes[target.id])
    if (info.Id !== item.image_id) throw new Error(`Validated image has changed: ${target.id}`)
    return { ...item, repository: target.repository }
  })
}

function atomicJson(path, value) {
  mkdirSync(dirname(path), { recursive: true })
  const temporary = `${path}.${randomUUID()}.tmp`
  writeFileSync(temporary, `${JSON.stringify(value, null, 2)}\n`, { flag: 'wx' })
  renameSync(temporary, path)
}

function dockerClient() {
  const runtime = process.platform === 'win32' ? resolveWindowsDockerRuntime() : {
    executable: 'docker', environment: process.env,
  }
  return (args, { capture = false, timeout = 120_000, allowFailure = false } = {}) => {
    const result = spawnSync(runtime.executable, args, {
      cwd: root, env: runtime.environment, windowsHide: true, encoding: 'utf8', timeout,
      maxBuffer: 8 * 1024 * 1024, stdio: capture ? 'pipe' : 'inherit',
    })
    if (result.error || result.status !== 0) {
      if (allowFailure) return null
      throw new Error(`docker ${args[0]} failed${result.error ? `: ${result.error.message}` : ` (exit ${result.status})`}`)
    }
    return capture ? result.stdout.trim() : ''
  }
}

export function validateRunningImage(docker, image, target, validator) {
  // No repository, credential directory or product payload is mounted into this container.
  docker(['run', '--rm', '--network', 'none', '--entrypoint', '/bin/sh', image, '-ec', validator,
    'validate-base', target.os, target.version])
  if (target.kind === 'builder') return
  const name = `aster-base-check-${randomUUID()}`
  let started = false
  try {
    docker(['run', '--detach', '--name', name, '--privileged', '--cgroupns=host',
      '--network', 'none', '--tmpfs', '/run', '--tmpfs', '/run/lock', '--tmpfs', '/sys:rw',
      '--volume', '/sys/fs/cgroup:/sys/fs/cgroup:rw', image])
    started = true
    docker(['exec', name, '/bin/sh', '-ec',
      'printf "[Service]\\nType=oneshot\\nExecStart=/bin/true\\nRemainAfterExit=yes\\n" > /etc/systemd/system/aster-base-probe.service; for i in $(seq 1 60); do if systemctl daemon-reload && systemctl start aster-base-probe.service && systemctl is-active --quiet aster-base-probe.service; then exit 0; fi; sleep 1; done; exit 1'],
    { timeout: 75_000 })
  } catch (error) {
    if (started) docker(['logs', '--tail', '80', name], { allowFailure: true })
    throw error
  } finally {
    if (started) docker(['rm', '--force', name])
  }
}

export function publishImages(docker, images, saveLock) {
  const published = []
  for (const item of images) {
    console.log(`Publishing validated image: ${item.tag}`)
    docker(['push', item.tag], { timeout: 30 * 60_000 })
    const info = JSON.parse(docker(['image', 'inspect', item.tag], { capture: true }))[0]
    if (info.Id !== item.image_id) throw new Error(`Image changed while publishing: ${item.target}`)
    const image = publishedReference(info, item.repository)
    docker(['pull', '--platform', 'linux/amd64', image], { timeout: 10 * 60_000 })
    const remote = JSON.parse(docker(['image', 'inspect', image], { capture: true }))[0]
    if (remote.Id !== item.image_id) throw new Error(`Published image differs from validated image: ${item.target}`)
    published.push({ target: item.target, image, base: item.base, recipe: item.recipe,
      image_id: item.image_id })
  }
  // Never switch only part of the matrix when a later upload fails.
  saveLock({ schema_version: 1, platform: 'linux/amd64', images: published })
}

function main() {
  const args = process.argv.slice(2)
  const selections = args.filter(arg => arg.startsWith('--only='))
  if (selections.length > 1) throw new Error('Use --only once')
  const only = selections[0]?.slice(7)
  const mode = parseArguments(args.filter(arg => !arg.startsWith('--only=')))
  if (only !== undefined && (!only || !['build', 'resume'].includes(mode))) throw new Error('--only requires --build or --resume and a target')
  if (mode === 'help') {
    console.log('Local GHCR maintenance (never called by Actions):\n'
      + '  npm run ci:images:plan     Preview without Docker/network/writes\n'
      + '  npm run ci:images:build    Refresh, build and validate all test bases locally\n'
      + '  npm run ci:images:resume   Resume unchanged successful image validations\n'
      + '  npm run ci:images:publish  Publish the unchanged validated images (safe retry)\n'
      + '  npm run ci:images:update   Build, validate and publish\n'
      + 'Requires Node 22+, Linux Docker and docker login ghcr.io for publishing.\n'
      + 'Systemd validation uses disposable privileged containers: use an isolated Docker host/VM.')
    return
  }
  if (process.env.GITHUB_ACTIONS === 'true') throw new Error('Image maintenance is local-only; Actions must consume pinned images')
  const targets = maintenanceTargets(JSON.parse(readFileSync(resolve(root, 'contracts/release-platforms.json'), 'utf8')))
  if (only && !targets.some(target => target.id === only)) throw new Error(`Unknown image: ${only}`)
  const validators = Object.fromEntries(targets.map(target => [target.id, readFileSync(resolve(root,
    `scripts/ci/systemd/validate-${target.kind === 'builder' ? 'toolchain' : 'base'}.sh`), 'utf8').replace(/\r\n/g, '\n')]))
  const maintainer = readFileSync(fileURLToPath(import.meta.url), 'utf8')
  const dockerfile = target => target.kind === 'builder' ? 'scripts/ci/linux-lab.Dockerfile' : `scripts/ci/systemd/${target.family}.Dockerfile`
  const hashes = Object.fromEntries(targets.map(target => [target.id, recipeHash(target,
    readFileSync(resolve(root, dockerfile(target)), 'utf8'), validators[target.id], maintainer)]))
  const checkpoint = target => resolve(receiptDirectory, `${target.id}.json`)
  const mirrorArguments = target => {
    const mirrors = target.kind === 'builder' || target.os === 'debian'
      ? [process.env.ASTER_CI_DEBIAN_MIRROR, process.env.ASTER_CI_DEBIAN_SECURITY_MIRROR]
      : target.os === 'ubuntu'
        ? [process.env.ASTER_CI_UBUNTU_MIRROR, process.env.ASTER_CI_UBUNTU_SECURITY_MIRROR]
        : []
    const names = target.kind === 'builder' ? ['DEBIAN_MIRROR', 'DEBIAN_SECURITY_MIRROR'] : ['APT_MIRROR', 'APT_SECURITY_MIRROR']
    return mirrors.flatMap((value, index) => value ? ['--build-arg', `${names[index]}=${value}`] : [])
  }
  if (mode === 'plan') {
    console.log(JSON.stringify({ platform: 'linux/amd64', targets, receipt: receiptPath, lock: lockPath }, null, 2))
    return
  }
  const docker = dockerClient()
  if (docker(['info', '--format', '{{.OSType}}'], { capture: true }) !== 'linux') throw new Error('Linux Docker Engine required')
  const inspect = image => JSON.parse(docker(['image', 'inspect', image], { capture: true }))[0]
  if (['build', 'resume', 'update'].includes(mode)) {
    const images = []
    const batch = `${new Date().toISOString().replace(/[^0-9]/g, '')}-${randomUUID().slice(0, 8)}`
    for (const target of targets.filter(target => !only || target.id === only)) {
      if (mode === 'resume' && existsSync(checkpoint(target))) {
        const previous = JSON.parse(readFileSync(checkpoint(target), 'utf8'))
        if (previous.recipe === hashes[target.id]) {
          const [validated] = validateReceipt({ schema_version: 1, images: [previous] }, [target], hashes, inspect)
          images.push(validated)
          console.log(`Reusing unchanged validated image: ${target.id}`)
          continue
        }
        console.log(`Validation inputs changed; rebuild from matching Docker layers and revalidate: ${target.id}`)
      }
      docker(['pull', '--platform', 'linux/amd64', target.base], { timeout: 10 * 60_000 })
      const base = publishedReference(inspect(target.base), target.base.split(':')[0])
      const tag = `${target.repository}:${batch}`
      // A fresh maintenance build refreshes packages. Resume may reuse matching completed
      // Docker layers, but ALWAYS validates a rebuilt image before creating a new receipt.
      docker(['build', ...(mode === 'resume' ? [] : ['--no-cache']), '--platform', 'linux/amd64',
        '--build-arg', `BASE_IMAGE=${base}`, ...mirrorArguments(target), '--label', `org.opencontainers.image.source=${source}`,
        '--label', `io.aster.ci.target=${target.id}`, '--label', `io.aster.ci.recipe=${hashes[target.id]}`,
        '--file', dockerfile(target), '--tag', tag, 'scripts/ci/systemd'],
      { timeout: 60 * 60_000 })
      const info = inspect(tag)
      validateImage(info, target, hashes[target.id])
      validateRunningImage(docker, info.Id, target, validators[target.id])
      const validated = { target: target.id, tag, base, image_id: info.Id, recipe: hashes[target.id] }
      atomicJson(checkpoint(target), validated)
      images.push(validated)
    }
    if (!only) atomicJson(receiptPath, { schema_version: 1, images })
    console.log(only ? `Validated ${only}. Receipt: ${checkpoint(targets.find(target => target.id === only))}`
      : `All CI bases validated. Receipt: ${receiptPath}`)
  }
  if (mode === 'publish' || mode === 'update') {
    const images = validateReceipt({ schema_version: 1,
      images: targets.map(target => JSON.parse(readFileSync(checkpoint(target), 'utf8'))) }, targets, hashes, inspect)
    publishImages(docker, images, lock => atomicJson(lockPath, lock))
    console.log(`Published and pulled back every pinned image. Review ${lockPath}; no Git push or Actions run was triggered.`)
  }
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  try { main() } catch (error) { console.error(error.message); process.exitCode = 1 }
}
