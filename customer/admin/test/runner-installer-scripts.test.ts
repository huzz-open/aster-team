import { mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { expect, it } from 'vitest'

const shellInstaller = resolve(process.cwd(), 'public/install-runner.sh')
const windowsInstaller = resolve(process.cwd(), 'public/install-runner.ps1')
const windowsInitializer = resolve(process.cwd(), '../deploy/windows/init.ps1')
const windowsInstallEngine = resolve(process.cwd(), '../deploy/windows/install.ps1')
const customerCli = resolve(process.cwd(), '../backend/cli/src/main.rs')

it('Runner shell installer is syntactically valid and performs the complete install flow', () => {
  if (process.platform !== 'win32') {
    const syntax = spawnSync('bash', ['-n', shellInstaller], { encoding: 'utf8' })
    expect(syntax.status, syntax.stderr).toBe(0)
  }
  const source = readFileSync(shellInstaller, 'utf8')
  expect(source).toMatch(/api\/runner\/install-package/)
  expect(source).toMatch(/Authorization: Bearer/)
  expect(source).not.toMatch(/github\.com/)
  expect(source).toMatch(/--version\) :/)
  expect(source).toMatch(/runner install/)
  expect(source).toMatch(/runner enroll/)
  expect(source).toMatch(/aster-team-runner-/)
  expect(source).toMatch(/--runner-only/)
  expect(source).toMatch(/logs\/install-runner\.log/)
  expect(source).toMatch(/Linux:x86_64/)
  expect(source).not.toMatch(/Darwin:/)
  expect(source).toMatch(/Press Enter to use the default, or enter another existing root directory/)
  expect(source).toMatch(/''\|y\|Y\|yes/)
  expect(source).toMatch(/The selected root directory does not exist:/)
  expect(source).toMatch(/invocation_root="\$\(pwd -P\)"/)
  expect(source).toMatch(/install_root="\$\{invocation_root%\/\}\/aster-team"/)
  expect(source).toMatch(/\$\{selection%\/\}\/aster-team/)
  expect(source).toMatch(/--install-root "\$install_root"/)
})

it('Runner Windows installer parses and performs the complete install flow', () => {
  const parserShell = process.platform === 'win32' ? 'powershell.exe' : 'pwsh'
  const syntax = spawnSync(parserShell, [
    '-NoProfile', '-NonInteractive', '-Command',
    `$errors = $null; [void][Management.Automation.Language.Parser]::ParseFile('${windowsInstaller.replaceAll("'", "''")}', [ref]$null, [ref]$errors); if ($errors.Count) { $errors | ForEach-Object { Write-Error $_ }; exit 1 }`,
  ], { encoding: 'utf8' })
  expect(syntax.status, syntax.error?.message || syntax.stderr).toBe(0)
  const source = readFileSync(windowsInstaller, 'utf8')
  expect(source).toMatch(/Get-FileHash/)
  expect(source).toMatch(/api\/runner\/install-package\/windows\/amd64/)
  expect(source).toMatch(/Authorization = "Bearer \$Token"/)
  expect(source).not.toMatch(/github\.com/)
  expect(source).toMatch(/\[string\]\$Version = ''/)
  expect(source).toMatch(/\$ProgressPreference = 'SilentlyContinue'/)
  expect(source).toMatch(/& \$cli runner install/)
  expect(source).toMatch(/@\('runner', 'enroll'/)
  expect(source).toMatch(/icacls\.exe/)
  expect(source).toMatch(/function Select-RunnerInstallRoot/)
  expect(source).toMatch(/function Resolve-InvocationRoot/)
  expect(source).toMatch(/-InvocationRoot \$\(ConvertTo-PowerShellLiteral \$invocationRoot\)/)
  expect(source).toMatch(/Start-Process[^\n]+-Verb RunAs/)
  expect(source).toMatch(/-EncodedCommand/)
  expect(source).toMatch(/Press Enter to close this window/)
  expect(source).toMatch(/aster-runner-install-error-/)
  expect(source).toMatch(/\$PSVersionTable\.PSEdition/)
  expect(source).not.toMatch(/Run this command from an elevated PowerShell terminal/)
  expect(source).toMatch(/The selected root directory does not exist:/)
  expect(source).toMatch(/Join-Path \$parentRoot 'Aster Team'/)
  expect(source).toMatch(/& \$initializer --install-root \$installRoot --runner-only/)
  expect(source).toMatch(/aster-team-runner-/)
  expect(source).toMatch(/install-runner\.log/)
  expect(source).toMatch(/Test-RepairableRunnerInstall/)
  expect(source).toMatch(/Test-CompletedRunnerInstall/)
  expect(source).toMatch(/Invoke-ExistingRunnerReenrollment/)
  expect(source).toMatch(/Runner enrollment token is invalid, expired, or already used/)
  expect(source).toMatch(/-InstallerUrl \$\(ConvertTo-PowerShellLiteral \$installerUriText\)/)
  expect(source).toMatch(/Reconnecting the existing Runner installation in:/)
  expect(source).not.toMatch(/Write-Host \('Runner installation failed: ' \+ \$_\.Exception\.Message\)/)
})

it('Runner Windows directory selection returns only the selected install path', () => {
  if (process.platform !== 'win32') return
  const parentRoot = mkdtempSync(join(tmpdir(), 'aster-runner-root-'))
  try {
    const source = readFileSync(windowsInstaller, 'utf8')
    const functionStart = source.indexOf('function Test-RepairableRunnerInstall')
    const functionEnd = source.indexOf('\n$identity =', functionStart)
    expect(functionStart).toBeGreaterThanOrEqual(0)
    expect(functionEnd).toBeGreaterThan(functionStart)
    const functionSource = source.slice(functionStart, functionEnd)
    const literal = parentRoot.replaceAll("'", "''")
    const probe = `function Read-Host { param([string]$Prompt) '${literal}' }\n${functionSource}\n$result = @(Select-RunnerInstallRoot '${literal}')\nif ($result.Count -ne 1) { throw \"Expected one path, received $($result.Count): $result\" }\n$expected = Join-Path '${literal}' 'Aster Team'\nif ($result[0] -ne $expected) { throw \"Expected $expected, received $($result[0])\" }`
    const encoded = Buffer.from(probe, 'utf16le').toString('base64')
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], { encoding: 'utf8' })
    expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0)
  } finally {
    rmSync(parentRoot, { recursive: true, force: true })
  }
})

it('Runner Windows directory selection accepts Enter for the command-directory default', () => {
  if (process.platform !== 'win32') return
  const parentRoot = mkdtempSync(join(tmpdir(), 'aster-runner-default-'))
  try {
    const source = readFileSync(windowsInstaller, 'utf8')
    const functionStart = source.indexOf('function Test-RepairableRunnerInstall')
    const functionEnd = source.indexOf('\n$identity =', functionStart)
    const functionSource = source.slice(functionStart, functionEnd)
    const literal = parentRoot.replaceAll("'", "''")
    const probe = `function Read-Host { param([string]$Prompt) '' }\n${functionSource}\n$result = @(Select-RunnerInstallRoot '${literal}')\n$expected = Join-Path '${literal}' 'Aster Team'\nif ($result.Count -ne 1 -or $result[0] -ne $expected) { throw \"Expected $expected, received $result\" }`
    const encoded = Buffer.from(probe, 'utf16le').toString('base64')
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], { encoding: 'utf8' })
    expect(result.status, result.stderr || result.stdout).toBe(0)
  } finally {
    rmSync(parentRoot, { recursive: true, force: true })
  }
})

it('Runner Windows directory selection reconnects a completed installation', () => {
  if (process.platform !== 'win32') return
  const parentRoot = mkdtempSync(join(tmpdir(), 'aster-runner-installed-'))
  const installRoot = join(parentRoot, 'Aster Team')
  try {
    mkdirSync(join(installRoot, 'current'), { recursive: true })
    mkdirSync(join(installRoot, 'config', 'runner'), { recursive: true })
    writeFileSync(join(installRoot, 'config', 'runner', 'install-role'), 'runner\n')
    const source = readFileSync(windowsInstaller, 'utf8')
    const functionStart = source.indexOf('function Test-RepairableRunnerInstall')
    const functionEnd = source.indexOf('\n$identity =', functionStart)
    const functions = source.slice(functionStart, functionEnd)
    const literal = parentRoot.replaceAll("'", "''")
    const expected = installRoot.replaceAll("'", "''")
    const probe = `function Fail([string]$Message) { throw $Message }\nfunction Read-Host { param([string]$Prompt) '' }\n${functions}\n$script:ExistingRunnerInstall = $false\n$result = Select-RunnerInstallRoot '${literal}'\nif (-not $script:ExistingRunnerInstall -or $result -ne '${expected}') { throw 'Expected existing Runner reconnection' }`
    const encoded = Buffer.from(probe, 'utf16le').toString('base64')
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], { encoding: 'utf8' })
    expect(result.status, `${result.stdout}\n${result.stderr}`).toBe(0)
  } finally {
    rmSync(parentRoot, { recursive: true, force: true })
  }
})

it('Runner Windows directory selection resumes and cleans an incomplete legacy layout', () => {
  if (process.platform !== 'win32') return
  const parentRoot = mkdtempSync(join(tmpdir(), 'aster-runner-repair-'))
  const installRoot = join(parentRoot, 'Aster Team')
  try {
    mkdirSync(join(installRoot, 'config', 'control'), { recursive: true })
    mkdirSync(join(installRoot, 'data', 'database'), { recursive: true })
    writeFileSync(join(installRoot, 'install.json'), JSON.stringify({
      schema: 'aster.installation-root.v1', root: installRoot, platform: 'windows',
    }))
    const source = readFileSync(windowsInstaller, 'utf8')
    const functionStart = source.indexOf('function Test-RepairableRunnerInstall')
    const functionEnd = source.indexOf('\n$identity =', functionStart)
    const functions = source.slice(functionStart, functionEnd)
    const literal = parentRoot.replaceAll("'", "''")
    const probe = `function Read-Host { param([string]$Prompt) '${literal}' }\n${functions}\n$result = Select-RunnerInstallRoot '${literal}'\nif (-not (Test-RepairableRunnerInstall $result)) { throw 'Expected a repairable Runner directory' }\nRemove-EmptyLegacyControlDirectories $result\nif (Test-Path -LiteralPath (Join-Path $result 'config\\control')) { throw 'Legacy Control directory was not removed' }\nif (Test-Path -LiteralPath (Join-Path $result 'data\\database')) { throw 'Legacy database directory was not removed' }`
    const encoded = Buffer.from(probe, 'utf16le').toString('base64')
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', encoded], { encoding: 'utf8' })
    expect(result.status, result.stderr || result.stdout).toBe(0)
  } finally {
    rmSync(parentRoot, { recursive: true, force: true })
  }
})

it('Windows Runner bootstrap verifies and stages only Runner files', () => {
  const initializer = readFileSync(windowsInitializer, 'utf8')
  const engine = readFileSync(windowsInstallEngine, 'utf8')
  expect(initializer).toMatch(/'--runner-only' \{ \$runnerOnly = \$true \}/)
  expect(initializer).toMatch(/\$bootstrap \+= '--runner-only'/)
  expect(engine).toMatch(/\$verifyBundleArguments \+= '--runner-only'/)
  expect(engine).toMatch(/\$paths = if \(\$runnerOnly\)/)
  expect(engine).toMatch(/'bin\\aster-runner\.exe', 'bin\\aster-team-cli\.exe'/)
  expect(engine).toMatch(/Copy-Item -LiteralPath \$source -Destination \$destination -Force/)
  expect(engine).toMatch(/function Get-RunnerServicePrefix/)
  expect(engine).toMatch(/ASTER_SERVICE_PREFIX.*Get-RunnerServicePrefix/)
  expect(engine).toMatch(/Another Aster Team installation already owns Windows task/)
  expect(engine).toMatch(/Existing install directory:/)
  const cli = readFileSync(customerCli, 'utf8')
  expect(cli).toMatch(/if !arguments\.runner_only \{\s*print_next_command\(&layout\);\s*\}/)
})
