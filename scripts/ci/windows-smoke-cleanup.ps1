# Shared by both Windows release smoke jobs. Never stop processes by image name.
function Get-AsterSmokeTaskPath([string]$ServicePrefix) {
    if ($ServicePrefix -and $ServicePrefix -cnotmatch '^[a-z][a-z0-9-]{0,31}$') { throw 'Invalid smoke service prefix.' }
    if ($ServicePrefix) { return "\Aster Team\$ServicePrefix\" }
    return '\Aster Team\'
}

function Set-AsterSmokeEnvironment([string]$ServicePrefix, [int]$PortOffset) {
    $names = @('ASTER_SERVICE_PREFIX', 'ASTER_PORT_OFFSET', 'ASTER_API_PORT', 'ASTER_MEMBER_PORT', 'ASTER_ADMIN_PORT',
        'ASTER_BLUE_API_PORT', 'ASTER_BLUE_MEMBER_PORT', 'ASTER_BLUE_ADMIN_PORT', 'ASTER_GREEN_API_PORT', 'ASTER_GREEN_MEMBER_PORT', 'ASTER_GREEN_ADMIN_PORT',
        'ASTER_CADDY_ADMIN_PORT', 'ASTER_DOMAIN_HTTP_PORT', 'ASTER_DOMAIN_HTTPS_PORT')
    $previous = @{}
    foreach ($name in $names) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
        # PowerShell can bind $null to an empty string; modern .NET preserves
        # that as an existing environment variable. Native null removes it.
        [Environment]::SetEnvironmentVariable($name, [NullString]::Value, 'Process')
    }
    [Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', $ServicePrefix, 'Process')
    [Environment]::SetEnvironmentVariable('ASTER_PORT_OFFSET', [string]$PortOffset, 'Process')
    return $previous
}

function Restore-AsterSmokeEnvironment($Previous) {
    foreach ($name in $Previous.Keys) {
        if ($null -eq $Previous[$name]) {
            [Environment]::SetEnvironmentVariable($name, [NullString]::Value, 'Process')
        } else {
            [Environment]::SetEnvironmentVariable($name, $Previous[$name], 'Process')
        }
    }
}

function Assert-AsterSmokeIsolation([string]$DefaultInstallRoot, [string]$ServicePrefix = '', [int]$PortOffset = 0) {
    # Query failures must abort, not be mistaken for an empty host. The Runner
    # scenario needs the same guard because cleanup uses the same task names.
    $script:AsterSmokeTaskPath = Get-AsterSmokeTaskPath $ServicePrefix
    if ($PortOffset -lt 0 -or $PortOffset -gt 54053) { throw 'Smoke port offset is out of range.' }
    $existing = @(Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -eq $script:AsterSmokeTaskPath })
    if ($existing.Count -gt 0) {
        throw 'Windows lifecycle smoke requires an unused Aster Team task namespace.'
    }
    if ([string]::IsNullOrWhiteSpace($DefaultInstallRoot) -or (-not $ServicePrefix -and (Test-Path -LiteralPath $DefaultInstallRoot))) {
        throw 'Windows lifecycle smoke refuses a host with an existing default Aster Team installation.'
    }
    $ports = @(2019, 11080, 11081, 11082, 11380, 11381, 11382, 11480, 11481, 11482) | ForEach-Object { $_ + $PortOffset }
    $listeners = @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { $_.State -eq 'Listen' -and $_.LocalPort -in $ports })
    if ($listeners.Count -gt 0) {
        throw "Windows lifecycle smoke ports are occupied: $(($listeners.LocalPort | Sort-Object -Unique) -join ', '). Choose an unused port offset."
    }
}

function Assert-AsterSmokeHost([string]$ServicePrefix = '', [int]$PortOffset = 0) {
    if ($PSVersionTable.PSVersion.Major -lt 7) {
        throw 'Windows lifecycle smoke requires PowerShell 7 (pwsh); product scripts still run in Windows PowerShell 5.1.'
    }
    $principal = [Security.Principal.WindowsPrincipal]::new([Security.Principal.WindowsIdentity]::GetCurrent())
    if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
        throw 'Windows lifecycle smoke must run from an Administrator PowerShell.'
    }
    $layout = Get-Content -LiteralPath (Join-Path $PSScriptRoot '..\..\contracts\install-layout.json') -Raw | ConvertFrom-Json
    Assert-AsterSmokeIsolation ([string]$layout.default_roots.windows) $ServicePrefix $PortOffset
}

function Remove-AsterTasks {
    $services = [ordered]@{ 'Control Blue' = 'control-blue'; 'Control Green' = 'control-green'; Runner = 'runner'; Caddy = 'caddy'; Maintenance = 'maintenance' }
    $launcher = Join-Path $installRoot 'config\services\service-launch.ps1'
    $failures = [Collections.Generic.List[string]]::new()
    foreach ($entry in $services.GetEnumerator()) {
        try {
            $task = Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -eq $script:AsterSmokeTaskPath -and $_.TaskName -eq $entry.Key }
            if ($task) {
                $actions = @($task.Actions)
                if ($actions.Count -ne 1 -or ([string]$actions[0].Arguments).IndexOf(('"' + $launcher + '"'), [StringComparison]::OrdinalIgnoreCase) -lt 0) {
                    throw "Refused to stop or delete a task outside this smoke installation: $($entry.Key)"
                }
            }
            if (Test-Path -LiteralPath $launcher -PathType Leaf) {
                & powershell.exe -NoProfile -NonInteractive -ExecutionPolicy Bypass -File $launcher -Service $entry.Value -Stop | Out-Host
                if ($LASTEXITCODE -ne 0) { throw "Service tree stop failed: $($entry.Value)" }
            }
            if ($task) {
                $task | Stop-ScheduledTask -ErrorAction Stop
                $task | Unregister-ScheduledTask -Confirm:$false -ErrorAction Stop
            }
        } catch { $failures.Add($_.Exception.Message) }
    }
    if ($failures.Count) { throw ($failures -join '; ') }
}

function Remove-AsterSmokeTree([string]$Path) {
    $item = Get-Item -LiteralPath $Path -Force -ErrorAction Stop
    if ($item.Attributes -band [IO.FileAttributes]::ReparsePoint) {
        if ($item.PSIsContainer) { [IO.Directory]::Delete($item.FullName) }
        else { [IO.File]::Delete($item.FullName) }
    } elseif ($item.PSIsContainer) {
        foreach ($child in Get-ChildItem -LiteralPath $Path -Force) { Remove-AsterSmokeTree $child.FullName }
        [IO.Directory]::Delete($item.FullName)
    } else {
        Remove-Item -LiteralPath $item.FullName -Force
    }
}

function Complete-AsterSmokeCleanup {
    # The caller creates this fresh root only after rejecting an existing path.
    $expectedInstall = [IO.Path]::GetFullPath((Join-Path $work 'install'))
    if (-not [string]::Equals($expectedInstall, [IO.Path]::GetFullPath($installRoot), [StringComparison]::OrdinalIgnoreCase) -or
        [string]::Equals($work.TrimEnd('\'), [IO.Path]::GetPathRoot($work).TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Refused unsafe Windows smoke cleanup root.'
    }
    Remove-AsterTasks
    # Job termination and deferred restore cleanup can briefly retain handles.
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (Test-Path -LiteralPath $work) {
        try { Remove-AsterSmokeTree $work; return }
        catch {
            if ([DateTime]::UtcNow -ge $deadline) { throw }
            Start-Sleep -Milliseconds 250
        }
    }
}
