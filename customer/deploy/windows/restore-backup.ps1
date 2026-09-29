$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw $Message
}

function Invoke-Checked([string]$Program, [string[]]$Arguments, [string]$Label) {
    & $Program @Arguments | Out-Host
    if ($LASTEXITCODE -ne 0) { Fail "$Label exited with status $LASTEXITCODE" }
}

function Ensure-AsterTaskFolder {
    $scheduler = New-Object -ComObject 'Schedule.Service'
    $scheduler.Connect()
    $folder = $scheduler.GetFolder('\')
    foreach ($part in $script:AsterTaskPath.Trim('\').Split('\')) {
        try { $folder = $scheduler.GetFolder($folder.Path.TrimEnd('\') + '\' + $part) }
        catch { $folder = $folder.CreateFolder($part) }
    }
}

function Assert-AsterTaskOwnership([string]$Root) {
    $launcher = '"' + (Join-Path $Root 'config\services\service-launch.ps1') + '"'
    foreach ($task in @(Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -eq $script:AsterTaskPath })) {
        $actions = @($task.Actions)
        if ($actions.Count -ne 1 -or ([string]$actions[0].Arguments).IndexOf($launcher, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw "Scheduled task namespace belongs to another installation: $($task.TaskPath)$($task.TaskName)"
        }
    }
}

function Set-AsterTaskRuntimePolicy([string]$FullName) {
    $taskName = $FullName.Substring($FullName.LastIndexOf('\') + 1)
    $settings = New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -MultipleInstances IgnoreNew `
        -RestartCount 999 `
        -RestartInterval (New-TimeSpan -Minutes 1) `
        -StartWhenAvailable
    Set-ScheduledTask -TaskName $taskName -TaskPath $script:AsterTaskPath -Settings $settings | Out-Null
}

$scriptArguments = @($args)
function Read-OptionValue([string]$Name, [ref]$Index) {
    if ($Index.Value + 1 -ge $scriptArguments.Count) { Fail "$Name requires a value" }
    $Index.Value++
    return [string]$scriptArguments[$Index.Value]
}

$backup = ''
$installRootArgument = ''
$confirmed = $false
$runnerOnly = $false
for ($index = 0; $index -lt $scriptArguments.Count; $index++) {
    switch ([string]$scriptArguments[$index]) {
        '--backup' { $backup = Read-OptionValue '--backup' ([ref]$index) }
        '--install-root' { $installRootArgument = Read-OptionValue '--install-root' ([ref]$index) }
        '--confirm-restore' { $confirmed = $true }
        '--runner-only' { $runnerOnly = $true }
        default { Fail "Unknown internal restore option: $($scriptArguments[$index])" }
    }
}
if (-not $confirmed) { Fail '--confirm-restore is required.' }
if (-not [System.IO.Path]::IsPathRooted($installRootArgument)) { Fail '--install-root must be absolute.' }
if (-not [System.IO.Path]::IsPathRooted($backup) -or -not (Test-Path -LiteralPath $backup -PathType Leaf)) {
    Fail '--backup must reference an existing absolute file.'
}
$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run restore from an elevated terminal.'
}

$installRoot = [System.IO.Path]::GetFullPath($installRootArgument).TrimEnd('\')
$backup = [System.IO.Path]::GetFullPath($backup)
$instanceJson = & (Join-Path $installRoot 'bin\aster-team-cli.exe') windows-instance
if ($LASTEXITCODE -ne 0) { Fail 'Windows instance configuration is invalid.' }
$script:AsterInstance = ($instanceJson -join "`n") | ConvertFrom-Json
$script:AsterTaskPath = [string]$script:AsterInstance.task_path
Assert-AsterTaskOwnership $installRoot
$parent = Split-Path -Parent $installRoot
$rootName = Split-Path -Leaf $installRoot
$restoreId = [guid]::NewGuid().ToString('N')
$workspaceRoot = Join-Path $installRoot "staging\restores\$restoreId"
$preparedRoot = Join-Path $workspaceRoot 'prepared'
$rollbackRoot = Join-Path $workspaceRoot 'rollback'
$safetyBackup = Join-Path $installRoot "backups\aster-team-before-restore-$restoreId.tar.gz"
$restoreCompleted = $false
if (Test-Path -LiteralPath $workspaceRoot) {
    Fail 'Restore workspace already exists.'
}
New-Item -ItemType Directory -Path (Join-Path $installRoot 'staging\restores') -Force | Out-Null
New-Item -ItemType Directory -Path (Join-Path $installRoot 'backups') -Force | Out-Null
New-Item -ItemType Directory -Path $preparedRoot | Out-Null
New-Item -ItemType Directory -Path $rollbackRoot | Out-Null

$managedEntries = @('bin', 'releases', 'config', 'data', 'state', 'logs', 'current', 'install.json')
$tasks = [ordered]@{
    ($script:AsterTaskPath + 'Control Blue') = 'control-blue'
    ($script:AsterTaskPath + 'Control Green') = 'control-green'
    ($script:AsterTaskPath + 'Runner') = 'runner'
    ($script:AsterTaskPath + 'Caddy') = 'caddy'
    ($script:AsterTaskPath + 'Maintenance') = 'maintenance'
}
if ($runnerOnly) { $tasks = [ordered]@{ ($script:AsterTaskPath + 'Runner') = 'runner' } }

function Stop-AsterTasks {
    $launcher = Join-Path $installRoot 'config\services\service-launch.ps1'
    foreach ($service in $tasks.Values) {
        Invoke-Checked 'powershell.exe' @('-NoProfile', '-NonInteractive', '-ExecutionPolicy', 'Bypass', '-File', $launcher, '-Service', $service, '-Stop') "stop $service process tree"
    }
}

function Move-AsterEntry([string]$Source, [string]$Destination) {
    if ($null -ne (Get-Item -LiteralPath $Destination -Force -ErrorAction SilentlyContinue)) {
        Fail "Restore refuses to merge or overwrite an existing entry: $Destination"
    }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while ($true) {
        try { Move-Item -LiteralPath $Source -Destination $Destination -ErrorAction Stop; return }
        catch [System.IO.IOException], [System.UnauthorizedAccessException] {
            if ([DateTime]::UtcNow -ge $deadline) { throw }
            Start-Sleep -Milliseconds 100
        }
    }
}

function Move-ManagedEntries([string]$SourceRoot, [string]$DestinationRoot) {
    # Preflight every destination before moving anything; track only successful
    # moves so a partial failure cannot nest directories during rollback.
    foreach ($entry in $managedEntries) {
        if ($null -ne (Get-Item -LiteralPath (Join-Path $DestinationRoot $entry) -Force -ErrorAction SilentlyContinue)) {
            Fail "Restore destination is not empty: $DestinationRoot/$entry"
        }
    }
    $moved = [Collections.Generic.List[string]]::new()
    try {
        foreach ($entry in $managedEntries) {
            $source = Join-Path $SourceRoot $entry
            if ($null -ne (Get-Item -LiteralPath $source -Force -ErrorAction SilentlyContinue)) {
                Move-AsterEntry $source (Join-Path $DestinationRoot $entry)
                $moved.Add($entry)
            }
        }
    } catch {
        for ($index = $moved.Count - 1; $index -ge 0; $index--) {
            $entry = $moved[$index]
            Move-AsterEntry (Join-Path $DestinationRoot $entry) (Join-Path $SourceRoot $entry)
        }
        throw
    }
}

function Restore-AsterReferences([string]$Root, $References) {
    foreach ($reference in $References) {
        if ($reference.path -notin @('current', 'state/slots/blue-release', 'state/slots/green-release') -or
            $reference.version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') {
            Fail 'Invalid verified release reference.'
        }
        $target = Join-Path $Root "releases\$($reference.version)"
        if (-not (Test-Path -LiteralPath (Join-Path $target 'RELEASE.json') -PathType Leaf)) { Fail 'Verified release target is missing.' }
        New-Item -ItemType Junction -Path (Join-Path $Root $reference.path) -Target $target | Out-Null
    }
}

function Remove-AsterRestoreTree([string]$Path) {
    # Never recurse through a junction: rollback links point at the live root.
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        if ($item.PSIsContainer) { [IO.Directory]::Delete($item.FullName) }
        else { [IO.File]::Delete($item.FullName) }
        return
    }
    if ($item.PSIsContainer) {
        foreach ($child in Get-ChildItem -LiteralPath $Path -Force) { Remove-AsterRestoreTree $child.FullName }
        [IO.Directory]::Delete($item.FullName)
    } else {
        Remove-Item -LiteralPath $item.FullName -Force
    }
}

function Start-DeferredWorkspaceCleanup([string]$Workspace) {
    $restoreRoot = [System.IO.Path]::GetFullPath((Join-Path $installRoot 'staging\restores')).TrimEnd('\') + '\'
    $workspacePath = [System.IO.Path]::GetFullPath($Workspace).TrimEnd('\')
    if (-not $workspacePath.StartsWith($restoreRoot, [StringComparison]::OrdinalIgnoreCase)) {
        Fail 'Refused to schedule cleanup outside the restore staging root.'
    }
    $relative = $workspacePath.Substring($restoreRoot.Length)
    if ($relative -notmatch '^[0-9a-f]{32}$') {
        Fail 'Refused to schedule cleanup for an invalid restore workspace.'
    }

    $parentProcessId = 0
    try {
        $parentProcess = Get-CimInstance Win32_Process -Filter "ProcessId=$PID" -ErrorAction Stop
        $parentProcessId = [int]$parentProcess.ParentProcessId
    } catch {
        $parentProcessId = 0
    }
    $cleanup = "function Remove-AsterRestoreTree {`n" + ${function:Remove-AsterRestoreTree}.ToString() + "`n}`n" + @'
$ErrorActionPreference = 'SilentlyContinue'
$installRoot = [System.IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('ASTER_TEAM_CLEANUP_INSTALL_ROOT', 'Process')).TrimEnd('\')
$workspace = [System.IO.Path]::GetFullPath([Environment]::GetEnvironmentVariable('ASTER_TEAM_CLEANUP_WORKSPACE', 'Process')).TrimEnd('\')
$restoreRoot = [System.IO.Path]::GetFullPath((Join-Path $installRoot 'staging\restores')).TrimEnd('\') + '\'
if (-not $workspace.StartsWith($restoreRoot, [StringComparison]::OrdinalIgnoreCase)) { exit 0 }
$relative = $workspace.Substring($restoreRoot.Length)
if ($relative -notmatch '^[0-9a-f]{32}$') { exit 0 }
foreach ($name in @('ASTER_TEAM_CLEANUP_RESTORE_PID', 'ASTER_TEAM_CLEANUP_PARENT_PID')) {
    $processId = 0
    if ([int]::TryParse([Environment]::GetEnvironmentVariable($name, 'Process'), [ref]$processId) -and $processId -gt 0) {
        Wait-Process -Id $processId -ErrorAction SilentlyContinue
    }
}
if (Test-Path -LiteralPath $workspace -PathType Container) {
    Remove-AsterRestoreTree $workspace
}
'@
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($cleanup))
    [Environment]::SetEnvironmentVariable('ASTER_TEAM_CLEANUP_INSTALL_ROOT', $installRoot, 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_TEAM_CLEANUP_WORKSPACE', $workspacePath, 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_TEAM_CLEANUP_RESTORE_PID', [string]$PID, 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_TEAM_CLEANUP_PARENT_PID', [string]$parentProcessId, 'Process')
    try {
        $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
        $process = Start-Process -FilePath $powershell `
            -ArgumentList @('-NoProfile', '-NonInteractive', '-WindowStyle', 'Hidden', '-EncodedCommand', $encoded) `
            -WindowStyle Hidden `
            -PassThru
        if ($null -eq $process) { Fail 'Could not start deferred restore cleanup.' }
    } finally {
        foreach ($name in @('ASTER_TEAM_CLEANUP_INSTALL_ROOT', 'ASTER_TEAM_CLEANUP_WORKSPACE', 'ASTER_TEAM_CLEANUP_RESTORE_PID', 'ASTER_TEAM_CLEANUP_PARENT_PID')) {
            [Environment]::SetEnvironmentVariable($name, $null, 'Process')
        }
    }
}

function Register-AsterTasks([string]$Root) {
    $launcher = Join-Path $Root 'config\services\service-launch.ps1'
    if (-not (Test-Path -LiteralPath $launcher -PathType Leaf)) { Fail 'Restored service launcher is missing.' }
    Ensure-AsterTaskFolder
    foreach ($entry in $tasks.GetEnumerator()) {
        # Match installation: pass the executable and arguments separately to
        # Task Scheduler, without schtasks /TR's extra Windows quoting layer.
        $taskName = $entry.Key.Substring($entry.Key.LastIndexOf('\') + 1)
        $action = New-ScheduledTaskAction `
            -Execute (Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe') `
            -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$launcher`" -Service $($entry.Value)"
        $trigger = if ($entry.Value -eq 'maintenance') {
            New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Minutes 1)
        } else { New-ScheduledTaskTrigger -AtStartup }
        $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
        Register-ScheduledTask -TaskName $taskName -TaskPath $script:AsterTaskPath -Action $action -Trigger $trigger -Principal $principal -Force | Out-Null
        Set-AsterTaskRuntimePolicy $entry.Key
        Disable-ScheduledTask -TaskName $taskName -TaskPath $script:AsterTaskPath | Out-Null
    }
    if ($runnerOnly) {
        if (Test-Path -LiteralPath (Join-Path $Root 'config\runner\identity.json') -PathType Leaf) {
            Invoke-Checked 'schtasks.exe' @('/Change', '/TN', ($script:AsterTaskPath + 'Runner'), '/ENABLE') 'enable restored Runner'
        }
        return
    }
    $active = Get-Content -LiteralPath (Join-Path $Root 'state\slots\active.json') -Raw | ConvertFrom-Json
    if ($active.schema -ne 'aster.active-release-slot.v1' -or $active.slot -notin @('blue', 'green')) {
        Fail 'Restored active slot metadata is invalid.'
    }
    $controlTask = if ($active.slot -eq 'blue') { ($script:AsterTaskPath + 'Control Blue') } else { ($script:AsterTaskPath + 'Control Green') }
    foreach ($task in @($controlTask, ($script:AsterTaskPath + 'Caddy'), ($script:AsterTaskPath + 'Maintenance'))) {
        Invoke-Checked 'schtasks.exe' @('/Change', '/TN', $task, '/ENABLE') "enable scheduled task $task"
    }
    if (Test-Path -LiteralPath (Join-Path $Root 'config\runner\identity.json') -PathType Leaf) {
        Invoke-Checked 'schtasks.exe' @('/Change', '/TN', ($script:AsterTaskPath + 'Runner'), '/ENABLE') 'enable local Runner task'
    }
    return $controlTask
}

function Start-RestoredServices([string]$Root) {
    $controlTask = Register-AsterTasks $Root
    if (-not $runnerOnly) {
        Invoke-Checked 'schtasks.exe' @('/Run', '/TN', $controlTask) 'start restored Control'
        Invoke-Checked 'schtasks.exe' @('/Run', '/TN', ($script:AsterTaskPath + 'Caddy')) 'start restored Caddy'
    }
    if (Test-Path -LiteralPath (Join-Path $Root 'config\runner\identity.json') -PathType Leaf) {
        Invoke-Checked 'schtasks.exe' @('/Run', '/TN', ($script:AsterTaskPath + 'Runner')) 'start restored Runner'
    }
    # An unenrolled dedicated Runner is intentionally disabled and has no Control.
    if ($runnerOnly -and -not (Test-Path -LiteralPath (Join-Path $Root 'config\runner\identity.json') -PathType Leaf)) { return }
    $cli = Join-Path $Root 'bin\aster-team-cli.exe'
    # Preserve a one-element array across the if pipeline. A scalar string is
    # splatted into individual characters by Windows PowerShell 5.1.
    [string[]]$statusArguments = if ($runnerOnly) { @('runner', 'status') } else { @('status') }
    $deadline = [DateTime]::UtcNow.AddSeconds(90)
    while ([DateTime]::UtcNow -lt $deadline) {
        & $cli @statusArguments | Out-Host
        if ($LASTEXITCODE -eq 0) { return }
        Start-Sleep -Seconds 1
    }
    Fail 'Restored installation readiness timed out. Check service logs and public endpoints.'
}

function Restart-AsterAfterFailure($OriginalFailure) {
    if ((Test-Path -LiteralPath $rollbackRoot) -and @(Get-ChildItem -LiteralPath $rollbackRoot -Force).Count -gt 0) {
        Write-Warning 'Rollback is incomplete. Services remain stopped; preserve the rollback directory for recovery.'
        throw $OriginalFailure
    }
    try { Start-RestoredServices $installRoot }
    catch { Write-Warning "Recovery restart also failed: $($_.Exception.Message)" }
    throw $OriginalFailure
}

try {
    $trustedCli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    $prepareArguments = @('prepare-windows-restore', '--source', $backup, '--destination', $preparedRoot, '--install-root', $installRoot)
    if ($runnerOnly) { $prepareArguments += '--runner-only' }
    $planJson = & $trustedCli @prepareArguments
    if ($LASTEXITCODE -ne 0) { Fail 'Backup preflight failed; live data has not been changed.' }
    $plan = ($planJson -join "`n") | ConvertFrom-Json
    $prepared = Join-Path $preparedRoot $rootName
    try {
        Stop-AsterTasks
        Invoke-Checked 'tar.exe' @('-C', $parent, '-czf', $safetyBackup, "--exclude=$rootName/backups/*", "--exclude=$rootName/staging/*", $rootName) 'pre-restore safety backup'
        Move-ManagedEntries $installRoot $rollbackRoot
    } catch {
        Restart-AsterAfterFailure $_
    }
    try {
        Move-ManagedEntries $prepared $installRoot
    } catch {
        Move-ManagedEntries $rollbackRoot $installRoot
        Restart-AsterAfterFailure $_
    }
    try {
        Restore-AsterReferences $installRoot $plan.references
        Start-RestoredServices $installRoot
    } catch {
        Stop-AsterTasks
        Move-ManagedEntries $installRoot $prepared
        Move-ManagedEntries $rollbackRoot $installRoot
        Restart-AsterAfterFailure $_
    }
    $restoreCompleted = $true
    Write-Output "Backup restored to $installRoot."
} finally {
    $rollbackPending = (Test-Path -LiteralPath $rollbackRoot) -and (@(Get-ChildItem -LiteralPath $rollbackRoot -Force -ErrorAction SilentlyContinue).Count -gt 0)
    if ($restoreCompleted) {
        try {
            Start-DeferredWorkspaceCleanup $workspaceRoot
        } catch {
            Write-Warning "Restore succeeded, but deferred cleanup could not be started. Remove $workspaceRoot after this command exits. $($_.Exception.Message)"
        }
    } elseif (-not $rollbackPending) {
        if (Test-Path -LiteralPath $workspaceRoot) {
            try { Remove-AsterRestoreTree $workspaceRoot }
            catch { Write-Warning "Restore failed; cleanup files remain at $workspaceRoot. $($_.Exception.Message)" }
        }
    } else {
        Write-Warning "Restore rollback files were preserved inside the installation root at $rollbackRoot."
    }
}
