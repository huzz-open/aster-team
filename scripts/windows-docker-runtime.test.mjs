import assert from 'node:assert/strict'
import { mkdirSync, mkdtempSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import test from 'node:test'

import { resolveWindowsDockerRuntime } from './windows-docker-runtime.mjs'

function fixture({ includeHelper = true } = {}) {
  const temporary = mkdtempSync(join(tmpdir(), 'aster-docker-runtime-'))
  const programFiles = join(temporary, 'Program Files')
  const profile = join(temporary, 'user')
  const docker = resolve(programFiles, 'Docker', 'Docker', 'resources', 'bin', 'docker.exe')
  mkdirSync(dirname(docker), { recursive: true })
  mkdirSync(resolve(profile, '.docker'), { recursive: true })
  writeFileSync(docker, '')
  if (includeHelper) writeFileSync(resolve(dirname(docker), 'docker-credential-desktop.exe'), '')
  writeFileSync(resolve(profile, '.docker', 'config.json'), JSON.stringify({ credsStore: 'desktop' }))
  return { programFiles, profile, docker }
}

test('Docker discovery adds its bin directory for the configured credential helper', () => {
  const { programFiles, profile, docker } = fixture()
  const calls = []
  const execute = (command, arguments_, options = {}) => {
    calls.push({ command, arguments_, options })
    if (command === 'where.exe') return { status: 1, stdout: '' }
    return { status: 0, stdout: '' }
  }
  const runtime = resolveWindowsDockerRuntime({
    environment: { ProgramFiles: programFiles, USERPROFILE: profile, Path: 'C:\\Windows' },
    execute,
  })
  assert.equal(runtime.executable, docker)
  assert.equal(runtime.credentialHelper, resolve(dirname(docker), 'docker-credential-desktop.exe'))
  assert.equal(calls.at(-1).command, docker)
  assert.match(calls.at(-1).options.env.Path, new RegExp(`^${dirname(docker).replaceAll('\\', '\\\\')}`))
})

test('Docker discovery fails before a build when the configured helper is missing', () => {
  const { programFiles, profile } = fixture({ includeHelper: false })
  assert.throws(
    () => resolveWindowsDockerRuntime({
      environment: { ProgramFiles: programFiles, USERPROFILE: profile, Path: 'C:\\Windows' },
      execute: () => ({ status: 1, stdout: '' }),
    }),
    /docker-credential-desktop\.exe was not found/,
  )
})
