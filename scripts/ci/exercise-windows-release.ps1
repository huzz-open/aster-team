[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Archive,
    [Parameter(Mandatory = $true)][string]$Version,
    [Parameter(Mandatory = $true)][string]$CandidateVersion,
    [Parameter(Mandatory = $true)][string]$CandidateArchive,
    [Parameter(Mandatory = $true)][string]$ReleaseTool,
    [Parameter(Mandatory = $true)][string]$ReleaseSigningKey,
    [Parameter(Mandatory = $true)][string]$ReleaseSigningKeyId,
    [Parameter(Mandatory = $true)][string]$WorkRoot,
    [Parameter(Mandatory = $true)][string]$DiagnosticsRoot,
    [string]$PaidLicenseSigner = '',
    [string]$PaidLicensePrivateKey = '',
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

if ($Version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$' -or
    $CandidateVersion -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
    throw 'Windows release smoke versions are invalid.'
}
Assert-Leaf $Archive 'release archive'
Assert-Leaf $CandidateArchive 'candidate archive'
Assert-Leaf $ReleaseTool 'release tool'
Assert-Leaf $ReleaseSigningKey 'release signing key'

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$work = [IO.Path]::GetFullPath($WorkRoot)
$diagnostics = [IO.Path]::GetFullPath($DiagnosticsRoot)
$extract = Join-Path $work 'package'
$installRoot = Join-Path $work 'install'
$password = Join-Path $work 'owner-password.txt'
$activePassword = Join-Path $work 'owner-active-password.txt'
$failed = $true

if (Test-Path -LiteralPath $work) { throw "Windows release smoke work root already exists: $work" }
New-Item -ItemType Directory -Force -Path $extract | Out-Null
$previousEnvironment = Set-AsterSmokeEnvironment $ServicePrefix $PortOffset
try {
    tar.exe -xzf $Archive -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'Windows package extraction failed' }
    $package = Join-Path $extract "aster-team-$Version-windows-amd64"
    $layoutContract = Get-Content -LiteralPath (Join-Path $repositoryRoot 'contracts\install-layout.json') -Raw | ConvertFrom-Json
    & (Join-Path $package 'init.ps1') --install-root $installRoot
    if ($LASTEXITCODE -ne 0) { throw 'Windows initialization failed' }
    $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    [IO.File]::WriteAllText($password, 'Windows-release-owner-2026!', [Text.UTF8Encoding]::new($false))
    [IO.File]::WriteAllText($activePassword, 'Windows-release-active-owner-2026!', [Text.UTF8Encoding]::new($false))
    & $cli install --unattended --owner-email owner@example.test --owner-password-file $password --access-protocol http --access-host 127.0.0.1 --bind-address 127.0.0.1
    if ($LASTEXITCODE -ne 0) { throw 'Windows installation failed' }
    $instanceMarkerHash = (Get-FileHash -LiteralPath (Join-Path $installRoot 'install.json') -Algorithm SHA256).Hash
    $bundledFreeLicense = Join-Path $package 'licenses\free-license.json'
    $installedLicense = Join-Path $installRoot 'config\license\license.json'
    $initialLicenseHash = $null
    if (Test-Path -LiteralPath $bundledFreeLicense -PathType Leaf) {
        Assert-Leaf $installedLicense 'automatically installed free License'
        $initialLicenseHash = (Get-FileHash -LiteralPath $bundledFreeLicense -Algorithm SHA256).Hash
        if ((Get-FileHash -LiteralPath $installedLicense -Algorithm SHA256).Hash -ne $initialLicenseHash) { throw 'Windows free License differs from signed package bytes' }
        & (Join-Path $installRoot 'current\bin\aster-control.exe') verify-bundled-free-license --source $installedLicense
        if ($LASTEXITCODE -ne 0) { throw 'Installed Windows free License validation failed' }
    }

    # An unrelated terminal environment must not retarget any lifecycle action.
    [Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', 'must-not-be-used', 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_PORT_OFFSET', '40000', 'Process')
    $installedAsterctl = Join-Path $installRoot 'current\client-tools\asterctl\windows-x86_64\asterctl.exe'
    Assert-Leaf $installedAsterctl 'installed asterctl'
    & $installedAsterctl version
    if ($LASTEXITCODE -ne 0) { throw 'Installed asterctl smoke test failed' }
    & $cli status
    if ($LASTEXITCODE -ne 0) { throw 'Windows status failed' }
    & $cli doctor
    if ($LASTEXITCODE -ne 0) { throw 'Windows diagnostics failed' }
    $request = Join-Path $installRoot 'backups\windows-license-request.json'
    & $cli license request --output $request
    $requestQr = [IO.Path]::ChangeExtension($request, 'qr.png')
    if ($LASTEXITCODE -ne 0 -or
        -not (Test-Path -LiteralPath $request -PathType Leaf) -or
        -not (Test-Path -LiteralPath $requestQr -PathType Leaf)) {
        throw 'Windows machine-bound license request failed'
    }
    if ($PaidLicenseSigner -or $PaidLicensePrivateKey) {
        Assert-Leaf $PaidLicenseSigner 'test-only paid License signer'
        Assert-Leaf $PaidLicensePrivateKey 'test-only paid License private key'
        if (-not $initialLicenseHash) { throw 'Paid switch test requires an automatically installed free License' }
        $paidLicense = Join-Path $work 'test-paid-license.json'
        & $PaidLicenseSigner --installation-profile (Join-Path $installRoot 'config\license\installation.json') --private-key $PaidLicensePrivateKey --minimum-version $Version --output $paidLicense
        if ($LASTEXITCODE -ne 0) { throw 'Temporary Windows paid License signing failed' }
        & $cli license install --source $paidLicense
        if ($LASTEXITCODE -ne 0) { throw 'Windows free-to-paid License import failed' }
        $initialLicenseHash = (Get-FileHash -LiteralPath $paidLicense -Algorithm SHA256).Hash
        if ((Get-FileHash -LiteralPath $installedLicense -Algorithm SHA256).Hash -ne $initialLicenseHash) { throw 'Windows paid License differs from the signed source' }
    }
    & $cli backup create
    if ($LASTEXITCODE -ne 0) { throw 'Windows backup failed' }
    $backup = Get-ChildItem -LiteralPath (Join-Path $installRoot 'backups') -Filter '*.tar.gz' -File |
        Sort-Object LastWriteTimeUtc -Descending | Select-Object -First 1
    if (-not $backup) { throw 'Windows backup was not created' }
    & $cli backup restore --source $backup.FullName --confirm
    if ($LASTEXITCODE -ne 0) { throw 'Windows backup restore failed' }
    $restoreCleanupDeadline = [DateTime]::UtcNow.AddSeconds(30)
    $restoreStaging = Join-Path $installRoot 'staging\restores'
    while ((Test-Path -LiteralPath $restoreStaging) -and
           @(Get-ChildItem -LiteralPath $restoreStaging -Force -ErrorAction SilentlyContinue).Count -gt 0 -and
           [DateTime]::UtcNow -lt $restoreCleanupDeadline) {
        Start-Sleep -Milliseconds 250
    }
    if ((Test-Path -LiteralPath $restoreStaging) -and
        @(Get-ChildItem -LiteralPath $restoreStaging -Force -ErrorAction SilentlyContinue).Count -gt 0) {
        throw 'Windows deferred restore cleanup did not finish'
    }
    $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    & $cli status
    if ($LASTEXITCODE -ne 0) { throw 'Windows post-restore status failed' }
    & (Join-Path $repositoryRoot 'scripts\ci\exercise-windows-upgrade.ps1') `
        -InstallRoot $installRoot `
        -DefaultInstallRoot ([string]$layoutContract.default_roots.windows) `
        -BaseVersion $Version `
        -CandidateVersion $CandidateVersion `
        -CandidateArchive $CandidateArchive `
        -ReleaseTool $ReleaseTool `
        -ReleaseSigningKey $ReleaseSigningKey `
        -ReleaseSigningKeyId $ReleaseSigningKeyId `
        -OwnerEmail 'owner@example.test' `
        -InitialPasswordFile $password `
        -ActivePasswordFile $activePassword
    if ($LASTEXITCODE -ne 0) { throw 'Windows upgrade lifecycle smoke failed' }
    if ($initialLicenseHash -and (Get-FileHash -LiteralPath $installedLicense -Algorithm SHA256).Hash -ne $initialLicenseHash) { throw 'Restore, upgrade or rollback replaced the installed Windows License' }
    $marker = Get-Content -LiteralPath (Join-Path $installRoot 'install.json') -Raw | ConvertFrom-Json
    if (-not [string]::Equals(
        [IO.Path]::GetFullPath([string]$marker.root).TrimEnd('\'),
        [IO.Path]::GetFullPath($installRoot).TrimEnd('\'),
        [StringComparison]::OrdinalIgnoreCase
    )) { throw 'Windows installation marker escaped the configured root' }
    if ((Get-FileHash -LiteralPath (Join-Path $installRoot 'install.json') -Algorithm SHA256).Hash -ne $instanceMarkerHash) {
        throw 'Install, restore or upgrade changed persisted instance configuration'
    }
    & $cli service stop control
    if ($LASTEXITCODE -ne 0) { throw 'Windows isolated Control stop failed' }
    & $cli service start control
    if ($LASTEXITCODE -ne 0) { throw 'Windows isolated Control start failed' }
    $readyDeadline = [DateTime]::UtcNow.AddSeconds(60)
    do {
        & $cli status
        if ($LASTEXITCODE -eq 0) { break }
        if ([DateTime]::UtcNow -ge $readyDeadline) { throw 'Windows isolated restart readiness failed' }
        Start-Sleep -Milliseconds 500
    } while ($true)
    & $cli uninstall
    if ($LASTEXITCODE -ne 0) { throw 'Windows isolated uninstall failed' }
    $uninstallDeadline = [DateTime]::UtcNow.AddSeconds(60)
    while (Test-Path -LiteralPath (Join-Path $installRoot 'bin')) {
        if ([DateTime]::UtcNow -ge $uninstallDeadline) { throw 'Windows uninstall deferred cleanup timed out' }
        Start-Sleep -Milliseconds 250
    }
    if (@(Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -eq $script:AsterSmokeTaskPath }).Count) {
        throw 'Windows uninstall left instance tasks registered'
    }
    Assert-Leaf (Join-Path $installRoot 'data\database\aster-team.db') 'preserved database after uninstall'
    if ((Get-FileHash -LiteralPath (Join-Path $installRoot 'install.json') -Algorithm SHA256).Hash -ne $instanceMarkerHash) {
        throw 'Windows uninstall changed the retained instance configuration'
    }
    $failed = $false
    Write-Output 'Windows install, backup, restore and upgrade smoke test passed.'
}
finally {
    Restore-AsterSmokeEnvironment $previousEnvironment
    if ($failed) {
        New-Item -ItemType Directory -Force -Path $diagnostics | Out-Null
        if (Test-Path -LiteralPath (Join-Path $installRoot 'logs')) {
            Copy-Item -LiteralPath (Join-Path $installRoot 'logs') -Destination $diagnostics -Recurse -Force
        }
        # Retain job and slot evidence before removing this owned installation.
        # Do not copy databases, credentials, keys or package upload payloads.
        foreach ($name in @('upgrades', 'slots')) {
            $statePath = Join-Path $installRoot "state\$name"
            if (Test-Path -LiteralPath $statePath) {
                Copy-Item -LiteralPath $statePath -Destination (Join-Path $diagnostics "state-$name") -Recurse -Force
            }
        }
        foreach ($taskName in @('Control Blue','Control Green','Runner','Caddy','Maintenance')) {
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
