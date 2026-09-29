param(
    [Parameter(Mandatory = $true)][string]$InstallRoot,
    [Parameter(Mandatory = $true)][string]$DefaultInstallRoot,
    [Parameter(Mandatory = $true)][string]$BaseVersion,
    [Parameter(Mandatory = $true)][string]$CandidateVersion,
    [Parameter(Mandatory = $true)][string]$CandidateArchive,
    [Parameter(Mandatory = $true)][string]$ReleaseTool,
    [Parameter(Mandatory = $true)][string]$ReleaseSigningKey,
    [Parameter(Mandatory = $true)][string]$ReleaseSigningKeyId,
    [Parameter(Mandatory = $true)][string]$OwnerEmail,
    [Parameter(Mandatory = $true)][string]$InitialPasswordFile,
    [Parameter(Mandatory = $true)][string]$ActivePasswordFile
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Assert-Leaf([string]$Path, [string]$Label) {
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) { throw "$Label is missing: $Path" }
}

function Invoke-Json([string]$Uri, [string]$Method, [object]$Body, [Microsoft.PowerShell.Commands.WebRequestSession]$Session) {
    $arguments = @{
        Uri = $Uri
        Method = $Method
        WebSession = $Session
        TimeoutSec = 15
    }
    if ($null -ne $Body) {
        $arguments.ContentType = 'application/json'
        $arguments.Body = ($Body | ConvertTo-Json -Compress)
    }
    Invoke-RestMethod @arguments
}

function Invoke-MaintenanceTask {
    Start-ScheduledTask -TaskName 'Maintenance' -TaskPath $script:AsterUpgradeTaskPath
}

function Wait-MaintenanceJob(
    [string]$JobId,
    [string]$ExpectedStatus,
    [Microsoft.PowerShell.Commands.WebRequestSession]$Session,
    [int]$TimeoutSeconds = 210
) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    $nextRun = [DateTime]::MinValue
    while ([DateTime]::UtcNow -lt $deadline) {
        try { $state = Invoke-Json "$script:AdminBase/api/admin/maintenance" 'GET' $null $Session }
        catch {
            $responseProperty = $_.Exception.PSObject.Properties['Response']
            $response = if ($responseProperty) { $responseProperty.Value } else { $null }
            if ($response -and [int]$response.StatusCode -notin @(502, 503, 504)) { throw }
            Start-Sleep -Milliseconds 250
            continue
        }
        $job = @($state.jobs | Where-Object { $_.id -eq $JobId }) | Select-Object -First 1
        if ($job) {
            if ($job.status -eq $ExpectedStatus) { return $job }
            if ($job.status -in @('succeeded', 'failed')) {
                throw "maintenance job $JobId ended as $($job.status), expected ${ExpectedStatus}: $($job.message)"
            }
            if ($job.status -eq 'queued' -and [DateTime]::UtcNow -ge $nextRun) {
                Invoke-MaintenanceTask
                $nextRun = [DateTime]::UtcNow.AddSeconds(3)
            }
        }
        Start-Sleep -Milliseconds 250
    }
    throw "maintenance job $JobId did not reach $ExpectedStatus within $TimeoutSeconds seconds"
}

function Queue-Upgrade(
    [string]$Archive,
    [Microsoft.PowerShell.Commands.WebRequestSession]$Session
) {
    Assert-Leaf $Archive 'upgrade archive'
    Invoke-RestMethod `
        -Uri "$script:AdminBase/api/admin/maintenance/upgrade" `
        -Method Post `
        -Form @{ package = Get-Item -LiteralPath $Archive } `
        -WebSession $Session `
        -TimeoutSec 120
}

function Queue-VersionDeletion(
    [string]$Version,
    [Microsoft.PowerShell.Commands.WebRequestSession]$Session
) {
    Invoke-Json "$script:AdminBase/api/admin/maintenance/versions/$([Uri]::EscapeDataString($Version))" 'DELETE' $null $Session
}

function Start-AvailabilityProbe([string]$StopFile, [string]$LogFile, [switch]$AllowMaintenanceOutage) {
    [string[]]$uris = @(("$script:ApiBase/healthz"), ("$script:AdminBase/"), ("$script:MemberBase/"))
    Start-Job -ScriptBlock {
        param($Stop, $Log, $Uris, $AllowOutage)
        $ErrorActionPreference = 'Stop'
        while (-not (Test-Path -LiteralPath $Stop)) {
            foreach ($uri in $Uris) {
                $started = [DateTime]::UtcNow
                $timer = [Diagnostics.Stopwatch]::StartNew()
                try {
                    $response = Invoke-WebRequest -Uri $uri -TimeoutSec 3 -UseBasicParsing
                    if ($response.StatusCode -ne 200) { throw "HTTP $($response.StatusCode)" }
                } catch {
                    $failure = [ordered]@{
                        started_at = $started.ToString('O')
                        failed_at = [DateTime]::UtcNow.ToString('O')
                        uri = $uri
                        elapsed_ms = $timer.ElapsedMilliseconds
                        error = $_.Exception.Message
                    }
                    Add-Content -LiteralPath $Log -Value ($failure | ConvertTo-Json -Compress)
                    if (-not $AllowOutage) { throw }
                }
            }
            Start-Sleep -Milliseconds 100
        }
    } -ArgumentList $StopFile, $LogFile, $uris, $AllowMaintenanceOutage.IsPresent
}

function Stop-AvailabilityProbe([System.Management.Automation.Job]$Job, [string]$StopFile) {
    New-Item -ItemType File -Path $StopFile -Force | Out-Null
    try {
        Wait-Job -Job $Job -Timeout 10 | Out-Null
        Receive-Job -Job $Job -ErrorAction Stop
        if ($Job.State -ne 'Completed') { throw "availability probe ended as $($Job.State)" }
    } finally {
        if ($Job.State -eq 'Running') { Stop-Job -Job $Job }
        Remove-Job -Job $Job -Force
    }
}

function Finish-UpgradeProbe(
    [System.Management.Automation.Job]$Job,
    [string]$StopFile,
    [string]$LogFile,
    [System.Management.Automation.ErrorRecord]$PrimaryFailure,
    [switch]$AllowMaintenanceOutage
) {
    $probeFailure = $null
    try { Stop-AvailabilityProbe $Job $StopFile }
    catch { $probeFailure = $_ }
    $details = ''
    $diagnosticFailure = $null
    try {
        if (Test-Path -LiteralPath $LogFile -PathType Leaf) { $details = [IO.File]::ReadAllText($LogFile) }
    } catch { $diagnosticFailure = $_ }
    $probeMessage = if ($probeFailure) { $probeFailure.Exception.Message } else { 'Recorded failed request' }
    if ($diagnosticFailure) { $probeMessage += "; probe diagnostic could not be read: $($diagnosticFailure.Exception.Message)" }
    if ($PrimaryFailure) {
        if ($probeFailure -or $details -or $diagnosticFailure) {
            [Console]::Error.WriteLine("Availability probe also failed: $probeMessage`n$details")
        }
        # A finally-block exception must not replace the lifecycle operation that
        # failed first. Its original ErrorRecord includes the script location.
        throw $PrimaryFailure
    }
    if ($probeFailure -or ($details -and -not $AllowMaintenanceOutage) -or $diagnosticFailure) {
        throw "Availability probe failed: $probeMessage`n$details"
    }
    if ($details) { Write-Output "Recorded maintenance-window outages:`n$details" }
}

function Assert-ControlSlotState([string]$ActiveSlot) {
    if ($ActiveSlot -notin @('blue', 'green')) { throw "invalid active Control slot: $ActiveSlot" }
    $activeName = if ($ActiveSlot -eq 'blue') { 'Control Blue' } else { 'Control Green' }
    $inactiveName = if ($ActiveSlot -eq 'blue') { 'Control Green' } else { 'Control Blue' }
    $activeTask = Get-ScheduledTask -TaskName $activeName -TaskPath $script:AsterUpgradeTaskPath
    $inactiveTask = Get-ScheduledTask -TaskName $inactiveName -TaskPath $script:AsterUpgradeTaskPath
    if ($activeTask.State -ne 'Running') { throw "$activeName task is $($activeTask.State), expected Running" }
    if ($inactiveTask.State -eq 'Running') { throw "$inactiveName task is still Running" }
}

function Assert-SelectedRelease([string]$Version) {
    $info = & (Join-Path $InstallRoot 'bin\aster-team-cli.exe') info
    if ($LASTEXITCODE -ne 0 -or @($info) -cnotcontains "selected release: $Version") {
        throw "Installed CLI cannot verify the selected release $Version"
    }
    $info | Out-Host
    $current = (Get-Item -LiteralPath (Join-Path $InstallRoot 'current')).Target
    if (-not ([string]$current).EndsWith("releases\$Version", [StringComparison]::OrdinalIgnoreCase)) {
        throw "stable current link does not select $Version"
    }
}

function Assert-ScheduledTasksUseInstallRoot([string]$Root) {
    foreach ($name in @('Control Blue', 'Control Green', 'Runner', 'Caddy', 'Maintenance')) {
        $task = Get-ScheduledTask -TaskName $name -TaskPath $script:AsterUpgradeTaskPath
        $actionText = @($task.Actions | ForEach-Object { "$($_.Execute) $($_.Arguments)" }) -join "`n"
        if ($actionText.IndexOf($Root, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw "$name scheduled task does not reference the configured installation root"
        }
    }
}

function Create-BrokenCandidate([string]$SourceArchive, [string]$Version, [string]$Workspace) {
    $extract = Join-Path $Workspace 'extract'
    New-Item -ItemType Directory -Force -Path $extract | Out-Null
    & tar.exe -xzf $SourceArchive -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'candidate fixture extraction failed' }
    $source = Get-ChildItem -LiteralPath $extract -Directory | Select-Object -First 1
    if (-not $source) { throw 'candidate fixture has no top-level release directory' }
    $broken = Join-Path $Workspace "aster-team-$Version-windows-amd64"
    Copy-Item -LiteralPath $source.FullName -Destination $broken -Recurse
    Remove-Item -LiteralPath (Join-Path $broken 'RELEASE.json') -Force
    [IO.File]::WriteAllText((Join-Path $broken 'VERSION'), "$Version`n", [Text.UTF8Encoding]::new($false))
    Copy-Item -LiteralPath $ReleaseTool -Destination (Join-Path $broken 'bin\aster-control.exe') -Force
    & $ReleaseTool sign `
        --root $broken `
        --private-key $ReleaseSigningKey `
        --key-id $ReleaseSigningKeyId `
        --version $Version `
        --architecture amd64 `
        --runtime msvc `
        --platform windows `
        --created-at ([DateTime]::UtcNow.ToString('yyyy-MM-ddTHH:mm:ss.fffZ')) | Out-Host
    if ($LASTEXITCODE -ne 0) { throw 'broken candidate signing failed' }
    $archive = Join-Path $Workspace "aster-team-$Version-windows-amd64.tar.gz"
    & tar.exe -czf $archive -C $Workspace (Split-Path -Leaf $broken)
    if ($LASTEXITCODE -ne 0) { throw 'broken candidate archive creation failed' }
    return $archive
}

$InstallRoot = [IO.Path]::GetFullPath($InstallRoot).TrimEnd('\')
$defaultRoot = [IO.Path]::GetFullPath($DefaultInstallRoot).TrimEnd('\')
if ([string]::Equals($InstallRoot, $defaultRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'Smoke installation must not use the default root.'
}
$marker = Get-Content -LiteralPath (Join-Path $InstallRoot 'install.json') -Raw | ConvertFrom-Json
if (-not [string]::Equals([IO.Path]::GetFullPath([string]$marker.root).TrimEnd('\'), $InstallRoot, [StringComparison]::OrdinalIgnoreCase)) {
    throw 'installation escaped the configured root'
}
$instanceJson = & (Join-Path $InstallRoot 'bin\aster-team-cli.exe') windows-instance
if ($LASTEXITCODE -ne 0) { throw 'Installed instance configuration is invalid.' }
$instance = ($instanceJson -join "`n") | ConvertFrom-Json
$script:AsterUpgradeTaskPath = [string]$instance.task_path
$access = Get-Content -LiteralPath (Join-Path $InstallRoot 'config\control\access.json') -Raw | ConvertFrom-Json
$script:AdminBase = [string]$access.admin_url
$script:ApiBase = [string]$access.api_url
$script:MemberBase = [string]$access.member_url
Assert-ScheduledTasksUseInstallRoot $InstallRoot
$CandidateArchive = [IO.Path]::GetFullPath($CandidateArchive)
$ReleaseTool = [IO.Path]::GetFullPath($ReleaseTool)
$ReleaseSigningKey = [IO.Path]::GetFullPath($ReleaseSigningKey)
foreach ($item in @(
    @($CandidateArchive, 'candidate archive'),
    @($ReleaseTool, 'release tool'),
    @($ReleaseSigningKey, 'release signing key'),
    @($InitialPasswordFile, 'initial password file'),
    @($ActivePasswordFile, 'active password file')
)) { Assert-Leaf $item[0] $item[1] }

$initialPassword = [IO.File]::ReadAllText($InitialPasswordFile).Trim()
$activePassword = [IO.File]::ReadAllText($ActivePasswordFile).Trim()
if ($initialPassword.Length -lt 12 -or $activePassword.Length -lt 12 -or $initialPassword -eq $activePassword) {
    throw 'CI owner passwords do not satisfy the password-change contract'
}

$session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
$login = Invoke-Json "$script:AdminBase/api/admin/auth/login" 'POST' @{ email = $OwnerEmail; password = $initialPassword } $session
if ($login.password_change_required) {
    Invoke-Json "$script:AdminBase/api/admin/auth/password" 'POST' @{
        current_password = $initialPassword
        new_password = $activePassword
    } $session | Out-Null
    $session = New-Object Microsoft.PowerShell.Commands.WebRequestSession
    $login = Invoke-Json "$script:AdminBase/api/admin/auth/login" 'POST' @{ email = $OwnerEmail; password = $activePassword } $session
}
if ($login.password_change_required) { throw 'owner password change did not complete' }

$probeRoot = Join-Path (Join-Path $InstallRoot 'staging') "ci-upgrade-probe-$([Guid]::NewGuid().ToString('N'))"
New-Item -ItemType Directory -Path $probeRoot | Out-Null
$stopFile = Join-Path $probeRoot 'stop'
$probeLog = Join-Path $InstallRoot 'logs\upgrade-availability.log'
$probe = Start-AvailabilityProbe $stopFile $probeLog -AllowMaintenanceOutage
$upgradeFailure = $null
try {
    $upgrade = Queue-Upgrade $CandidateArchive $session
    Wait-MaintenanceJob $upgrade.id 'succeeded' $session | Out-Null
    Write-Output "Windows upgrade job $($upgrade.id) succeeded; checking active and selected releases."

    $active = Get-Content -LiteralPath (Join-Path $InstallRoot 'state\slots\active.json') -Raw | ConvertFrom-Json
    if ($active.version -ne $CandidateVersion) { throw "active slot is $($active.version), expected $CandidateVersion" }
    Assert-ControlSlotState $active.slot
    Assert-SelectedRelease $CandidateVersion
    foreach ($uri in @("$script:ApiBase/healthz", "$script:AdminBase/", "$script:MemberBase/")) {
        $response = Invoke-WebRequest -Uri $uri -TimeoutSec 10 -UseBasicParsing
        if ($response.StatusCode -ne 200) { throw "Public access did not recover: $uri" }
    }

    $brokenVersionMatch = [regex]::Match($CandidateVersion, '^(\d+)\.(\d+)\.(\d+)')
    if (-not $brokenVersionMatch.Success) { throw 'candidate version cannot produce a failure fixture version' }
    $brokenVersion = "$($brokenVersionMatch.Groups[1].Value).$($brokenVersionMatch.Groups[2].Value).$([int64]$brokenVersionMatch.Groups[3].Value + 1)-ci-fail"
    $fixtureWorkspace = Join-Path (Join-Path $InstallRoot 'staging') "ci-broken-upgrade-$([Guid]::NewGuid().ToString('N'))"
    New-Item -ItemType Directory -Path $fixtureWorkspace | Out-Null
    try {
        $brokenArchive = Create-BrokenCandidate $CandidateArchive $brokenVersion $fixtureWorkspace
        $failed = Queue-Upgrade $brokenArchive $session
        Wait-MaintenanceJob $failed.id 'failed' $session | Out-Null
        Write-Output "Windows rejected-candidate job $($failed.id) finished; checking rollback state."
    } finally {
        Remove-Item -LiteralPath $fixtureWorkspace -Recurse -Force -ErrorAction SilentlyContinue
    }

    $activeAfterFailure = Get-Content -LiteralPath (Join-Path $InstallRoot 'state\slots\active.json') -Raw | ConvertFrom-Json
    if ($activeAfterFailure.version -ne $CandidateVersion) { throw 'failed upgrade changed the active version' }
    if ($activeAfterFailure.slot -ne $active.slot) { throw 'failed upgrade changed the active slot' }
    Assert-ControlSlotState $activeAfterFailure.slot
    Assert-SelectedRelease $CandidateVersion
    foreach ($uri in @("$script:ApiBase/healthz", "$script:AdminBase/", "$script:MemberBase/")) {
        $response = Invoke-WebRequest -Uri $uri -TimeoutSec 10 -UseBasicParsing
        if ($response.StatusCode -ne 200) { throw "Public access did not recover: $uri" }
    }
    Write-Output 'Windows rollback state verified; checking historical-version cleanup.'

    $currentDeletionRefused = $false
    try {
        Queue-VersionDeletion $CandidateVersion $session | Out-Null
    } catch {
        $status = [int]$_.Exception.Response.StatusCode
        if ($status -eq 400) { $currentDeletionRefused = $true } else { throw }
    }
    if (-not $currentDeletionRefused) { throw 'current version deletion was not refused' }

    foreach ($version in @($BaseVersion, $brokenVersion)) {
        $cleanup = Queue-VersionDeletion $version $session
        Wait-MaintenanceJob $cleanup.id 'succeeded' $session | Out-Null
        if (Test-Path -LiteralPath (Join-Path $InstallRoot "releases\$version")) {
            throw "historical release was not deleted: $version"
        }
        if (Test-Path -LiteralPath (Join-Path $InstallRoot "backups\upgrades\$version")) {
            throw "historical upgrade snapshots were not deleted: $version"
        }
    }
} catch {
    $upgradeFailure = $_
} finally {
    Finish-UpgradeProbe $probe $stopFile $probeLog $upgradeFailure -AllowMaintenanceOutage
    Remove-Item -LiteralPath $probeRoot -Recurse -Force -ErrorAction SilentlyContinue
}

Write-Output "Windows maintenance upgrade verified: $BaseVersion -> $CandidateVersion; failed candidate rolled back and historical versions were deleted."
