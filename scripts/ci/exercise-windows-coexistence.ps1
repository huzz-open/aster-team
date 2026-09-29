[CmdletBinding()]
param([Parameter(Mandatory = $true)][string]$Configuration)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest
. (Join-Path $PSScriptRoot 'windows-smoke-cleanup.ps1')
$options = Get-Content -LiteralPath $Configuration -Raw | ConvertFrom-Json
Assert-AsterSmokeHost 'lab-a' 10000
Assert-AsterSmokeHost 'lab-b' 20000
Assert-AsterSmokeHost 'lab-runner' 30000
$script:AsterSmokeTaskPath = Get-AsterSmokeTaskPath 'lab-a'
$lab = [IO.Path]::GetFullPath([string]$options.WorkRoot)
if (Test-Path -LiteralPath $lab) { throw "Lab root already exists; preserve it for inspection: $lab" }
if ([string]::Equals($lab.TrimEnd('\'), [IO.Path]::GetPathRoot($lab).TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) { throw 'Lab root cannot be a drive root' }
$work = Join-Path $lab 'anchor'
$installRoot = Join-Path $work 'install'
$extract = Join-Path $work 'package'
$diagnostics = [IO.Path]::GetFullPath([string]$options.DiagnosticsRoot)
$previous = Set-AsterSmokeEnvironment 'lab-a' 10000
$probe = $null
$failed = $true
$locationPushed = $false

function Get-UnrelatedTaskDefinitions {
    $tasks = @(Get-ScheduledTask -ErrorAction Stop | Where-Object {
        $_.TaskPath -like '\Aster Team\*' -and $_.TaskPath -notin @('\Aster Team\lab-a\', '\Aster Team\lab-b\', '\Aster Team\lab-runner\')
    })
    $result = @{}
    foreach ($task in $tasks) { $result[$task.TaskPath + $task.TaskName] = Export-ScheduledTask -TaskName $task.TaskName -TaskPath $task.TaskPath }
    return $result
}

$unrelatedTasks = Get-UnrelatedTaskDefinitions
New-Item -ItemType Directory -Force -Path $extract, $diagnostics | Out-Null
try {
    # The CLI exports an automatic license request in its working directory.
    # Keep those test artifacts with diagnostics, including npm entry-point runs.
    Push-Location -LiteralPath $diagnostics
    $locationPushed = $true
    & tar.exe -xzf ([string]$options.Archive) -C $extract
    if ($LASTEXITCODE -ne 0) { throw 'Anchor package extraction failed' }
    $package = Join-Path $extract "aster-team-$($options.Version)-windows-amd64"
    & (Join-Path $package 'init.ps1') --install-root $installRoot
    if ($LASTEXITCODE -ne 0) { throw 'Anchor initialization failed' }
    $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    $password = Join-Path $work 'owner-password.txt'
    [IO.File]::WriteAllText($password, 'Windows-anchor-independent-2026!', [Text.UTF8Encoding]::new($false))
    & $cli install --unattended --owner-email anchor@example.test --owner-password-file $password --access-protocol http --access-host 127.0.0.1 --bind-address 127.0.0.1
    if ($LASTEXITCODE -ne 0) { throw 'Anchor installation failed' }
    $freeLicense = Join-Path $package 'licenses\free-license.json'
    if (Test-Path -LiteralPath $freeLicense -PathType Leaf) {
        if ((Get-FileHash -LiteralPath (Join-Path $installRoot 'config\license\license.json') -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $freeLicense -Algorithm SHA256).Hash) { throw 'Anchor free License was not automatically installed' }
    }
    $hashes = @{}
    foreach ($relative in @('install.json', 'config\keys\database.key', 'config\keys\installation.key', 'config\license\installation.json', 'config\control\access.json', 'config\caddy\Caddyfile')) {
        $hashes[$relative] = (Get-FileHash -LiteralPath (Join-Path $installRoot $relative) -Algorithm SHA256).Hash
    }
    $runningTasks = @{}
    foreach ($name in @('Control Blue', 'Caddy')) {
        $runningTasks[$name] = (Get-ScheduledTaskInfo -TaskName $name -TaskPath $script:AsterSmokeTaskPath).LastRunTime
    }
    $stopFile = Join-Path $work 'stop-probe'
    $probe = Start-Job -ScriptBlock {
        param($StopFile)
        $ErrorActionPreference = 'Stop'
        while (-not (Test-Path -LiteralPath $StopFile)) {
            foreach ($uri in @('http://127.0.0.1:21080/healthz', 'http://127.0.0.1:21082/')) {
                $response = Invoke-WebRequest -Uri $uri -UseBasicParsing -TimeoutSec 5
                if ($response.StatusCode -ne 200) { throw "Anchor became unavailable: $uri" }
            }
            Start-Sleep -Milliseconds 500
        }
    } -ArgumentList $stopFile
    $controlArguments = @{}
    foreach ($name in @('Archive', 'Version', 'CandidateArchive', 'CandidateVersion', 'ReleaseTool', 'ReleaseSigningKey', 'ReleaseSigningKeyId')) {
        $controlArguments[$name] = [string]$options.$name
    }
    $controlArguments.WorkRoot = Join-Path $lab 'control'
    $controlArguments.DiagnosticsRoot = Join-Path $diagnostics 'control'
    $controlArguments.ServicePrefix = 'lab-b'
    $controlArguments.PortOffset = 20000
    foreach ($name in @('PaidLicenseSigner', 'PaidLicensePrivateKey')) {
        if ($options.PSObject.Properties.Name -contains $name) { $controlArguments[$name] = [string]$options.$name }
    }
    & (Join-Path $PSScriptRoot 'exercise-windows-release.ps1') @controlArguments
    & (Join-Path $PSScriptRoot 'exercise-windows-runner-install.ps1') `
        -Archive ([string]$options.Archive) -Version ([string]$options.Version) `
        -WorkRoot (Join-Path $lab 'runner') -DiagnosticsRoot (Join-Path $diagnostics 'runner') `
        -ServicePrefix 'lab-runner' -PortOffset 30000
    New-Item -ItemType File -Path $stopFile | Out-Null
    Wait-Job -Job $probe -Timeout 15 | Out-Null
    Receive-Job -Job $probe -ErrorAction Stop
    if ($probe.State -ne 'Completed') { throw "Anchor availability probe ended as $($probe.State)" }
    foreach ($relative in $hashes.Keys) {
        if ((Get-FileHash -LiteralPath (Join-Path $installRoot $relative) -Algorithm SHA256).Hash -ne $hashes[$relative]) {
            throw "Another instance changed anchor identity or configuration: $relative"
        }
    }
    foreach ($name in $runningTasks.Keys) {
        $task = Get-ScheduledTask -TaskName $name -TaskPath $script:AsterSmokeTaskPath
        if ($task.State -ne 'Running' -or (Get-ScheduledTaskInfo -InputObject $task).LastRunTime -ne $runningTasks[$name]) {
            throw "Another instance stopped or restarted anchor task: $name"
        }
    }
    $session = [Microsoft.PowerShell.Commands.WebRequestSession]::new()
    $login = Invoke-RestMethod -Uri 'http://127.0.0.1:21082/api/admin/auth/login' -Method Post -WebSession $session -ContentType 'application/json' `
        -Body (@{ email = 'anchor@example.test'; password = [IO.File]::ReadAllText($password) } | ConvertTo-Json -Compress)
    if (-not $login.password_change_required) { throw 'Anchor database identity changed' }
    $currentTasks = Get-UnrelatedTaskDefinitions
    foreach ($name in $unrelatedTasks.Keys) {
        if (-not $currentTasks.ContainsKey($name) -or $currentTasks[$name] -cne $unrelatedTasks[$name]) { throw "Unrelated scheduled task changed: $name" }
    }
    $failed = $false
    Write-Output 'Windows coexisting instances passed: Control lifecycle, Runner lifecycle, anchor availability, identity and task isolation.'
} finally {
    if ($locationPushed) { Pop-Location }
    Restore-AsterSmokeEnvironment $previous
    if ($null -ne $probe) {
        if ($probe.State -eq 'Running') { Stop-Job -Job $probe }
        Remove-Job -Job $probe -Force
    }
    if ($failed -and (Test-Path -LiteralPath (Join-Path $installRoot 'logs'))) {
        Copy-Item -LiteralPath (Join-Path $installRoot 'logs') -Destination (Join-Path $diagnostics 'anchor') -Recurse -Force
    }
    try { Complete-AsterSmokeCleanup }
    catch {
        if (-not $failed) { throw }
        Write-Warning "Anchor cleanup failed; retained $work. $($_.Exception.Message)"
    }
    # Child roots are individually owned and cleaned by their lifecycle scripts.
    # Remove only an empty coordinator directory, never recurse here.
    if ((Test-Path -LiteralPath $lab) -and @(Get-ChildItem -LiteralPath $lab -Force).Count -eq 0) { [IO.Directory]::Delete($lab) }
}
