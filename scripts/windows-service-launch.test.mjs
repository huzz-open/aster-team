import assert from 'node:assert/strict'
import { spawn, spawnSync } from 'node:child_process'
import { mkdtempSync, readFileSync, rmSync, mkdirSync, existsSync, statSync, writeFileSync, copyFileSync, openSync, closeSync, unlinkSync } from 'node:fs'
import { createServer } from 'node:net'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { setTimeout as delay } from 'node:timers/promises'
import { createPublicKey, verify } from 'node:crypto'
import { sourceFingerprint, createWindowsLicenseFixture } from './ci/prepare-windows-pipeline.mjs'

const root = fileURLToPath(new URL('../', import.meta.url))
const launcher = resolve(root, 'customer/deploy/windows/service-launch.ps1')
const installer = resolve(root, 'customer/deploy/windows/install.ps1')
const restorer = resolve(root, 'customer/deploy/windows/restore-backup.ps1')
const cleanup = resolve(root, 'scripts/ci/windows-smoke-cleanup.ps1')
const lifecycleScripts = ['exercise-windows-release.ps1', 'exercise-windows-runner-install.ps1', 'exercise-windows-upgrade.ps1', 'exercise-windows-coexistence.ps1']
  .map(name => resolve(root, 'scripts/ci', name))
const ps = value => `'${value.replaceAll("'", "''")}'`
const windows = process.platform === 'win32'
const powershell = windows && join(process.env.SystemRoot, 'System32/WindowsPowerShell/v1.0/powershell.exe')

test('Windows fixture resume is bound to every source byte and build input', () => {
  const read = name => Buffer.from(name)
  const original = sourceFingerprint(['a', 'b'], read, { head: 'one', rust: 'pinned' })
  assert.equal(sourceFingerprint(['b', 'a', 'a'], read, { head: 'one', rust: 'pinned' }), original)
  assert.notEqual(sourceFingerprint(['a', 'b'], () => Buffer.from('changed'), { head: 'one', rust: 'pinned' }), original)
  assert.notEqual(sourceFingerprint(['a', 'b', 'new'], read, { head: 'one', rust: 'pinned' }), original)
  assert.notEqual(sourceFingerprint(['a', 'b'], read, { head: 'two', rust: 'pinned' }), original)
})

function prepareInstallation(directory) {
  mkdirSync(join(directory, 'config/services'), { recursive: true })
  copyIfChanged(launcher, join(directory, 'config/services/service-launch.ps1'))
  writeFileSync(join(directory, 'install.json'), JSON.stringify({
    schema: 'aster.installation-root.v1', platform: 'windows', root: directory,
  }))
}

function copyIfChanged(source, destination) {
  if (!existsSync(destination) || !readFileSync(source).equals(readFileSync(destination))) {
    copyFileSync(source, destination)
  }
}

function fixedCaddyDirectory(prepare = true) {
  // Share the primary checkout's ignored test runtime across task worktrees.
  // Stable executable/script paths avoid repeated Windows execution prompts.
  const git = spawnSync('git', ['rev-parse', '--path-format=absolute', '--git-common-dir'], {
    cwd: root, encoding: 'utf8', windowsHide: true,
  })
  assert.ifError(git.error)
  assert.equal(git.status, 0, git.stderr)
  const directory = resolve(git.stdout.trim(), '..', 'target/windows-service-tests/caddy')
  if (!prepare) return directory
  const marker = join(directory, '.fixture-owner')
  if (existsSync(directory)) {
    assert.equal(readFileSync(marker, 'utf8'), 'aster.windows-caddy-test.v1', 'Refusing to overwrite an unowned directory')
  } else {
    mkdirSync(directory, { recursive: true })
    writeFileSync(marker, 'aster.windows-caddy-test.v1', { flag: 'wx' })
  }
  return directory
}

const caddyBinary = process.env.ASTER_CADDY_BIN || (windows && join(fixedCaddyDirectory(false), 'bin/caddy.exe'))

function runProbe(body, prepare = () => {}, powershellMajor = 5) {
  const directory = mkdtempSync(join(tmpdir(), 'aster service probe '))
  try {
    prepare(directory)
    const script = `
$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
if ($PSVersionTable.PSVersion.Major -ne ${powershellMajor}) { throw 'Regression must run in PowerShell ${powershellMajor}' }
foreach ($path in @(${[launcher, installer, restorer, cleanup, ...lifecycleScripts].map(ps).join(', ')})) {
    $tokens = $null
    $errors = $null
    $ast = [Management.Automation.Language.Parser]::ParseFile($path, [ref]$tokens, [ref]$errors)
    if ($errors.Count) { throw 'Product script has parse errors' }
    foreach ($function in $ast.FindAll({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] }, $false)) {
        if ($path -in @(${lifecycleScripts.map(ps).join(', ')}) -and $function.Name -notin @('Create-BrokenCandidate', 'Stop-AvailabilityProbe', 'Finish-UpgradeProbe', 'Wait-MaintenanceJob')) { continue }
        . ([scriptblock]::Create($function.Extent.Text))
    }
}
$script:AsterRoot = ${ps(directory)}
$script:AsterTaskPath = '\\Aster Team\\'
$script:AsterSmokeTaskPath = '\\Aster Team\\'
$Service = 'fixture'
$node = ${ps(process.execPath)}
${body}
`
    const result = spawnSync(powershellMajor === 5 ? powershell : 'pwsh.exe', [
      '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64'),
    ], { encoding: 'utf8', timeout: 30_000, maxBuffer: 16 * 1024 * 1024, windowsHide: true })
    assert.ifError(result.error)
    const log = join(directory, 'logs/fixture.log')
    return { ...result, log: existsSync(log) && statSync(log).isFile() && readFileSync(log, 'utf8') }
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
}

for (const [primary, probe, logged] of [[true, true, true], [true, false, false], [true, false, true], [false, true, true], [false, false, true], [false, false, false]]) {
  test(`Windows upgrade preserves primary errors and strict probe failures (${primary}/${probe}/${logged})`, { skip: !windows }, () => {
    const result = runProbe(`
function Stop-AvailabilityProbe { ${probe ? "throw 'secondary probe timeout'" : "'Probe completed' | Out-Null"} }
$primary = $null
${primary ? "try { throw 'primary maintenance failure' } catch { $primary = $_ }" : ''}
$log = Join-Path $script:AsterRoot 'probe.log'
${logged ? "[IO.File]::WriteAllText($log, '2026-09-06T05:50:00Z http://127.0.0.1:21080/healthz timeout')" : ''}
$caught = $null
try { Finish-UpgradeProbe $null (Join-Path $script:AsterRoot 'stop') $log $primary }
catch { $caught = $_ }
${primary
    ? "if (-not $caught -or $caught.Exception.Message -ne 'primary maintenance failure') { throw 'Primary exception was masked' }"
    : probe || logged
      ? "if (-not $caught -or $caught.Exception.Message -notmatch 'Availability probe failed:' -or $caught.Exception.Message -notmatch '21080/healthz') { throw 'Probe failure or request detail was lost' }"
      : "if ($caught) { throw $caught }"}
Write-Output 'Error priority verified'
`, () => {}, 7)
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /Error priority verified/)
    if (primary && probe) assert.match(`${result.stdout}\n${result.stderr}`, /secondary probe timeout/)
  })
}

test('Maintenance job polling survives a transport exception without a Response property', { skip: !windows }, () => {
  const result = runProbe(`
$script:AdminBase = 'http://127.0.0.1:1'
$script:readCount = 0
function Invoke-Json {
    $script:readCount++
    if ($script:readCount -eq 1) { throw [IO.IOException]::new('connection reset') }
    return @{ jobs = @(@{ id = 'job'; status = 'succeeded' }) }
}
$session = [Microsoft.PowerShell.Commands.WebRequestSession]::new()
$job = Wait-MaintenanceJob 'job' 'succeeded' $session 5
if ($job.status -ne 'succeeded' -or $script:readCount -ne 2) { throw 'Polling did not recover' }
Write-Output 'Maintenance polling recovered'
`, () => {}, 7)
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
})

for (const probeFails of [false, true]) {
  test(`Maintenance outages do not hide probe infrastructure errors (${probeFails})`, { skip: !windows }, () => {
    const result = runProbe(`
function Stop-AvailabilityProbe { ${probeFails ? "throw 'probe worker failed'" : ''} }
$log = Join-Path $script:AsterRoot 'probe.log'
[IO.File]::WriteAllText($log, 'maintenance request outage')
$caught = $null
try { Finish-UpgradeProbe $null (Join-Path $script:AsterRoot 'stop') $log $null -AllowMaintenanceOutage }
catch { $caught = $_ }
${probeFails ? "if (-not $caught -or $caught.Exception.Message -notmatch 'probe worker failed') { throw 'Infrastructure error was hidden' }" : "if ($caught) { throw $caught }"}
Write-Output 'Maintenance probe policy verified'
`, () => {}, 7)
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /Maintenance probe policy verified/)
  })
}

for (const primary of [false, true]) {
  test(`Windows probe diagnostic IO failures remain visible without masking the primary error (${primary})`, { skip: !windows }, () => {
    const result = runProbe(`
function Stop-AvailabilityProbe { }
$primary = $null
${primary ? "try { throw 'primary maintenance failure' } catch { $primary = $_ }" : ''}
$log = Join-Path $script:AsterRoot 'probe.log'
[IO.File]::WriteAllText($log, 'request failed')
$locked = [IO.File]::Open($log, 'Open', 'ReadWrite', 'None')
$caught = $null
try { Finish-UpgradeProbe $null (Join-Path $script:AsterRoot 'stop') $log $primary }
catch { $caught = $_ }
finally { $locked.Dispose() }
if (-not $caught) { throw 'An unreadable diagnostic was incorrectly accepted' }
${primary
    ? "if ($caught.Exception.Message -ne 'primary maintenance failure') { throw 'Diagnostic IO replaced the primary failure' }"
    : "if ($caught.Exception.Message -notmatch 'probe diagnostic could not be read') { throw 'Diagnostic IO failure was lost' }"}
Write-Output 'Diagnostic failure priority verified'
`, () => {}, 7)
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /Diagnostic failure priority verified/)
    if (primary) assert.match(result.stderr, /probe diagnostic could not be read/)
  })
}

for (const signerExit of [0, 23]) {
  test(`Windows failure fixture keeps signer stdout out of its archive return value (exit ${signerExit})`, { skip: !windows }, () => {
    const result = runProbe(`
$ReleaseTool = Join-Path $script:AsterRoot 'signer.cmd'
$ReleaseSigningKey = Join-Path $script:AsterRoot 'test.seed'
$ReleaseSigningKeyId = 'fixture-only'
$rejected = $false
$actual = $null
try { $actual = Create-BrokenCandidate (Join-Path $script:AsterRoot 'source.tar.gz') '2.0.3-ci-fail' (Join-Path $script:AsterRoot 'broken') }
catch { $rejected = $_.Exception.Message -eq 'broken candidate signing failed'; if (-not $rejected) { throw } }
if (${signerExit} -eq 0) {
    if ($rejected -or $actual -isnot [string] -or -not (Test-Path -LiteralPath $actual -PathType Leaf)) { throw 'Signer stdout contaminated the archive return value' }
    Write-Output 'Single archive path returned'
} else {
    if (-not $rejected -or $null -ne $actual) { throw 'Signing failure was not preserved' }
    Write-Output 'Signing failure preserved'
}
`, directory => {
      const source = join(directory, 'source')
      mkdirSync(join(source, 'release/bin'), { recursive: true })
      writeFileSync(join(source, 'release/VERSION'), '2.0.2-ci-upgrade\n')
      writeFileSync(join(source, 'release/RELEASE.json'), '{}')
      writeFileSync(join(source, 'release/bin/aster-control.exe'), 'unit test fixture only')
      writeFileSync(join(directory, 'signer.cmd'), `@echo off\r\necho fixture RELEASE.json\r\nexit /b ${signerExit}\r\n`)
      const archive = spawnSync('tar.exe', ['-czf', join(directory, 'source.tar.gz'), '-C', source, 'release'], { encoding: 'utf8', windowsHide: true })
      assert.ifError(archive.error)
      assert.equal(archive.status, 0, archive.stderr)
    })
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, signerExit === 0 ? /Single archive path returned/ : /Signing failure preserved/)
  })
}

test('Windows installer confines Caddy commands to the instance and restores the caller environment', { skip: !windows }, () => {
  const code = `const assert = require('node:assert/strict'); const path = require('node:path'); const root = process.argv[1];
    assert.equal(process.env.HOME, path.join(root, 'data/caddy'));
    assert.equal(process.env.XDG_DATA_HOME, path.join(root, 'data/caddy'));
    assert.equal(process.env.XDG_CONFIG_HOME, path.join(root, 'config/caddy'));`
  const result = runProbe(`
[Environment]::SetEnvironmentVariable('HOME', 'keep-existing-home', 'Process')
[Environment]::SetEnvironmentVariable('XDG_DATA_HOME', $null, 'Process')
[Environment]::SetEnvironmentVariable('XDG_CONFIG_HOME', 'keep-existing-config', 'Process')
Invoke-AsterCaddy $node @('-e', ${ps(code)}, $script:AsterRoot) 'Caddy environment probe' $script:AsterRoot
if ($env:HOME -ne 'keep-existing-home' -or $env:XDG_CONFIG_HOME -ne 'keep-existing-config' -or
    [Environment]::GetEnvironmentVariable('XDG_DATA_HOME', 'Process')) { throw 'Caller environment changed' }
Write-Output 'Caddy environment restored'
`)
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
  assert.match(result.stdout, /Caddy environment restored/)
  const source = readFileSync(installer, 'utf8')
  assert.doesNotMatch(source, /Invoke-Checked \(Join-Path \$releaseDirectory 'bin\\caddy\.exe'\)/)
  assert.match(source, /Invoke-AsterCaddy .*@\('validate'.*\$installRoot/)
})

test('Windows installer restores Caddy environment even when the native command fails', { skip: !windows }, () => {
  const result = runProbe(`
[Environment]::SetEnvironmentVariable('HOME', 'keep-after-failure', 'Process')
[Environment]::SetEnvironmentVariable('XDG_DATA_HOME', 'keep-data', 'Process')
[Environment]::SetEnvironmentVariable('XDG_CONFIG_HOME', $null, 'Process')
$rejected = $false
try { Invoke-AsterCaddy $node @('-e', 'process.exit(23)') 'Caddy failure probe' $script:AsterRoot }
catch { $rejected = $_.Exception.Message -eq 'Caddy failure probe exited with status 23' }
if (-not $rejected) { throw 'Native Caddy failure was not reported' }
if ($env:HOME -ne 'keep-after-failure' -or $env:XDG_DATA_HOME -ne 'keep-data' -or
    [Environment]::GetEnvironmentVariable('XDG_CONFIG_HOME', 'Process')) { throw 'Caller environment changed after failure' }
Write-Output 'Caddy failure preserved'
`)
  assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
  assert.match(result.stdout, /Caddy failure preserved/)
})

test('Windows launcher separates native output from PowerShell errors and avoids environment dumps', () => {
  const source = readFileSync(launcher, 'utf8')
  assert.doesNotMatch(source, /2>&1|--environ/)
  assert.match(source, /RedirectStandardOutput = \$true/)
  assert.match(source, /RedirectStandardError = \$true/)
  assert.match(source, /ReadLineAsync/)
  assert.match(source, /\$process\.ExitCode -ne 0/)
  assert.match(source, /\$Service-launcher\.log/)
  assert.match(source, /exit 1/)
  const install = readFileSync(installer, 'utf8')
  assert.match(install, /Wait-AsterInstallationReady \(Join-Path \$installRoot 'bin\\aster-team-cli\.exe'\)/)
  assert.match(install, /& \$Cli status/)
  for (const name of ['release', 'runner-install']) {
    const smoke = readFileSync(resolve(root, `scripts/ci/exercise-windows-${name}.ps1`), 'utf8')
    assert.match(smoke, /Get-ScheduledTaskInfo/)
    assert.match(smoke, /task-results\.txt/)
    assert.match(smoke, /Copy-Item -LiteralPath \(Join-Path \$installRoot 'logs'\)/)
    assert.match(smoke, /windows-smoke-cleanup\.ps1/)
    assert.match(smoke, /Complete-AsterSmokeCleanup/)
  }
})

test('PowerShell 5.1 preserves stderr INFO logs, stdout, Unicode and final unterminated lines', { skip: !windows }, () => {
  const code = 'process.stdout.write("stdout-ok\\n"); process.stderr.write("INFO stderr 正常\\n"); process.stderr.write("last-line")'
  const result = runProbe(`Invoke-AsterProgram -Program $node -Arguments @('-e', ${ps(code)})`)
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.log, /stdout-ok/)
  assert.match(result.log, /INFO stderr 正常/)
  assert.match(result.log, /last-line/)
})

test('PowerShell 5.1 preserves Windows argv boundaries without a shell', { skip: !windows }, () => {
  const args = ['', 'path with spaces', 'C:\\trailing slash\\', 'a"b', 'a\\"b', '正常', 'x&y|z', '$literal']
  const code = 'console.log("ARGV=" + JSON.stringify(process.argv.slice(1)))'
  const result = runProbe(`Invoke-AsterProgram -Program $node -Arguments @('-e', ${ps(code)}, '--', ${args.map(ps).join(', ')})`)
  assert.equal(result.status, 0, result.stderr)
  const actual = JSON.parse(result.log.match(/ARGV=(.*)/)[1])
  assert.deepEqual(actual, args)
})

test('PowerShell 5.1 drains both full pipes without deadlock and retains the tails', { skip: !windows }, () => {
  const code = 'for(let i=0;i<256;i++){process.stdout.write("O".repeat(4096)+"\\n");process.stderr.write("E".repeat(4096)+"\\n")}process.stdout.write("OUT-TAIL\\n");process.stderr.write("ERR-TAIL\\n")'
  const result = runProbe(`Invoke-AsterProgram -Program $node -Arguments @('-e', ${ps(code)}) | Out-Null`)
  assert.equal(result.status, 0, result.stderr)
  assert.match(result.log, /OUT-TAIL/)
  assert.match(result.log, /ERR-TAIL/)
})

test('PowerShell 5.1 fails on a real nonzero exit after retaining stderr', { skip: !windows }, () => {
  const code = 'process.stderr.write("real-failure\\n");process.exitCode=17'
  const result = runProbe(`Invoke-AsterProgram -Program $node -Arguments @('-e', ${ps(code)})`)
  assert.notEqual(result.status, 0)
  assert.match(result.stderr, /Service process exited with status 17/)
  assert.match(result.log, /real-failure/)
})

test('PowerShell 5.1 does not suppress executable or log IO failures', { skip: !windows }, () => {
  const missing = runProbe(`Invoke-AsterProgram -Program (Join-Path $AsterRoot 'missing.exe') -Arguments @('run')`)
  assert.notEqual(missing.status, 0)
  assert.match(missing.stderr, /Service executable is missing/)
  const result = runProbe(`Invoke-AsterProgram -Program $node -Arguments @('-e', 'console.log("must-log")')`, directory => {
    mkdirSync(join(directory, 'logs'))
    mkdirSync(join(directory, 'logs/fixture.log'))
  })
  assert.notEqual(result.status, 0)
})

test('installation readiness requires successful status and waits for startup, not just Control', { skip: !windows }, () => {
  const healthy = runProbe(`
$script:probes = 0
function Test-Status { param($Command) if ($Command -ne 'status') { throw 'Expected status' }; $script:probes++; $global:LASTEXITCODE = [int]($script:probes -lt 2) }
Wait-AsterInstallationReady 'Test-Status' 5
if ($script:probes -ne 2) { throw 'Readiness did not retry' }
`)
  assert.equal(healthy.status, 0, healthy.stderr)
  const failed = runProbe(`
function Test-Status { param($Command) $global:LASTEXITCODE = 1 }
Wait-AsterInstallationReady 'Test-Status' 0
`)
  assert.notEqual(failed.status, 0)
  assert.match(failed.stderr, /Installation readiness timed out/)
})

test('full launcher exits nonzero and persists its own startup exception', { skip: !windows }, () => {
  const directory = mkdtempSync(join(tmpdir(), 'aster launcher failure '))
  try {
    prepareInstallation(directory)
    const result = spawnSync(powershell, [
      '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', join(directory, 'config/services/service-launch.ps1'), '-Service', 'caddy',
    ], { encoding: 'utf8', timeout: 10_000, windowsHide: true })
    assert.ifError(result.error)
    assert.equal(result.status, 1)
    const log = readFileSync(join(directory, 'logs/caddy-launcher.log'), 'utf8')
    assert.match(log, /caddy launcher failed/)
    assert.match(log, /Service executable is missing/)
  } finally {
    rmSync(directory, { recursive: true, force: true })
  }
})

test('restore preflights destinations and rolls back a partially completed move', { skip: !windows }, () => {
  const result = runProbe(`
$source = Join-Path $AsterRoot 'source'
$destination = Join-Path $AsterRoot 'destination'
New-Item -ItemType Directory -Path $source, $destination | Out-Null
$managedEntries = @('one', 'two')
foreach ($name in $managedEntries) { [IO.File]::WriteAllText((Join-Path $source $name), $name) }
[IO.File]::WriteAllText((Join-Path $destination 'two'), 'existing')
try { Move-ManagedEntries $source $destination; throw 'Expected collision rejection' }
catch { if ($_.Exception.Message -notlike '*destination is not empty*') { throw } }
if (-not (Test-Path -LiteralPath (Join-Path $source 'one'))) { throw 'Moved before preflight completed' }
Remove-Item -LiteralPath (Join-Path $destination 'two')
$script:realMove = (Get-Item 'function:Move-AsterEntry').ScriptBlock
$script:moves = 0
function Move-AsterEntry($Source, $Destination) {
    $script:moves++
    if ($script:moves -eq 2) { throw 'injected move failure' }
    & $script:realMove $Source $Destination
}
try { Move-ManagedEntries $source $destination; throw 'Expected injected failure' }
catch { if ($_.Exception.Message -ne 'injected move failure') { throw } }
foreach ($name in $managedEntries) {
    if ([IO.File]::ReadAllText((Join-Path $source $name)) -ne $name) { throw 'Rollback lost content' }
}
if (@(Get-ChildItem -LiteralPath $destination).Count -ne 0) { throw 'Rollback left partial moves' }
`)
  assert.equal(result.status, 0, result.stderr)
})

test('restored junctions target the final release and cleanup never follows rollback junctions', { skip: !windows }, () => {
  const result = runProbe(`
$release = Join-Path $AsterRoot 'releases\\2.0.1'
New-Item -ItemType Directory -Path $release | Out-Null
[IO.File]::WriteAllText((Join-Path $release 'RELEASE.json'), 'keep-live-release')
$references = @([pscustomobject]@{ path = 'current'; version = '2.0.1' })
Restore-AsterReferences $AsterRoot $references
if ((Get-Item -LiteralPath (Join-Path $AsterRoot 'current')).LinkType -ne 'Junction') { throw 'Missing current junction' }
$rollback = Join-Path $AsterRoot 'staging\\rollback'
New-Item -ItemType Directory -Path $rollback | Out-Null
New-Item -ItemType Junction -Path (Join-Path $rollback 'current') -Target $release | Out-Null
Remove-AsterRestoreTree $rollback
if ([IO.File]::ReadAllText((Join-Path $AsterRoot 'current\\RELEASE.json')) -ne 'keep-live-release') { throw 'Cleanup followed a junction into live data' }
# Remove the remaining fixture junction explicitly before Node removes its temp root.
[IO.Directory]::Delete((Join-Path $AsterRoot 'current'))
`)
  assert.equal(result.status, 0, result.stderr)
})

test('Runner-only restore keeps an unenrolled Runner disabled without Control metadata or tasks', { skip: !windows }, () => {
  const result = runProbe(`
$runnerOnly = $true
$tasks = [ordered]@{ '\\Aster Team\\Runner' = 'runner' }
$script:calls = [Collections.Generic.List[string]]::new()
function Ensure-AsterTaskFolder {}
function Set-AsterTaskRuntimePolicy($FullName) {}
function New-ScheduledTaskAction($Execute, $Argument) { return $Argument }
function New-ScheduledTaskTrigger([switch]$AtStartup) { return 'startup' }
function New-ScheduledTaskPrincipal($UserId, $LogonType, $RunLevel) { return $UserId }
function Register-ScheduledTask($TaskName, $TaskPath, $Action, $Trigger, $Principal, [switch]$Force) { $script:calls.Add(('register|' + $TaskName)) }
function Disable-ScheduledTask($TaskName, $TaskPath) { $script:calls.Add(('disable|' + $TaskName)) }
function Invoke-Checked($Program, $Arguments, $Label) { $script:calls.Add(($Arguments -join '|')) }
New-Item -ItemType Directory -Path (Join-Path $AsterRoot 'config\\services') | Out-Null
[IO.File]::WriteAllText((Join-Path $AsterRoot 'config\\services\\service-launch.ps1'), 'fixture')
Start-RestoredServices $AsterRoot
if ($script:calls.Count -ne 2 -or $script:calls[1] -ne 'disable|Runner') { throw 'Unenrolled Runner was enabled or started' }
if (($script:calls -join ';') -match 'Control|Caddy|Maintenance') { throw 'Runner restore registered Control services' }
`)
  assert.equal(result.status, 0, result.stderr)
})

for (const runnerOnly of [false, true]) {
  test(`restored services preserve native status argv and retry (${runnerOnly ? 'Runner' : 'Control'})`, { skip: !windows }, () => {
    const result = runProbe(`
$runnerOnly = $${runnerOnly}
function Register-AsterTasks($Root) { return '\\Aster Team\\Control Blue' }
function Invoke-Checked($Program, $Arguments, $Label) {}
$script:statusCalls = 0
$script:received = ''
# Keep the real restore function, replacing only its native CLI with an argv
# recorder. A PowerShell function mock would hide native string splatting.
$invocation = '& $cli @statusArguments | Out-Host'
$source = (Get-Item 'function:Start-RestoredServices').ScriptBlock.ToString()
if (-not $source.Contains($invocation)) { throw 'Restore native invocation changed; update the recorder' }
$recorder = @'
$script:received = & $node -e 'console.log(JSON.stringify(process.argv.slice(1)))' -- @statusArguments
if ($LASTEXITCODE -ne 0) { throw 'Native argv recorder failed' }
$script:statusCalls++
$global:LASTEXITCODE = [int]($script:statusCalls -lt 2)
'@
& ([scriptblock]::Create($source.Replace($invocation, $recorder))) $AsterRoot
if ($script:statusCalls -ne 2) { throw 'Restore readiness did not retry' }
Write-Output ('RESTORE_ARGV=' + $script:received)
`, directory => {
      mkdirSync(join(directory, 'config/runner'), { recursive: true })
      writeFileSync(join(directory, 'config/runner/identity.json'), '{}')
    })
    assert.equal(result.status, 0, result.stderr)
    assert.deepEqual(JSON.parse(result.stdout.match(/RESTORE_ARGV=(.*)/)[1]), runnerOnly ? ['runner', 'status'] : ['status'])
  })
}

test('checked native logging does not contaminate a function return value', { skip: !windows }, () => {
  const result = runProbe(`
function Result-Probe {
    Invoke-Checked $node @('-e', 'console.log(12345)') 'fixture'
    return 'control-task'
}
$actual = @(Result-Probe)
if ($actual.Count -ne 1 -or $actual[0] -ne 'control-task') { throw 'Native stdout contaminated task name' }
`)
  assert.equal(result.status, 0, result.stderr)
})

test('both Windows lifecycle entry points preflight their namespace before creating workspaces', () => {
  for (const path of lifecycleScripts.slice(0, 2)) {
    const source = readFileSync(path, 'utf8')
    assert.ok(source.indexOf('Assert-AsterSmokeHost') > 0)
    assert.ok(source.indexOf('Assert-AsterSmokeHost') < source.indexOf('New-Item'))
  }
})

test('PowerShell task identity uses persisted prefixes, not inherited environment', { skip: !windows }, () => {
  const result = runProbe(`
$env:ASTER_SERVICE_PREFIX = 'wrong-terminal'
if ((Get-AsterInstanceTaskPath ([pscustomobject]@{})) -cne '\\Aster Team\\') { throw 'Legacy task path changed' }
$instance = [pscustomobject]@{ schema = 'aster.windows-instance.v1'; service_prefix = 'lab-a' }
$marker = [pscustomobject]@{ windows_instance = $instance }
if ((Get-AsterInstanceTaskPath $marker) -cne '\\Aster Team\\lab-a\\') { throw 'Persisted namespace ignored' }
foreach ($prefix in @('..', 'lab/a', 'UPPER', '1lab', ('x' * 33))) {
    $instance.service_prefix = $prefix
    try { Get-AsterInstanceTaskPath $marker; throw 'Expected invalid prefix' }
    catch { if ($_.Exception.Message -notlike '*Invalid Windows service prefix*') { throw } }
}
Write-Output 'All invalid prefixes rejected and persisted identity retained'
`)
  assert.equal(result.status, 0, result.stderr)
})

for (const major of [5, 7]) {
  test(`PowerShell ${major} removes smoke overrides from native children and restores unset versus empty values`, { skip: !windows }, () => {
    const portNames = ['ASTER_API_PORT', 'ASTER_MEMBER_PORT', 'ASTER_ADMIN_PORT', 'ASTER_BLUE_API_PORT',
      'ASTER_BLUE_MEMBER_PORT', 'ASTER_BLUE_ADMIN_PORT', 'ASTER_GREEN_API_PORT', 'ASTER_GREEN_MEMBER_PORT',
      'ASTER_GREEN_ADMIN_PORT', 'ASTER_CADDY_ADMIN_PORT', 'ASTER_DOMAIN_HTTP_PORT', 'ASTER_DOMAIN_HTTPS_PORT']
    const childCheck = `const assert = require('node:assert/strict');
      for (const name of ${JSON.stringify(portNames)}) assert.equal(Object.hasOwn(process.env, name), false, name + ' must be absent');
      assert.equal(process.env.ASTER_SERVICE_PREFIX, 'lab-a'); assert.equal(process.env.ASTER_PORT_OFFSET, '10000');`
    const result = runProbe(`
[Environment]::SetEnvironmentVariable('ASTER_API_PORT', '9999', 'Process')
[Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', 'original', 'Process')
[Environment]::SetEnvironmentVariable('ASTER_PORT_OFFSET', [NullString]::Value, 'Process')
[Environment]::SetEnvironmentVariable('ASTER_ADMIN_PORT', '', 'Process')
$adminWasPresent = [Environment]::GetEnvironmentVariables('Process').Contains('ASTER_ADMIN_PORT')
$previous = Set-AsterSmokeEnvironment 'lab-a' 10000
$nested = Set-AsterSmokeEnvironment 'lab-b' 20000
Restore-AsterSmokeEnvironment $nested
& $node (Join-Path $script:AsterRoot 'env-probe.cjs')
if ($LASTEXITCODE -ne 0) { throw 'Native child inherited empty port overrides' }
Restore-AsterSmokeEnvironment $previous
$restored = [Environment]::GetEnvironmentVariables('Process')
if ($restored['ASTER_API_PORT'] -ne '9999' -or $restored['ASTER_SERVICE_PREFIX'] -ne 'original' -or
    $restored.Contains('ASTER_PORT_OFFSET') -or $restored.Contains('ASTER_ADMIN_PORT') -ne $adminWasPresent) {
    throw 'Environment restoration changed value presence'
}
if ($adminWasPresent -and $restored['ASTER_ADMIN_PORT'] -cne '') { throw 'Original empty string was not preserved' }
Write-Output 'Native environment isolation passed'
`, directory => writeFileSync(join(directory, 'env-probe.cjs'), childCheck), major)
    assert.equal(result.status, 0, `${result.stdout}\n${result.stderr}`)
    assert.match(result.stdout, /Native environment isolation passed/)
  })
}

test('custom smoke namespaces coexist and environment overrides are restored', { skip: !windows }, () => {
  const result = runProbe(`
function Get-ScheduledTask { return [pscustomobject]@{ TaskPath = '\\Aster Team\\existing\\' } }
function Get-NetTCPConnection { return [pscustomobject]@{ State = 'Listen'; LocalPort = 11080 } }
function Test-Path { return $true }
Assert-AsterSmokeIsolation 'C:\\existing' 'lab-b' 20000
if ($script:AsterSmokeTaskPath -cne '\\Aster Team\\lab-b\\') { throw 'Wrong namespace' }
try { Assert-AsterSmokeIsolation 'C:\\existing' 'existing' 20000; throw 'Expected collision' }
catch { if ($_.Exception.Message -notlike '*unused Aster Team task namespace*') { throw } }
$env:ASTER_API_PORT = '9999'
$env:ASTER_SERVICE_PREFIX = 'original'
$previous = Set-AsterSmokeEnvironment 'lab-b' 20000
if ($env:ASTER_API_PORT -or $env:ASTER_SERVICE_PREFIX -ne 'lab-b' -or $env:ASTER_PORT_OFFSET -ne '20000') { throw 'Smoke inherited unrelated overrides' }
Restore-AsterSmokeEnvironment $previous
if ($env:ASTER_API_PORT -ne '9999' -or $env:ASTER_SERVICE_PREFIX -ne 'original') { throw 'Smoke leaked environment changes' }
`)
  assert.equal(result.status, 0, result.stderr)
})

test('Windows smoke isolation fails closed on existing tasks, installs, ports and inventory errors', { skip: !windows }, () => {
  const result = runProbe(`
$script:scenario = 'clean'
function Get-ScheduledTask {
    if ($script:scenario -eq 'inventory-error') { throw 'inventory unavailable' }
    if ($script:scenario -eq 'tasks') { return [pscustomobject]@{ TaskPath = '\\Aster Team\\' } }
}
function Test-Path { return ($script:scenario -eq 'install') }
function Get-NetTCPConnection {
    if ($script:scenario -eq 'ports') { return [pscustomobject]@{ State = 'Listen'; LocalPort = 11080 } }
    if ($script:scenario -eq 'network-error') { throw 'network inventory unavailable' }
}
Assert-AsterSmokeIsolation 'C:\\unused-fixture-root'
foreach ($scenario in @('tasks', 'install', 'ports', 'inventory-error', 'network-error')) {
    $script:scenario = $scenario
    $rejected = $false
    try { Assert-AsterSmokeIsolation 'C:\\unused-fixture-root' } catch { $rejected = $true }
    if (-not $rejected) { throw "Unsafe host was accepted: $scenario" }
}
`)
  assert.equal(result.status, 0, result.stderr)
})

test('smoke cleanup refuses a same-name scheduled task owned by another installation', { skip: !windows }, () => {
  const result = runProbe(`
$installRoot = $AsterRoot
function Get-ScheduledTask { return [pscustomobject]@{
    TaskPath = '\\Aster Team\\'; TaskName = 'Control Blue'
    Actions = @([pscustomobject]@{ Arguments = '-File "C:\\existing-install\\service-launch.ps1" -Service control-blue' })
} }
function Test-Path { return $false }
function Stop-ScheduledTask { throw 'must-not-stop' }
function Unregister-ScheduledTask { throw 'must-not-delete' }
try { Remove-AsterTasks; throw 'Expected ownership rejection' }
catch { if ($_.Exception.Message -notlike '*outside this smoke installation*') { throw } }
Write-Output 'Unrelated scheduled task preserved'
`)
  assert.equal(result.status, 0, result.stdout + result.stderr)
})

test('incomplete rollback stays stopped and restart failures preserve the original error', { skip: !windows }, () => {
  const result = runProbe(`
$installRoot = $AsterRoot
$rollbackRoot = Join-Path $AsterRoot 'rollback'
New-Item -ItemType Directory -Path $rollbackRoot | Out-Null
[IO.File]::WriteAllText((Join-Path $rollbackRoot 'data'), 'must-preserve')
$script:restarts = 0
function Start-RestoredServices($Root) { $script:restarts++; throw 'secondary restart failure' }
try { Restart-AsterAfterFailure 'original failure' }
catch { if ($_.Exception.Message -ne 'original failure') { throw } }
if ($script:restarts -ne 0) { throw 'Restarted partially restored data' }
Remove-Item -LiteralPath (Join-Path $rollbackRoot 'data')
try { Restart-AsterAfterFailure 'original failure' }
catch { if ($_.Exception.Message -ne 'original failure') { throw } }
if ($script:restarts -ne 1) { throw 'Did not attempt a safe restart' }
Write-Output 'Original failure preserved after both recovery paths'
`)
  assert.equal(result.status, 0, result.stderr)
})

async function freePort() {
  const server = createServer()
  await new Promise((resolve_, reject) => { server.once('error', reject); server.listen(0, '127.0.0.1', resolve_) })
  const { port } = server.address()
  await new Promise(resolve_ => server.close(resolve_))
  return port
}

test('real Caddy releases ports and its executable after normal exit, wrapper termination and explicit stop', {
  skip: !windows || !caddyBinary || !existsSync(caddyBinary),
}, async () => {
  const directory = fixedCaddyDirectory()
  const lock = join(directory, '.run.lock')
  const lockDescriptor = openSync(lock, 'wx')
  let child
  let completed
  let adminPort
  let stderr = ''
  try {
    const httpPort = await freePort()
    do { adminPort = await freePort() } while (adminPort === httpPort)
    prepareInstallation(directory)
    mkdirSync(join(directory, 'bin'), { recursive: true })
    mkdirSync(join(directory, 'config/caddy'), { recursive: true })
    copyIfChanged(caddyBinary, join(directory, 'bin/caddy.exe'))
    writeFileSync(join(directory, 'config/caddy/Caddyfile'), `{
  admin 127.0.0.1:${adminPort}
  persist_config off
  auto_https off
}
http://127.0.0.1:${httpPort} {
  respond "aster-caddy-regression-ok"
}
`)
    for (const stopMode of ['normal', 'wrapper-only', 'service-stop']) {
    child = spawn(powershell, [
      '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', join(directory, 'config/services/service-launch.ps1'), '-Service', 'caddy',
    ], { windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'] })
    child.stdout.resume()
    child.stderr.on('data', chunk => { stderr += chunk })
    completed = new Promise((resolve_, reject) => { child.once('error', reject); child.once('close', resolve_) })
    let healthy = false
    for (let attempt = 0; attempt < 100; attempt++) {
      assert.equal(child.exitCode, null, stderr)
      try {
        const response = await fetch(`http://127.0.0.1:${httpPort}`, { signal: AbortSignal.timeout(300) })
        healthy = response.status === 200 && await response.text() === 'aster-caddy-regression-ok'
      } catch { /* The listener may not exist until startup completes. */ }
      if (healthy) break
      await delay(100)
    }
    assert.equal(healthy, true, `Caddy failed readiness: ${stderr}`)
    // The log must be available while the service is alive, not only on exit.
    await delay(100)
    const log = readFileSync(join(directory, 'logs/caddy.log'), 'utf8')
    assert.match(log, /"level":"info"/)
    assert.match(log, /using config from file/)
    assert.doesNotMatch(log, /caddy\.HomeDir=|USERPROFILE=/)
    const stop = stopMode === 'normal' ? spawnSync(join(directory, 'bin/caddy.exe'), ['stop', '--address', `127.0.0.1:${adminPort}`], {
      encoding: 'utf8', timeout: 5000, windowsHide: true,
    }) : stopMode === 'wrapper-only' ? spawnSync('taskkill.exe', ['/PID', String(child.pid), '/F'], {
      encoding: 'utf8', timeout: 5000, windowsHide: true,
    }) : spawnSync(powershell, [
      '-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass',
      '-File', join(directory, 'config/services/service-launch.ps1'), '-Service', 'caddy', '-Stop',
    ], {
      encoding: 'utf8', timeout: 45_000, windowsHide: true,
    })
    assert.ifError(stop.error)
    assert.equal(stop.status, 0, stop.stderr)
    const code = await Promise.race([completed, delay(5000).then(() => 'timeout')])
    assert.notEqual(code, 'timeout', stderr)
    if (stopMode === 'normal') assert.equal(code, 0, stderr)
    // No /T fallback before these assertions: killing only the wrapper must
    // close the listener and release the actual executable's delete handle.
    const exclusive = spawnSync(powershell, ['-NoProfile', '-NonInteractive', '-Command', `
$listener = [Net.Sockets.TcpListener]::new([Net.IPAddress]::Loopback, ${httpPort})
$listener.Start(); $listener.Stop()
$deadline = [DateTime]::UtcNow.AddSeconds(5)
while ($true) {
    try {
        $file = [IO.File]::Open(${ps(join(directory, 'bin/caddy.exe'))}, 'Open', 'ReadWrite', 'None')
        $file.Dispose(); break
    } catch {
        if ([DateTime]::UtcNow -ge $deadline) { throw }
        Start-Sleep -Milliseconds 100
    }
}
`], { encoding: 'utf8', timeout: 10_000, windowsHide: true })
    assert.equal(exclusive.status, 0, `${stopMode}: ${exclusive.stderr}`)
    }
  } finally {
    if (child && child.exitCode === null) {
      // Stop only this test's process tree; never select global Caddy services.
      spawnSync('taskkill.exe', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true })
      await completed
    }
    closeSync(lockDescriptor)
    unlinkSync(lock)
    // Keep the executable, launcher and diagnostics at stable paths for reuse.
  }
})


test('Windows fixtures use fresh scoped free and paid keys and include the signed free allowance', () => {
  const fixture = createWindowsLicenseFixture('2.0.1-test.1', '2026-09-08T00:00:00.000Z')
  const profiles = JSON.parse(fixture.licenseKeys)
  const document = JSON.parse(fixture.freeLicenseBytes)
  assert.deepEqual(profiles[0].policy.sources, ['free_distribution'])
  assert.deepEqual(profiles[1].policy.sources, ['commercial_order'])
  assert.notEqual(profiles[0].public_key_spki, profiles[1].public_key_spki)
  assert.equal(document.claims.binding.mode, 'unbound')
  assert.equal(document.claims.validity.expiry.mode, 'none')
  const shared = JSON.parse(readFileSync(new URL('../contracts/test-vectors/license.v2.json', import.meta.url), 'utf8')).cases.find(item => item.name === 'free_no_expiry').document.claims
  for (const key of ['plan_id', 'plan_version', 'edition', 'minimum_version', 'quota_policy_version']) assert.deepEqual(document.claims[key], shared[key])
  assert.deepEqual(document.claims.entitlements, profiles[0].policy.entitlement_ceiling)
  assert.deepEqual(document.claims.entitlements.quotas.map(item => item.limit.value), [3, 1, 1, 1])
  const canonical = value => Array.isArray(value) ? value.map(canonical) : value !== null && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map(key => [key, canonical(value[key])])) : value
  const key = createPublicKey({ key: Buffer.from(profiles[0].public_key_spki, 'base64url'), format: 'der', type: 'spki' })
  assert.equal(verify(null, Buffer.from(JSON.stringify(canonical(document.claims))), key, Buffer.from(document.signature, 'base64url')), true)
  assert.notEqual(createWindowsLicenseFixture('2.0.1-test.1', '2026-09-08T00:00:00.000Z').licenseKeys, fixture.licenseKeys)
})
