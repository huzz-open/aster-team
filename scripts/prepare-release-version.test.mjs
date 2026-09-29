import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { spawnSync } from 'node:child_process'
import test from 'node:test'

import {
  compareSemanticVersions,
  parseReleaseVersionArguments,
  prepareReleaseVersion,
} from './prepare-release-version.mjs'

test('release version preparation accepts one normalized SemVer', () => {
  assert.deepEqual(parseReleaseVersionArguments(['--version=2.0.1-rc.3']), {
    help: false,
    version: '2.0.1-rc.3',
  })
  assert.deepEqual(parseReleaseVersionArguments(['--version', 'v2.0.1']), {
    help: false,
    version: '2.0.1',
  })
  assert.throws(() => parseReleaseVersionArguments([]), /--version is invalid/)
  assert.throws(() => parseReleaseVersionArguments(['--version=2.01.0']), /--version is invalid/)
  assert.throws(() => parseReleaseVersionArguments(['--version=2.0.1', '--force']), /Unknown argument/)
})

test('semantic version comparison follows release and prerelease precedence', () => {
  assert.equal(compareSemanticVersions('2.0.1-rc.3', '2.0.1-rc.2'), 1)
  assert.equal(compareSemanticVersions('2.0.1', '2.0.1-rc.3'), 1)
  assert.equal(compareSemanticVersions('2.0.1-rc.10', '2.0.1-rc.3'), 1)
  assert.equal(compareSemanticVersions('2.0.1-rc.3', '2.0.1-rc.3+build.7'), 0)
  assert.equal(compareSemanticVersions('2.0.0', '2.0.1-rc.3'), -1)
})

test('release preparation persists one version across npm and Cargo manifests and locks', () => {
  const root = mkdtempSync(join(tmpdir(), 'aster-release-version-'))
  mkdirSync(join(root, 'crate', 'src'), { recursive: true })
  writeFileSync(join(root, 'package.json'), '{\n  "name": "fixture",\n  "version": "1.0.0-rc.1"\n}\n')
  writeFileSync(join(root, 'package-lock.json'), '{\n  "name": "fixture",\n  "version": "1.0.0-rc.1",\n  "lockfileVersion": 3,\n  "packages": {\n    "": {\n      "name": "fixture",\n      "version": "1.0.0-rc.1"\n    }\n  }\n}\n')
  writeFileSync(join(root, 'Cargo.toml'), '[workspace]\nresolver = "3"\nmembers = ["crate"]\n\n[workspace.package]\nversion = "1.0.0-rc.1"\nedition = "2024"\n')
  writeFileSync(join(root, 'crate', 'Cargo.toml'), '[package]\nname = "fixture-crate"\nversion.workspace = true\nedition.workspace = true\n')
  writeFileSync(join(root, 'crate', 'src', 'lib.rs'), '')
  const cargo = process.platform === 'win32' ? 'cargo.exe' : 'cargo'
  const generated = spawnSync(cargo, ['generate-lockfile'], { cwd: root, encoding: 'utf8' })
  assert.equal(generated.status, 0, generated.stderr)

  assert.deepEqual(prepareReleaseVersion(root, '1.0.0-rc.2', cargo), {
    currentVersion: '1.0.0-rc.1',
    version: '1.0.0-rc.2',
  })
  assert.equal(JSON.parse(readFileSync(join(root, 'package.json'), 'utf8')).version, '1.0.0-rc.2')
  assert.equal(JSON.parse(readFileSync(join(root, 'package-lock.json'), 'utf8')).packages[''].version, '1.0.0-rc.2')
  assert.match(readFileSync(join(root, 'Cargo.toml'), 'utf8'), /version = "1\.0\.0-rc\.2"/)
  assert.match(readFileSync(join(root, 'Cargo.lock'), 'utf8'), /name = "fixture-crate"\r?\nversion = "1\.0\.0-rc\.2"/)
})
