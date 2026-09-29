import assert from 'node:assert/strict'
import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { imageTargets, maintenanceTargets, parseArguments, publishedReference, publishImages, recipeHash,
  validateImage, validateReceipt, validateRunningImage } from './base-images.mjs'

const manifest = JSON.parse(readFileSync(new URL('../../contracts/release-platforms.json', import.meta.url), 'utf8'))
const targets = imageTargets(manifest)
const target = targets[0]
const hash = 'a'.repeat(64)
const id = `sha256:${'b'.repeat(64)}`
const digest = `sha256:${'c'.repeat(64)}`
const info = {
  Id: id, Os: 'linux', Architecture: 'amd64', RepoDigests: [`${target.repository}@${digest}`],
  Config: { Cmd: ['/sbin/init'], Labels: { 'io.aster.ci.recipe': hash, 'io.aster.ci.target': target.id,
    'org.opencontainers.image.source': 'https://github.com/huzz-open/aster-team' } },
}
const item = { target: target.id, tag: `${target.repository}:20260905-12345678`,
  image_id: id, base: `ubuntu@${digest}`, recipe: hash, repository: target.repository }
const receipt = { schema_version: 1, images: [item] }

test('default is a read-only plan and modes are mutually exclusive', () => {
  assert.equal(parseArguments([]), 'plan')
  for (const mode of ['plan', 'build', 'publish', 'update', 'help']) assert.equal(parseArguments([`--${mode}`]), mode)
  assert.throws(() => parseArguments(['--build', '--publish']))
  assert.throws(() => parseArguments(['--skip-validation']))
})

test('all six Linux lab runtimes are covered and the compiler has a separate image', () => {
  assert.deepEqual(targets.map(value => value.id), ['ubuntu-20.04', 'ubuntu-22.04', 'ubuntu-24.04', 'debian-12', 'debian-13', 'rocky-linux-9'])
  assert.equal(maintenanceTargets(manifest).at(-1).kind, 'builder')
  assert.equal(maintenanceTargets(manifest).length, 7)
  assert.equal(targets.at(-1).os, 'rocky')
  assert.ok(targets.every(value => value.repository.startsWith('ghcr.io/huzz-open/aster-team-ci-')))
})

test('invalid, duplicated or empty targets fail closed', () => {
  for (const mutate of [
    value => { value.platforms[0].smoke_targets[0].id = '../../secrets' },
    value => { value.platforms[0].smoke_targets.push(value.platforms[0].smoke_targets[0]) },
    value => { value.platforms[0].smoke_targets[0].image = '--secret=anything' },
    value => { value.platforms[0].smoke_targets[0].package_family = 'dnf' },
    value => { value.platforms[0].smoke_targets = [] },
  ]) {
    const changed = structuredClone(manifest)
    mutate(changed)
    assert.throws(() => imageTargets(changed))
  }
})

test('receipt hashes bind recipe, target and validator, independent of Windows line endings', () => {
  const original = recipeHash(target, 'FROM base\n', 'validate\n', 'script\n')
  assert.equal(original, recipeHash(target, 'FROM base\r\n', 'validate\r\n', 'script\r\n'))
  assert.notEqual(original, recipeHash(target, 'FROM other\n', 'validate\n', 'script\n'))
  assert.notEqual(original, recipeHash(target, 'FROM base\n', 'new validation\n', 'script\n'))
  assert.notEqual(original, recipeHash(target, 'FROM base\n', 'validate\n', 'new script\n'))
})

test('image identity rejects architecture, command, source and recipe mismatches', () => {
  validateImage(info, target, hash)
  for (const mutate of [
    value => { value.Architecture = 'arm64' }, value => { value.Os = 'windows' },
    value => { value.Id = 'tag' }, value => { value.Config.Cmd = ['/bin/sh'] },
    value => { value.Config.Labels['io.aster.ci.recipe'] = 'changed' },
    value => { value.Config.Labels['org.opencontainers.image.source'] = 'another repository' },
  ]) {
    const changed = structuredClone(info)
    mutate(changed)
    assert.throws(() => validateImage(changed, target, hash))
  }
})

test('published references must match the exact repository and an immutable digest', () => {
  assert.equal(publishedReference(info, target.repository), `${target.repository}@${digest}`)
  for (const RepoDigests of [[], [`${target.repository}:latest`], [`evil/${target.repository}@${digest}`],
    [`${target.repository}@${digest}`, `${target.repository}@${id}`]]) {
    assert.throws(() => publishedReference({ RepoDigests }, target.repository))
  }
})

test('publication resumes only from unchanged validated local image IDs and current recipes', () => {
  assert.equal(validateReceipt(receipt, [target], { [target.id]: hash }, () => info).length, 1)
  assert.throws(() => validateReceipt(receipt, targets, { [target.id]: hash }, () => info))
  assert.throws(() => validateReceipt(receipt, [target], { [target.id]: 'changed' }, () => info))
  assert.throws(() => validateReceipt(receipt, [target], { [target.id]: hash }, () => ({ ...info, Id: digest })))
  const changed = structuredClone(receipt)
  changed.images[0].tag = 'ghcr.io/someone/other:latest'
  assert.throws(() => validateReceipt(changed, [target], { [target.id]: hash }, () => info))
})

test('systemd validation is isolated and removes only its own container', () => {
  const calls = []
  validateRunningImage(args => calls.push(args), id, target, 'validate')
  assert.ok(calls[0].includes('--network') && calls[0].includes('none'))
  assert.ok(calls[1].includes('--privileged'))
  assert.ok(calls[2].includes('exec'))
  assert.equal(calls[3][0], 'rm')
  assert.equal(calls[3][2], calls[1][calls[1].indexOf('--name') + 1])
  assert.ok(!JSON.stringify(calls).includes('/workspace'))
})

test('failed runtime validation still cleans up and never publishes', () => {
  const calls = []
  assert.throws(() => validateRunningImage(args => {
    calls.push(args)
    if (args[0] === 'exec') throw new Error('systemd did not boot')
  }, id, target, 'validate'), /systemd did not boot/)
  assert.equal(calls.at(-2)[0], 'logs')
  assert.equal(calls.at(-1)[0], 'rm')
  assert.ok(!calls.some(args => args[0] === 'push'))
})

test('publishing pulls the exact digest back before updating the lock; no rebuild or retest', () => {
  const calls = []
  let saved
  publishImages(args => { calls.push(args); return JSON.stringify([info]) }, [item], value => { saved = value })
  assert.deepEqual(calls.map(args => args[0]), ['push', 'image', 'pull', 'image'])
  assert.equal(calls[2].at(-1), `${target.repository}@${digest}`)
  assert.equal(saved.images[0].image, `${target.repository}@${digest}`)
})

test('upload or pull-back failure never writes a partial lock', () => {
  for (const failure of ['push', 'pull']) {
    let saved = false
    assert.throws(() => publishImages(args => {
      if (args[0] === failure) throw new Error('network failed')
      return JSON.stringify([info])
    }, [item], () => { saved = true }), /network failed/)
    assert.equal(saved, false)
  }
  let saved = false
  let pushes = 0
  assert.throws(() => publishImages(args => {
    if (args[0] === 'push' && ++pushes === 2) throw new Error('second image failed')
    return JSON.stringify([info])
  }, [item, item], () => { saved = true }))
  assert.equal(saved, false)
})

test('plan runs without Docker and maintenance refuses execution in Actions', () => {
  const script = fileURLToPath(new URL('./base-images.mjs', import.meta.url))
  const planned = spawnSync(process.execPath, [script, '--plan'], { encoding: 'utf8',
    env: { ...process.env, GITHUB_ACTIONS: 'false' } })
  assert.equal(planned.status, 0, planned.stderr)
  assert.equal(JSON.parse(planned.stdout).targets.length, 7)
  const refused = spawnSync(process.execPath, [script, '--update'], { encoding: 'utf8',
    env: { ...process.env, GITHUB_ACTIONS: 'true' } })
  assert.equal(refused.status, 1)
  assert.match(refused.stderr, /local-only/)
})
