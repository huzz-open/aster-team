#!/usr/bin/env node

import { closeSync, copyFileSync, existsSync, mkdirSync, openSync, readSync, statSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const argument = name => process.argv.find(value => value.startsWith(`${name}=`))?.slice(name.length + 1) || ''
const output = resolve(root, argument('--output') || 'target/client-tools/asterctl-windows-x86_64.exe')
const cargoTargetDirectory = resolve(root, process.env.CARGO_TARGET_DIR || 'target')

if (process.platform !== 'win32' || process.arch !== 'x64') {
  throw new Error('asterctl for Windows x64 must be built on a native Windows x64 host')
}

function visualStudioBuildEnvironment() {
  const installerRoot = process.env['ProgramFiles(x86)']
  if (!installerRoot) throw new Error('ProgramFiles(x86) is unavailable')
  const vswhere = resolve(installerRoot, 'Microsoft Visual Studio', 'Installer', 'vswhere.exe')
  if (!existsSync(vswhere) || !statSync(vswhere).isFile()) {
    throw new Error('Visual Studio Build Tools discovery tool is missing')
  }
  const discovery = spawnSync(vswhere, [
    '-latest', '-products', '*', '-requires', 'Microsoft.VisualStudio.Component.VC.Tools.x86.x64',
    '-property', 'installationPath',
  ], { encoding: 'utf8', windowsHide: true })
  const installation = discovery.stdout?.trim()
  if (discovery.status !== 0 || !installation) {
    throw new Error('Microsoft Visual Studio C++ Build Tools are required to build asterctl')
  }
  const developerShell = resolve(installation, 'Common7', 'Tools', 'VsDevCmd.bat')
  const environment = spawnSync('cmd.exe', [
    '/d', '/s', '/c', `""${developerShell}" -arch=amd64 -host_arch=amd64 >nul && set"`,
  ], { encoding: 'utf8', windowsHide: true, windowsVerbatimArguments: true })
  if (environment.status !== 0) throw new Error('Visual Studio developer environment initialization failed')
  return Object.fromEntries(environment.stdout.split(/\r?\n/).flatMap(line => {
    const separator = line.indexOf('=')
    return separator > 0 ? [[line.slice(0, separator), line.slice(separator + 1)]] : []
  }))
}

function run(command, args, environment) {
  const result = spawnSync(command, args, {
    cwd: root,
    env: { ...process.env, ...environment },
    stdio: 'inherit',
    windowsHide: true,
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`${command} failed with status ${result.status}`)
}

function verifyPortableExecutable(path) {
  const stats = statSync(path)
  if (!stats.isFile() || stats.size === 0 || stats.size > 64 * 1024 * 1024) {
    throw new Error('asterctl.exe has an invalid file size')
  }
  const descriptor = openSync(path, 'r')
  try {
    const magic = Buffer.alloc(2)
    if (readSync(descriptor, magic, 0, magic.length, 0) !== magic.length || magic.toString('ascii') !== 'MZ') {
      throw new Error('asterctl.exe is not a Windows portable executable')
    }
  } finally {
    closeSync(descriptor)
  }
}

function verifyStaticCRT(binary, environment) {
  const result = spawnSync('dumpbin.exe', ['/dependents', binary], {
    env: { ...process.env, ...environment }, encoding: 'utf8', windowsHide: true,
  })
  if (result.status !== 0) throw new Error(`dumpbin failed while checking ${binary}`)
  const dependencies = `${result.stdout || ''}\n${result.stderr || ''}`
  if (/\b(?:VCRUNTIME\d*|MSVCP\d*|ucrtbase)\.dll\b/i.test(dependencies)) {
    throw new Error('asterctl.exe must use the static Microsoft C/C++ runtime')
  }
}

const environment = {
  ...visualStudioBuildEnvironment(),
  RUSTFLAGS: `${process.env.RUSTFLAGS || ''} -C target-feature=+crt-static`.trim(),
}
run('cargo.exe', ['build', '--release', '--locked', '-p', 'asterctl'], environment)
const binary = resolve(cargoTargetDirectory, 'release', 'asterctl.exe')
verifyPortableExecutable(binary)
verifyStaticCRT(binary, environment)
mkdirSync(dirname(output), { recursive: true })
copyFileSync(binary, output)
console.log(`asterctl Windows x64: ${output}`)
