param(
    [Parameter(Mandatory = $true)][string]$Archive,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$WorkRoot,
    [Parameter(Mandatory = $true)][string]$DiagnosticsRoot,
    [string]$ServicePrefix = '',
    [int]$PortOffset = 0
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-Leaf([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "$Label is missing: $Path" }
}

. (Join-Path $PSScriptRoot 'windows-smoke-cleanup.ps1')
Assert-AsterSmokeHost $ServicePrefix $PortOffset

if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { throw 'Windows Runner smoke version is invalid' }
$archivePath = [IO.Path]::GetFullPath($Archive)
$checksumPath = "$archivePath.sha256"
Assert-Leaf $archivePath 'Windows package'
Assert-Leaf $checksumPath 'Windows package checksum'
$checksum = (Get-Content -LiteralPath $checksumPath -Raw).Trim()
$archiveName = [IO.Path]::GetFileName($archivePath)
if ($checksum -notmatch "^([0-9a-f]{64})  $([regex]::Escape($archiveName))$") { throw 'Windows package checksum file is invalid' }
$expectedDigest = $checksum.Substring(0, 64)
$actualDigest = (Get-FileHash -LiteralPath $archivePath -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualDigest -ne $expectedDigest) { throw 'Windows package checksum does not match' }

$work = [IO.Path]::GetFullPath($WorkRoot)
$diagnostics = [IO.Path]::GetFullPath($DiagnosticsRoot)
$extract = Join-Path $work 'package'
$installRoot = Join-Path $work 'install'
$backup = Join-Path $work 'runner-backup.tar.gz'
$failed = $true
if (Test-Path -LiteralPath $work) { throw "Windows Runner smoke work root already exists: $work" }
New-Item -ItemType Directory -Force -Path $extract | Out-Null
$previousEnvironment = Set-AsterSmokeEnvironment $ServicePrefix $PortOffset

try {
    tar.exe -xzf $archivePath -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'Windows package extraction failed' }
    $package = Join-Path $extract "aster-team-$Version-windows-amd64"
    & (Join-Path $package 'init.ps1') --install-root $installRoot
    if ($LASTEXITCODE -ne 0) { throw 'Windows Runner initialization failed' }
    $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    & $cli runner install
    if ($LASTEXITCODE -ne 0) { throw 'Windows Runner installation failed' }
    $instanceMarkerHash = (Get-FileHash -LiteralPath (Join-Path $installRoot 'install.json') -Algorithm SHA256).Hash
    [Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', 'must-not-be-used', 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_PORT_OFFSET', '40000', 'Process')

    $role = Join-Path $installRoot 'config\runner\install-role'
    Assert-Leaf $role 'Windows Runner role marker'
    if ((Get-Content -LiteralPath $role -Raw).Trim() -ne 'runner') { throw 'Windows Runner role marker is invalid' }
    $runnerTask = Get-ScheduledTask -TaskName 'Runner' -TaskPath $script:AsterSmokeTaskPath -ErrorAction Stop
    if ($runnerTask.State.ToString() -ne 'Disabled') { throw 'Unconfigured Windows Runner task must remain disabled' }
    foreach ($taskName in @('Control Blue', 'Control Green', 'Caddy', 'Maintenance')) {
        if (Get-ScheduledTask -TaskName $taskName -TaskPath $script:AsterSmokeTaskPath -ErrorAction SilentlyContinue) {
            throw "Runner-only installation created unexpected task: $taskName"
        }
    }
    if (Test-Path -LiteralPath (Join-Path $installRoot 'data\database\aster-team.db')) {
        throw 'Runner-only installation created a Control database'
    }

    & $cli runner backup create --output $backup
    if ($LASTEXITCODE -ne 0) { throw 'Windows Runner backup failed' }
    Assert-Leaf $backup 'Windows Runner backup'
    & (Join-Path $package 'init.ps1') --install-root $installRoot
    if ($LASTEXITCODE -ne 0) { throw 'Windows Runner reinitialization failed' }
    & $cli runner upgrade
    if ($LASTEXITCODE -eq 0) { throw 'A same-version Windows Runner candidate unexpectedly upgraded' }
    & $cli runner backup restore --source $backup --confirm
    if ($LASTEXITCODE -ne 0) { throw 'Windows Runner backup restore failed' }
    if ((Get-Content -LiteralPath $role -Raw).Trim() -ne 'runner') { throw 'Restored Windows Runner role marker is invalid' }
    $runnerTask = Get-ScheduledTask -TaskName 'Runner' -TaskPath $script:AsterSmokeTaskPath -ErrorAction Stop
    if ($runnerTask.State.ToString() -ne 'Disabled') { throw 'Restored unconfigured Windows Runner task must remain disabled' }
    if ((Get-FileHash -LiteralPath (Join-Path $installRoot 'install.json') -Algorithm SHA256).Hash -ne $instanceMarkerHash) {
        throw 'Runner bootstrap or restore changed persisted instance configuration'
    }

    $failed = $false
    Write-Output "Windows Runner-only install smoke test passed for Aster Team $Version."
}
finally {
    Restore-AsterSmokeEnvironment $previousEnvironment
    if ($failed) {
        New-Item -ItemType Directory -Force -Path $diagnostics | Out-Null
        if (Test-Path -LiteralPath (Join-Path $installRoot 'logs')) {
            Copy-Item -LiteralPath (Join-Path $installRoot 'logs') -Destination $diagnostics -Recurse -Force
        }
        foreach ($taskName in @('Control Blue', 'Control Green', 'Runner', 'Caddy', 'Maintenance')) {
            $task = Get-ScheduledTask -TaskName $taskName -TaskPath $script:AsterSmokeTaskPath -ErrorAction SilentlyContinue
            if ($task) {
                $task | Format-List * | Out-File (Join-Path $diagnostics 'tasks.txt') -Append -Encoding utf8
                $task | Get-ScheduledTaskInfo | Format-List * |
                    Out-File (Join-Path $diagnostics 'task-results.txt') -Append -Encoding utf8
            }
        }
    }
    try {
        Complete-AsterSmokeCleanup
    } catch {
        # Preserve the original lifecycle failure; cleanup alone still fails CI.
        if (-not $failed) { throw }
        Write-Warning "Windows smoke cleanup also failed; files remain at $work. $($_.Exception.Message)"
    }
}
