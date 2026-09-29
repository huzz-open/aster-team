import assert from 'node:assert/strict'
import { mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import {
  MAX_BUNDLED_FREE_LICENSE_BYTES,
  freezeBundledFreeLicense,
  stageBundledFreeLicense,
  verifyStagedBundledFreeLicense,
} from './bundled-free-license.mjs'

test('bundled free license input is frozen once and shares the installer size boundary', () => {
  const directory = resolve(tmpdir(), `aster-bundled-license-${process.pid}-${Date.now()}`)
  try {
    mkdirSync(directory, { recursive: true })
    const source = resolve(directory, 'source.json')
    writeFileSync(source, Buffer.alloc(MAX_BUNDLED_FREE_LICENSE_BYTES, 0x20))
    const frozen = freezeBundledFreeLicense(source)
    writeFileSync(source, Buffer.from('replaced after freeze'))

    const bundle = resolve(directory, 'bundle')
    const target = stageBundledFreeLicense(bundle, frozen)
    assert.equal(readFileSync(target).length, MAX_BUNDLED_FREE_LICENSE_BYTES)
    assert.notDeepEqual(readFileSync(target), readFileSync(source))

    writeFileSync(source, Buffer.alloc(MAX_BUNDLED_FREE_LICENSE_BYTES + 1, 0x20))
    assert.throws(() => freezeBundledFreeLicense(source), /between 1 byte and 64 KiB/)
    assert.throws(() => freezeBundledFreeLicense(directory), /between 1 byte and 64 KiB/)
    assert.throws(() => stageBundledFreeLicense(resolve(directory, 'oversized'), Buffer.alloc(MAX_BUNDLED_FREE_LICENSE_BYTES + 1)), /bytes are invalid/)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('staged license verification uses the immutable release Control copy', () => {
  const directory = resolve(tmpdir(), `aster-bundled-verifier-${process.pid}-${Date.now()}`)
  try {
    const bundle = resolve(directory, 'bundle')
    const target = resolve(bundle, 'licenses/free-license.json')
    mkdirSync(resolve(bundle, 'bin'), { recursive: true })
    mkdirSync(resolve(bundle, 'licenses'), { recursive: true })
    writeFileSync(resolve(bundle, 'bin/aster-control'), 'release-copy')
    writeFileSync(target, 'signed-license')
    writeFileSync(resolve(directory, 'mutable-target-control'), 'replacement-build-output')

    let invocation
    verifyStagedBundledFreeLicense(bundle, target, (binary, args) => {
      invocation = { binary, args, binaryBytes: readFileSync(binary, 'utf8') }
    })
    assert.deepEqual(invocation, {
      binary: resolve(bundle, 'bin/aster-control'),
      args: ['verify-bundled-free-license', '--source', target],
      binaryBytes: 'release-copy',
    })
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})


test('Windows bundle verification invokes the shipped exe and rejects unknown platforms', () => {
  const bundle = resolve(tmpdir(), 'aster-windows-verifier-contract')
  const target = resolve(bundle, 'licenses/free-license.json')
  const calls = []
  verifyStagedBundledFreeLicense(bundle, target, (...args) => calls.push(args), 'windows')
  assert.deepEqual(calls, [[resolve(bundle, 'bin/aster-control.exe'), ['verify-bundled-free-license', '--source', target]]])
  assert.throws(() => verifyStagedBundledFreeLicense(bundle, target, () => { throw new Error('should not execute') }, 'unknown'), /unsupported/)
})
