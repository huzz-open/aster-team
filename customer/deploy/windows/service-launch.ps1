[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)]
    [ValidateSet('control-blue', 'control-green', 'runner', 'caddy', 'maintenance')]
    [string]$Service,
    [switch]$Stop
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Resolve-AsterRoot {
    $candidate = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
    $markerPath = Join-Path $candidate 'install.json'
    if (-not (Test-Path -LiteralPath $markerPath -PathType Leaf)) {
        throw "Aster Team installation marker is missing: $markerPath"
    }
    $marker = Get-Content -LiteralPath $markerPath -Raw | ConvertFrom-Json
    if ($marker.schema -ne 'aster.installation-root.v1' -or $marker.platform -ne 'windows') {
        throw 'Aster Team installation marker is invalid for Windows.'
    }
    $recorded = [System.IO.Path]::GetFullPath([string]$marker.root)
    if (-not [string]::Equals($candidate.TrimEnd('\'), $recorded.TrimEnd('\'), [StringComparison]::OrdinalIgnoreCase)) {
        throw 'Aster Team installation marker does not match the launcher root.'
    }
    $script:AsterTaskPath = Get-AsterInstanceTaskPath $marker
    return $candidate.TrimEnd('\')
}

function Get-AsterInstanceTaskPath($Marker) {
    # Resolve only the task identity here. Install/restore validate the complete
    # configuration with the CLI. The launcher must also work while that CLI
    # executable is being replaced during an upgrade.
    if ($null -eq $Marker.PSObject.Properties['windows_instance']) { return '\Aster Team\' }
    $instance = $Marker.windows_instance
    if ($null -eq $instance -or $instance.schema -cne 'aster.windows-instance.v1' -or
        $instance.service_prefix -isnot [string]) { throw 'Invalid Windows instance identity.' }
    $prefix = $instance.service_prefix
    if ($prefix -and $prefix -cnotmatch '^[a-z][a-z0-9-]{0,31}$') { throw 'Invalid Windows service prefix.' }
    if ($prefix) { return "\Aster Team\$prefix\" }
    return '\Aster Team\'
}

function Import-AsterEnvironment {
    param(
        [Parameter(Mandatory = $true)][string]$Path,
        [Parameter(Mandatory = $true)][string[]]$Allowed,
        [switch]$Optional
    )
    if (-not (Test-Path -LiteralPath $Path -PathType Leaf)) {
        if ($Optional) { return @{} }
        throw "Required environment file is missing: $Path"
    }
    $values = @{}
    foreach ($rawLine in Get-Content -LiteralPath $Path) {
        $line = $rawLine.TrimEnd("`r")
        if ([string]::IsNullOrWhiteSpace($line) -or $line.StartsWith('#')) { continue }
        $parts = $line.Split('=', 2)
        if ($parts.Count -ne 2 -or -not ($Allowed -contains $parts[0]) -or [string]::IsNullOrWhiteSpace($parts[1])) {
            throw "Unsupported setting in $Path"
        }
        if ($values.ContainsKey($parts[0])) { throw "Duplicate setting in $Path" }
        $values[$parts[0]] = $parts[1]
        [Environment]::SetEnvironmentVariable($parts[0], $parts[1], 'Process')
    }
    return $values
}

function Resolve-AsterRelease {
    param([Parameter(Mandatory = $true)][string]$Reference)
    $referenceItem = Get-Item -LiteralPath $Reference
    $resolved = if ($referenceItem.LinkType -in @('Junction', 'SymbolicLink')) {
        $target = [string]@($referenceItem.Target)[0]
        if ([string]::IsNullOrWhiteSpace($target)) { throw "Release reference has no target: $Reference" }
        if (-not [System.IO.Path]::IsPathRooted($target)) { $target = Join-Path $referenceItem.Parent.FullName $target }
        [System.IO.Path]::GetFullPath($target)
    } else {
        [System.IO.Path]::GetFullPath($referenceItem.FullName)
    }
    $releaseRoot = [System.IO.Path]::GetFullPath((Join-Path $script:AsterRoot 'releases')).TrimEnd('\')
    $releasePrefix = $releaseRoot + '\'
    if (-not $resolved.StartsWith($releasePrefix, [StringComparison]::OrdinalIgnoreCase)) {
        throw "Release reference escapes the protected release directory: $Reference"
    }
    if (-not (Test-Path -LiteralPath (Join-Path $resolved 'RELEASE.json') -PathType Leaf)) {
        throw "Release reference is incomplete: $Reference"
    }
    return $resolved
}

function ConvertTo-AsterProcessArgument {
    param([AllowEmptyString()][string]$Value)
    # ProcessStartInfo.ArgumentList is unavailable in Windows PowerShell 5.1.
    # Quote using the Windows argv rules, including quotes and trailing slashes.
    return '"' + (($Value -replace '(\\*)"', '$1$1\"') -replace '(\\+)$', '$1$1') + '"'
}

function Initialize-AsterServiceJob {
    if ('AsterServiceJob' -as [type]) { return }
    # The launcher owns the only long-lived handle. Its children inherit the
    # job, not the handle: even TerminateProcess/Task Scheduler cannot orphan
    # them. Keep this self-contained because the installed launcher is copied.
    Add-Type -TypeDefinition @'
using System;
using System.ComponentModel;
using System.Runtime.InteropServices;
using System.Threading;
public static class AsterServiceJob {
    [StructLayout(LayoutKind.Sequential)] struct BasicLimits {
        public long ProcessTime, JobTime;
        public uint Flags;
        public UIntPtr MinimumWorkingSet, MaximumWorkingSet;
        public uint ActiveProcessLimit;
        public UIntPtr Affinity;
        public uint PriorityClass, SchedulingClass;
    }
    [StructLayout(LayoutKind.Sequential)] struct IoCounters {
        public ulong ReadOperations, WriteOperations, OtherOperations, ReadBytes, WriteBytes, OtherBytes;
    }
    [StructLayout(LayoutKind.Sequential)] struct ExtendedLimits {
        public BasicLimits Basic;
        public IoCounters Io;
        public UIntPtr ProcessMemory, JobMemory, PeakProcessMemory, PeakJobMemory;
    }
    [StructLayout(LayoutKind.Sequential)] struct Accounting {
        public long UserTime, KernelTime, PeriodUserTime, PeriodKernelTime;
        public uint PageFaults, TotalProcesses, ActiveProcesses, TerminatedProcesses;
    }
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr CreateJobObject(IntPtr security, string name);
    [DllImport("kernel32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
    static extern IntPtr OpenJobObject(uint access, bool inherit, string name);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool SetInformationJobObject(IntPtr job, int kind, ref ExtendedLimits limits, int size);
    [DllImport("kernel32.dll", SetLastError=true)]
    static extern bool QueryInformationJobObject(IntPtr job, int kind, out Accounting info, int size, IntPtr returned);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool AssignProcessToJobObject(IntPtr job, IntPtr process);
    [DllImport("kernel32.dll", SetLastError=true)] static extern bool TerminateJobObject(IntPtr job, uint code);
    [DllImport("kernel32.dll")] static extern IntPtr GetCurrentProcess();
    [DllImport("kernel32.dll")] static extern bool CloseHandle(IntPtr handle);
    static IntPtr ownedJob;
    public static void Enter(string name) {
        if (ownedJob != IntPtr.Zero) return;
        IntPtr job = CreateJobObject(IntPtr.Zero, name);
        int error = Marshal.GetLastWin32Error();
        if (job == IntPtr.Zero) throw new Win32Exception(error);
        try {
            if (error == 183) throw new InvalidOperationException("Service job already exists: " + name);
            var limits = new ExtendedLimits();
            limits.Basic.Flags = 0x2000; // JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
            if (!SetInformationJobObject(job, 9, ref limits, Marshal.SizeOf(limits))) throw new Win32Exception();
            if (!AssignProcessToJobObject(job, GetCurrentProcess())) throw new Win32Exception();
            ownedJob = job; // Do not close while the launcher itself is alive.
        } catch { CloseHandle(job); throw; }
    }
    public static void StopAndWait(string name) {
        IntPtr job = OpenJobObject(0x0004 | 0x0008, false, name); // QUERY | TERMINATE
        if (job == IntPtr.Zero) {
            int error = Marshal.GetLastWin32Error();
            if (error == 2) return; // A destroyed job has no live processes.
            throw new Win32Exception(error);
        }
        try {
            if (!TerminateJobObject(job, 1)) throw new Win32Exception();
            DateTime deadline = DateTime.UtcNow.AddSeconds(30);
            do {
                Accounting info;
                if (!QueryInformationJobObject(job, 1, out info, Marshal.SizeOf(typeof(Accounting)), IntPtr.Zero)) throw new Win32Exception();
                if (info.ActiveProcesses == 0) return;
                Thread.Sleep(50);
            } while (DateTime.UtcNow < deadline);
            throw new TimeoutException("Service processes did not stop: " + name);
        } finally { CloseHandle(job); }
    }
}
'@
}

function Get-AsterServiceJobName {
    $hash = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $hash.ComputeHash([Text.Encoding]::UTF8.GetBytes($script:AsterRoot.ToUpperInvariant()))
        return 'Global\AsterTeam-' + ([BitConverter]::ToString($digest).Replace('-', '')) + '-' + $Service
    } finally { $hash.Dispose() }
}

function Stop-AsterService {
    $names = @{ 'control-blue' = 'Control Blue'; 'control-green' = 'Control Green'; runner = 'Runner'; caddy = 'Caddy'; maintenance = 'Maintenance' }
    $task = Get-ScheduledTask -TaskName $names[$Service] -TaskPath $script:AsterTaskPath -ErrorAction SilentlyContinue
    if ($null -ne $task) {
        $actions = @($task.Actions)
        $launcher = '"' + (Join-Path $AsterRoot 'config\services\service-launch.ps1') + '"'
        if ($actions.Count -ne 1 -or ([string]$actions[0].Arguments).IndexOf($launcher, [StringComparison]::OrdinalIgnoreCase) -lt 0) {
            throw 'Refused to stop a scheduled task from another installation.'
        }
    }
    if ($null -ne $task -and $task.State.ToString() -eq 'Running') {
        $task | Stop-ScheduledTask -ErrorAction Stop
    }
    Initialize-AsterServiceJob
    [AsterServiceJob]::StopAndWait((Get-AsterServiceJobName))
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    do {
        $task = Get-ScheduledTask -TaskName $names[$Service] -TaskPath $script:AsterTaskPath -ErrorAction SilentlyContinue
        if ($null -eq $task -or $task.State.ToString() -ne 'Running') { return }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw "Scheduled task did not stop: $Service"
}

function Invoke-AsterProgram {
    param(
        [Parameter(Mandatory = $true)][string]$Program,
        [Parameter(Mandatory = $true)][AllowEmptyString()][string[]]$Arguments
    )
    if (-not (Test-Path -LiteralPath $Program -PathType Leaf)) {
        throw "Service executable is missing: $Program"
    }
    Initialize-AsterServiceJob
    [AsterServiceJob]::Enter((Get-AsterServiceJobName))
    $logDirectory = Join-Path $script:AsterRoot 'logs'
    if (-not (Test-Path -LiteralPath $logDirectory -PathType Container)) {
        New-Item -ItemType Directory -Path $logDirectory | Out-Null
    }
    $logPath = Join-Path $logDirectory "$Service.log"
    if ((Test-Path -LiteralPath $logPath -PathType Leaf) -and (Get-Item -LiteralPath $logPath).Length -gt 10485760) {
        Move-Item -LiteralPath $logPath -Destination "$logPath.1" -Force
    }
    $start = [Diagnostics.ProcessStartInfo]::new()
    $start.FileName = $Program
    $start.Arguments = ($Arguments | ForEach-Object { ConvertTo-AsterProcessArgument $_ }) -join ' '
    $start.WorkingDirectory = $script:AsterRoot
    $start.UseShellExecute = $false
    $start.CreateNoWindow = $true
    $start.RedirectStandardOutput = $true
    $start.RedirectStandardError = $true
    $start.StandardOutputEncoding = [Text.UTF8Encoding]::new($false)
    $start.StandardErrorEncoding = [Text.UTF8Encoding]::new($false)
    $process = [Diagnostics.Process]::new()
    $process.StartInfo = $start
    $started = $false
    try {
        $started = $process.Start()
        # Drain both pipes concurrently. stderr is a logging stream, not a
        # PowerShell ErrorRecord; normal Caddy logs must not stop the launcher.
        $readers = @($process.StandardOutput, $process.StandardError)
        $pending = @($readers[0].ReadLineAsync(), $readers[1].ReadLineAsync())
        while ($null -ne $pending[0] -or $null -ne $pending[1]) {
            $waiting = [Threading.Tasks.Task[]]@($pending | Where-Object { $null -ne $_ })
            [void][Threading.Tasks.Task]::WaitAny($waiting)
            for ($index = 0; $index -lt 2; $index++) {
                if ($null -eq $pending[$index] -or -not $pending[$index].IsCompleted) { continue }
                $text = $pending[$index].GetAwaiter().GetResult()
                if ($null -eq $text) {
                    $pending[$index] = $null
                    continue
                }
                Add-Content -LiteralPath $logPath -Value "[$([DateTime]::UtcNow.ToString('O'))] $text" -Encoding UTF8
                Write-Output $text
                $pending[$index] = $readers[$index].ReadLineAsync()
            }
        }
        $process.WaitForExit()
        if ($process.ExitCode -ne 0) {
            throw "Service process exited with status $($process.ExitCode)"
        }
    } finally {
        # A launcher/logging failure must not leave an unmonitored child alive.
        if ($started -and -not $process.HasExited) {
            $process.Kill()
            $process.WaitForExit()
        }
        $process.Dispose()
    }
}

try {
    $script:AsterRoot = Resolve-AsterRoot
    if ($Stop) {
        Stop-AsterService
        exit 0
    }
    $controlAllowed = @(
        'ASTER_CONTROL_LISTEN',
        'ASTER_CONTROL_ADMIN_LISTEN',
        'ASTER_CONTROL_MEMBER_LISTEN',
        'ASTER_CONTROL_ALLOW_INSECURE_HTTP',
        'ASTER_CONTROL_SECURE_COOKIES',
        'ASTER_CONTROL_API_TLS_CERTIFICATE',
        'ASTER_CONTROL_API_TLS_PRIVATE_KEY'
    )

    switch ($Service) {
        'control-blue' {
            Import-AsterEnvironment -Path (Join-Path $AsterRoot 'config\control\control.env') -Allowed $controlAllowed -Optional | Out-Null
            Import-AsterEnvironment -Path (Join-Path $AsterRoot 'config\control\control-blue.env') -Allowed $controlAllowed | Out-Null
            $release = Resolve-AsterRelease (Join-Path $AsterRoot 'state\slots\blue-release')
            Invoke-AsterProgram -Program (Join-Path $release 'bin\aster-control.exe') -Arguments @('verify-machine-identity')
            Invoke-AsterProgram -Program (Join-Path $release 'bin\aster-control.exe') -Arguments @(
                'serve', '--database-driver', 'sqlcipher',
                '--admin-assets', (Join-Path $release 'admin'),
                '--member-assets', (Join-Path $release 'member')
            )
        }
        'control-green' {
            Import-AsterEnvironment -Path (Join-Path $AsterRoot 'config\control\control.env') -Allowed $controlAllowed -Optional | Out-Null
            Import-AsterEnvironment -Path (Join-Path $AsterRoot 'config\control\control-green.env') -Allowed $controlAllowed | Out-Null
            $release = Resolve-AsterRelease (Join-Path $AsterRoot 'state\slots\green-release')
            Invoke-AsterProgram -Program (Join-Path $release 'bin\aster-control.exe') -Arguments @('verify-machine-identity')
            Invoke-AsterProgram -Program (Join-Path $release 'bin\aster-control.exe') -Arguments @(
                'serve', '--database-driver', 'sqlcipher',
                '--admin-assets', (Join-Path $release 'admin'),
                '--member-assets', (Join-Path $release 'member')
            )
        }
        'runner' {
            $runner = Import-AsterEnvironment -Path (Join-Path $AsterRoot 'config\runner\runner.env') -Allowed @(
                'ASTER_RUNNER_CONTROL_WSS',
                'ASTER_RUNNER_ALLOW_INSECURE_HTTP',
                'ASTER_RUNNER_CONTROL_CA_CERTIFICATE',
                'ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE'
            )
            $release = Resolve-AsterRelease (Join-Path $AsterRoot 'current')
            $arguments = @(
                'serve', '--control-wss', $runner.ASTER_RUNNER_CONTROL_WSS,
                '--allowed-upstream-host', 'api.openai.com',
                '--allowed-upstream-host', 'auth.openai.com',
                '--allowed-upstream-host', 'api.anthropic.com',
                '--allowed-upstream-host', 'chatgpt.com',
                '--allowed-upstream-host', 'api.deepseek.com',
                '--allowed-upstream-host', 'open.bigmodel.cn',
                '--allowed-upstream-host', 'api.z.ai'
            )
            Invoke-AsterProgram -Program (Join-Path $release 'bin\aster-runner.exe') -Arguments $arguments
        }
        'caddy' {
            $caddyData = Join-Path $AsterRoot 'data\caddy'
            [Environment]::SetEnvironmentVariable('HOME', $caddyData, 'Process')
            [Environment]::SetEnvironmentVariable('XDG_DATA_HOME', $caddyData, 'Process')
            [Environment]::SetEnvironmentVariable('XDG_CONFIG_HOME', (Join-Path $AsterRoot 'config\caddy'), 'Process')
            Invoke-AsterProgram -Program (Join-Path $AsterRoot 'bin\caddy.exe') -Arguments @(
                'run', '--config', (Join-Path $AsterRoot 'config\caddy\Caddyfile'), '--adapter', 'caddyfile'
            )
        }
        'maintenance' {
            Invoke-AsterProgram -Program (Join-Path $AsterRoot 'bin\aster-team-cli.exe') -Arguments @('maintenance', 'run-next')
        }
    }
} catch {
    $failure = "[$([DateTime]::UtcNow.ToString('O'))] $Service launcher failed ($($_.FullyQualifiedErrorId), line $($_.InvocationInfo.ScriptLineNumber)): $($_.Exception.Message)"
    [Console]::Error.WriteLine($failure)
    if (Get-Variable -Name AsterRoot -Scope Script -ErrorAction SilentlyContinue) {
        try {
            $logDirectory = Join-Path $script:AsterRoot 'logs'
            New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
            Add-Content -LiteralPath (Join-Path $logDirectory "$Service-launcher.log") -Value $failure -Encoding UTF8
        } catch {
            [Console]::Error.WriteLine("Could not persist launcher failure: $($_.Exception.Message)")
        }
    }
    exit 1
}
