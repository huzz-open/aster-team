$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw $Message
}

function Write-Utf8File([string]$Path, [string]$Value) {
    $parent = Split-Path -Parent $Path
    if (-not (Test-Path -LiteralPath $parent -PathType Container)) {
        New-Item -ItemType Directory -Path $parent | Out-Null
    }
    $encoding = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($Path, $Value, $encoding)
}

function ConvertFrom-WindowsVerbatimPath([string]$Path) {
    if ($Path.StartsWith('\\?\UNC\', [StringComparison]::OrdinalIgnoreCase)) {
        return '\\' + $Path.Substring(8)
    }
    if ($Path.StartsWith('\\?\', [StringComparison]::OrdinalIgnoreCase)) {
        return $Path.Substring(4)
    }
    return $Path
}

function Get-Sha256Hex([string]$Path) {
    $stream = [System.IO.File]::OpenRead($Path)
    try {
        $sha256 = [System.Security.Cryptography.SHA256]::Create()
        try {
            return ([System.BitConverter]::ToString($sha256.ComputeHash($stream))).Replace('-', '').ToLowerInvariant()
        } finally {
            $sha256.Dispose()
        }
    } finally {
        $stream.Dispose()
    }
}

function Invoke-Checked([string]$Program, [string[]]$Arguments, [string]$Label) {
    & $Program @Arguments
    if ($LASTEXITCODE -ne 0) { Fail "$Label exited with status $LASTEXITCODE" }
}

function Invoke-AsterCaddy([string]$Program, [string[]]$Arguments, [string]$Label, [string]$Root) {
    $environment = @{
        HOME = (Join-Path $Root 'data\caddy')
        XDG_DATA_HOME = (Join-Path $Root 'data\caddy')
        XDG_CONFIG_HOME = (Join-Path $Root 'config\caddy')
    }
    $previous = @{}
    foreach ($name in $environment.Keys) {
        $previous[$name] = [Environment]::GetEnvironmentVariable($name, 'Process')
    }
    try {
        foreach ($name in $environment.Keys) {
            [Environment]::SetEnvironmentVariable($name, $environment[$name], 'Process')
        }
        Invoke-Checked $Program $Arguments $Label
    } finally {
        foreach ($name in $previous.Keys) {
            [Environment]::SetEnvironmentVariable($name, $previous[$name], 'Process')
        }
    }
}

function Wait-AsterInstallationReady([string]$Cli, [int]$TimeoutSeconds = 90) {
    $deadline = [DateTime]::UtcNow.AddSeconds($TimeoutSeconds)
    do {
        # Reuse the CLI's active-slot, Caddy, TLS and three public-entry checks.
        # Do not merge stderr into PowerShell's error stream on PowerShell 5.1.
        & $Cli status
        if ($LASTEXITCODE -eq 0) { return }
        if ([DateTime]::UtcNow -ge $deadline) { break }
        Start-Sleep -Seconds 1
    } while ([DateTime]::UtcNow -lt $deadline)
    Fail 'Installation readiness timed out: Control, Caddy or a public entry is unhealthy. Check logs and run aster-team-cli doctor.'
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
    $launcher = [IO.Path]::GetFullPath((Join-Path $Root 'config\services\service-launch.ps1'))
    foreach ($task in @(Get-ScheduledTask -ErrorAction Stop | Where-Object { $_.TaskPath -eq $script:AsterTaskPath })) {
        $actions = @($task.Actions)
        $existingLauncher = if ($actions.Count -eq 1) {
            $match = [regex]::Match([string]$actions[0].Arguments, '(?i)(?:^|\s)-File\s+(?:"([^"]+)"|(\S+))')
            if ($match.Success) {
                $value = if ($match.Groups[1].Success) { $match.Groups[1].Value } else { $match.Groups[2].Value }
                try { [IO.Path]::GetFullPath((ConvertFrom-WindowsVerbatimPath $value)) } catch { $null }
            }
        }
        if ($null -eq $existingLauncher -or
            -not [string]::Equals($existingLauncher, $launcher, [StringComparison]::OrdinalIgnoreCase)) {
            $taskName = "$($task.TaskPath)$($task.TaskName)"
            $existingRoot = if ($null -ne $existingLauncher) {
                Split-Path -Parent (Split-Path -Parent (Split-Path -Parent $existingLauncher))
            } else {
                'unknown'
            }
            throw "Another Aster Team installation already owns Windows task $taskName. Existing install directory: $existingRoot. Requested Runner directory: $Root. Uninstall the existing Windows service or remove its stale task before retrying."
        }
    }
}

function Get-RunnerServicePrefix([string]$Root) {
    $normalized = [IO.Path]::GetFullPath($Root).TrimEnd('\').ToLowerInvariant()
    $sha256 = [Security.Cryptography.SHA256]::Create()
    try {
        $digest = $sha256.ComputeHash([Text.Encoding]::UTF8.GetBytes($normalized))
    } finally {
        $sha256.Dispose()
    }
    $suffix = ([BitConverter]::ToString($digest)).Replace('-', '').Substring(0, 12).ToLowerInvariant()
    return "runner-$suffix"
}

function Get-AsterTaskName([string]$FullName) {
    if (-not $FullName.StartsWith($script:AsterTaskPath, [StringComparison]::Ordinal)) {
        Fail "Scheduled task is outside the Aster Team folder: $FullName"
    }
    $taskName = $FullName.Substring($FullName.LastIndexOf('\') + 1)
    if ([string]::IsNullOrWhiteSpace($taskName)) { Fail "Scheduled task name is invalid: $FullName" }
    return $taskName
}

function New-AsterTaskSettings {
    return New-ScheduledTaskSettingsSet `
        -AllowStartIfOnBatteries `
        -DontStopIfGoingOnBatteries `
        -ExecutionTimeLimit ([TimeSpan]::Zero) `
        -MultipleInstances IgnoreNew `
        -RestartCount 999 `
        -RestartInterval (New-TimeSpan -Minutes 1) `
        -StartWhenAvailable
}

function Register-AsterTask([string]$FullName, [string]$Service, [string]$Schedule, [string]$Launcher) {
    $taskName = Get-AsterTaskName $FullName
    $powershell = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
    $action = New-ScheduledTaskAction `
        -Execute $powershell `
        -Argument "-NoProfile -NonInteractive -ExecutionPolicy Bypass -File `"$Launcher`" -Service $Service"
    $trigger = if ($Schedule -eq 'startup') {
        New-ScheduledTaskTrigger -AtStartup
    } else {
        New-ScheduledTaskTrigger -Once -At (Get-Date).AddMinutes(1) -RepetitionInterval (New-TimeSpan -Minutes 1)
    }
    $principal = New-ScheduledTaskPrincipal -UserId 'SYSTEM' -LogonType ServiceAccount -RunLevel Highest
    Register-ScheduledTask `
        -TaskName $taskName `
        -TaskPath $script:AsterTaskPath `
        -Action $action `
        -Trigger $trigger `
        -Principal $principal `
        -Settings (New-AsterTaskSettings) `
        -Force | Out-Null
}

function Set-AsterTaskEnabled([string]$FullName, [bool]$Enabled) {
    $taskName = Get-AsterTaskName $FullName
    if ($Enabled) {
        Enable-ScheduledTask -TaskName $taskName -TaskPath $script:AsterTaskPath | Out-Null
    } else {
        Disable-ScheduledTask -TaskName $taskName -TaskPath $script:AsterTaskPath | Out-Null
    }
}

function Start-AsterTask([string]$FullName) {
    Start-ScheduledTask -TaskName (Get-AsterTaskName $FullName) -TaskPath $script:AsterTaskPath
}

function Stop-AsterTask([string]$FullName) {
    Stop-ScheduledTask -TaskName (Get-AsterTaskName $FullName) -TaskPath $script:AsterTaskPath -ErrorAction SilentlyContinue
}

$scriptArguments = @($args)

function Read-OptionValue([string]$Name, [ref]$Index) {
    if ($Index.Value + 1 -ge $scriptArguments.Count) { Fail "$Name requires a value" }
    $Index.Value++
    return [string]$scriptArguments[$Index.Value]
}

$options = @{
    InstallRoot = ''
    ReleaseRoot = ''
    ManifestSha256 = ''
    OwnerEmail = ''
    OwnerPasswordFile = ''
    AccessProtocol = 'http'
    AccessHost = ''
    BindAddress = ''
    CertificateSource = ''
    TlsCertificate = ''
    TlsPrivateKey = ''
}
$skipOwner = $false
$installLocalRunner = $false
$recoverPreserved = $false
$runnerOnly = $false
for ($index = 0; $index -lt $scriptArguments.Count; $index++) {
    switch ([string]$scriptArguments[$index]) {
        '--install-root' { $options.InstallRoot = Read-OptionValue '--install-root' ([ref]$index) }
        '--release-root' { $options.ReleaseRoot = Read-OptionValue '--release-root' ([ref]$index) }
        '--release-manifest-sha256' { $options.ManifestSha256 = Read-OptionValue '--release-manifest-sha256' ([ref]$index) }
        '--owner-email' { $options.OwnerEmail = Read-OptionValue '--owner-email' ([ref]$index) }
        '--owner-password-file' { $options.OwnerPasswordFile = Read-OptionValue '--owner-password-file' ([ref]$index) }
        '--access-protocol' { $options.AccessProtocol = Read-OptionValue '--access-protocol' ([ref]$index) }
        '--access-host' { $options.AccessHost = Read-OptionValue '--access-host' ([ref]$index) }
        '--bind-address' { $options.BindAddress = Read-OptionValue '--bind-address' ([ref]$index) }
        '--certificate-source' { $options.CertificateSource = Read-OptionValue '--certificate-source' ([ref]$index) }
        '--tls-certificate' { $options.TlsCertificate = Read-OptionValue '--tls-certificate' ([ref]$index) }
        '--tls-private-key' { $options.TlsPrivateKey = Read-OptionValue '--tls-private-key' ([ref]$index) }
        '--skip-owner' { $skipOwner = $true }
        '--install-local-runner' { $installLocalRunner = $true }
        '--recover-preserved' { $recoverPreserved = $true }
        '--runner-only' { $runnerOnly = $true }
        default { Fail "Unknown internal installer option: $($scriptArguments[$index])" }
    }
}

if ($options.AccessProtocol -eq 'http') {
    if (-not [string]::IsNullOrWhiteSpace($options.CertificateSource) -and $options.CertificateSource -ne 'none') {
        Fail 'HTTP access does not accept a certificate source.'
    }
    $options.CertificateSource = 'none'
} elseif ($options.AccessProtocol -eq 'https') {
    if ($options.CertificateSource -notin @('caddy', 'provided')) {
        Fail 'HTTPS access requires certificate source caddy or provided.'
    }
} else {
    Fail '--access-protocol must be http or https.'
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run the Aster Team installer from an elevated terminal.'
}
if ([string]::IsNullOrWhiteSpace($options.InstallRoot) -or -not [System.IO.Path]::IsPathRooted($options.InstallRoot)) {
    Fail '--install-root must be an absolute Windows path.'
}
if ([string]::IsNullOrWhiteSpace($options.ReleaseRoot) -or -not [System.IO.Path]::IsPathRooted($options.ReleaseRoot)) {
    Fail '--release-root must be an absolute Windows path.'
}
$installRoot = [System.IO.Path]::GetFullPath($options.InstallRoot).TrimEnd('\')
$bundleRoot = [System.IO.Path]::GetFullPath((ConvertFrom-WindowsVerbatimPath $options.ReleaseRoot)).TrimEnd('\')
$versionPath = Join-Path $bundleRoot 'VERSION'
$manifestPath = Join-Path $bundleRoot 'RELEASE.json'
$packageCli = Join-Path $bundleRoot 'bin\aster-team-cli.exe'
$requiredReleaseFiles = @(
    $versionPath,
    $manifestPath,
    $packageCli,
    (Join-Path $bundleRoot 'bin\aster-runner.exe'),
    (Join-Path $bundleRoot 'windows\service-launch.ps1')
)
if (-not $runnerOnly) {
    $requiredReleaseFiles += @(
        (Join-Path $bundleRoot 'bin\aster-control.exe'),
        (Join-Path $bundleRoot 'bin\caddy.exe')
    )
}
foreach ($required in $requiredReleaseFiles) {
    if (-not (Test-Path -LiteralPath $required -PathType Leaf)) { Fail "Signed package is incomplete: $required" }
}
$version = (Get-Content -LiteralPath $versionPath -Raw).Trim()
if ($version -notmatch '^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { Fail 'VERSION is invalid.' }
$actualManifestSha256 = Get-Sha256Hex $manifestPath
$current = Join-Path $installRoot 'current'
$isUpgrade = Test-Path -LiteralPath $current
if ($isUpgrade) {
    if (-not $runnerOnly) { Fail 'Control upgrades must be submitted through the maintenance upgrade flow.' }
    $runnerRole = Join-Path $installRoot 'config\runner\install-role'
    if (-not (Test-Path -LiteralPath $runnerRole -PathType Leaf) -or (Get-Content -LiteralPath $runnerRole -Raw).Trim() -ne 'runner') {
        Fail 'The existing installation is not a dedicated Runner.'
    }
    $verifierCli = Join-Path $current 'bin\aster-team-cli.exe'
    if (-not (Test-Path -LiteralPath $verifierCli -PathType Leaf)) { Fail 'The trusted installed CLI is unavailable.' }
} else {
    if ($options.ManifestSha256 -notmatch '^[0-9a-f]{64}$' -or $actualManifestSha256 -ne $options.ManifestSha256) {
        Fail 'RELEASE.json does not match the separately supplied SHA-256.'
    }
    $verifierCli = $packageCli
}
$verifyBundleArguments = @('verify-release', '--root', $bundleRoot)
if ($runnerOnly) { $verifyBundleArguments += '--runner-only' }
Invoke-Checked $verifierCli $verifyBundleArguments 'release verification'
$bundledFreeLicense = Join-Path $bundleRoot 'licenses\free-license.json'
$useBundledFreeLicense = $false
if (-not $runnerOnly -and -not $isUpgrade -and -not $recoverPreserved -and (Test-Path -LiteralPath $bundledFreeLicense)) {
    $licenseInfo = Get-Item -LiteralPath $bundledFreeLicense -Force
    if ($licenseInfo.PSIsContainer -or ($licenseInfo.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0 -or $licenseInfo.Length -lt 1 -or $licenseInfo.Length -gt 65536) {
        Fail 'Bundled free license must be an ordinary file between 1 byte and 64 KiB.'
    }
    Invoke-Checked (Join-Path $bundleRoot 'bin\aster-control.exe') @('verify-bundled-free-license', '--source', $bundledFreeLicense, '--minimum-valid-for-seconds', '900') 'bundled free license preflight'
    $useBundledFreeLicense = $true
}

$previousServicePrefix = [Environment]::GetEnvironmentVariable('ASTER_SERVICE_PREFIX', 'Process')
try {
    if ($runnerOnly) {
        [Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', (Get-RunnerServicePrefix $installRoot), 'Process')
    }
    $instanceJson = & (Join-Path $installRoot 'bin\aster-team-cli.exe') windows-instance --initialize
} finally {
    [Environment]::SetEnvironmentVariable('ASTER_SERVICE_PREFIX', $previousServicePrefix, 'Process')
}
if ($LASTEXITCODE -ne 0) { Fail 'Windows instance configuration failed.' }
$script:AsterInstance = ($instanceJson -join "`n") | ConvertFrom-Json
$script:AsterTaskPath = [string]$script:AsterInstance.task_path
$ports = $script:AsterInstance.configuration.ports
Assert-AsterTaskOwnership $installRoot
if (-not $runnerOnly -and -not $isUpgrade -and -not $recoverPreserved) {
    $ip = $null
    $publicPorts = if ([Net.IPAddress]::TryParse($options.AccessHost, [ref]$ip)) { @($ports.api, $ports.member, $ports.admin) }
        elseif ($options.AccessProtocol -eq 'https') { @($ports.domain_https) } else { @($ports.domain_http) }
    $requiredPorts = @($ports.blue_api, $ports.blue_member, $ports.blue_admin, $ports.green_api, $ports.green_member, $ports.green_admin, $ports.caddy_admin) + $publicPorts
    $occupied = @(Get-NetTCPConnection -ErrorAction Stop | Where-Object { $_.State -eq 'Listen' -and $_.LocalPort -in $requiredPorts })
    if ($occupied.Count) { Fail "Instance ports are occupied: $(($occupied.LocalPort | Sort-Object -Unique) -join ', ')" }
}

if (-not $recoverPreserved -and -not $runnerOnly) {
    if ([string]::IsNullOrWhiteSpace($options.OwnerEmail) -and -not $skipOwner) { Fail '--owner-email is required.' }
    if (-not $skipOwner -and -not (Test-Path -LiteralPath $options.OwnerPasswordFile -PathType Leaf)) { Fail '--owner-password-file is required.' }
}

$paths = if ($runnerOnly) {
    @('bin', 'releases', 'config\cli', 'config\runner', 'config\services', 'data\runner', 'data\runtime', 'state\locks', 'staging\upgrades', 'backups\upgrades', 'logs')
} else {
    @(
        'bin', 'releases', 'config\cli', 'config\control', 'config\runner', 'config\caddy', 'config\services',
        'config\keys', 'config\license', 'config\tls', 'data\database', 'data\runner', 'data\caddy',
        'data\runtime', 'data\settlements', 'data\plugins\incoming', 'data\plugins\versions', 'state\plugins', 'state\migrations', 'state\upgrades\queued', 'state\upgrades\running',
        'state\upgrades\completed', 'state\slots', 'state\locks', 'staging\upgrades', 'backups\upgrades', 'logs'
    )
}
foreach ($relative in $paths) {
    $path = Join-Path $installRoot $relative
    if (-not (Test-Path -LiteralPath $path -PathType Container)) { New-Item -ItemType Directory -Path $path | Out-Null }
}
Invoke-Checked 'icacls.exe' @($installRoot, '/inheritance:r', '/grant:r', '*S-1-5-18:(OI)(CI)F', '*S-1-5-32-544:(OI)(CI)F') 'installation ACL'

$releaseDirectory = Join-Path (Join-Path $installRoot 'releases') $version
if (Test-Path -LiteralPath $releaseDirectory) {
    $existingManifest = Join-Path $releaseDirectory 'RELEASE.json'
    if (-not (Test-Path -LiteralPath $existingManifest -PathType Leaf) -or (Get-Sha256Hex $existingManifest) -ne $actualManifestSha256) {
        Fail "Release $version is already staged with different contents."
    }
    $verifyStagedArguments = @('verify-release', '--root', $releaseDirectory)
    if ($runnerOnly) { $verifyStagedArguments += '--runner-only' }
    Invoke-Checked $verifierCli $verifyStagedArguments 'staged release verification'
} else {
    $stage = Join-Path (Join-Path $installRoot 'releases') (".$version.$([guid]::NewGuid().ToString('N'))")
    New-Item -ItemType Directory -Path $stage | Out-Null
    try {
        if ($runnerOnly) {
            foreach ($relative in @(
                'RELEASE.json', 'VERSION', 'bin\aster-runner.exe', 'bin\aster-team-cli.exe',
                'init.ps1', 'libexec\install.ps1', 'libexec\restore-backup.ps1', 'windows\service-launch.ps1'
            )) {
                $source = Join-Path $bundleRoot $relative
                $destination = Join-Path $stage $relative
                $destinationParent = Split-Path -Parent $destination
                if (-not (Test-Path -LiteralPath $destinationParent -PathType Container)) {
                    New-Item -ItemType Directory -Path $destinationParent | Out-Null
                }
                Copy-Item -LiteralPath $source -Destination $destination -Force
            }
        } else {
            foreach ($item in Get-ChildItem -LiteralPath $bundleRoot -Force) {
                Copy-Item -LiteralPath $item.FullName -Destination $stage -Recurse -Force
            }
        }
        $verifyStageArguments = @('verify-release', '--root', $stage)
        if ($runnerOnly) { $verifyStageArguments += '--runner-only' }
        Invoke-Checked $verifierCli $verifyStageArguments 'staged release verification'
        Move-Item -LiteralPath $stage -Destination $releaseDirectory
    } catch {
        if ((Test-Path -LiteralPath $stage) -and $stage.StartsWith((Join-Path $installRoot 'releases'), [StringComparison]::OrdinalIgnoreCase)) {
            Remove-Item -LiteralPath $stage -Recurse -Force
        }
        throw
    }
}

if ($runnerOnly) {
    $launcher = Join-Path $installRoot 'config\services\service-launch.ps1'
    $identityPath = Join-Path $installRoot 'config\runner\identity.json'
    $oldLauncher = if ($isUpgrade -and (Test-Path -LiteralPath $launcher -PathType Leaf)) {
        [System.IO.File]::ReadAllBytes($launcher)
    } else {
        $null
    }
    $previousCurrent = $null
    $currentSwitched = $false
    if ($isUpgrade) {
        Stop-AsterTask ($script:AsterTaskPath + 'Runner')
        $deadline = [DateTime]::UtcNow.AddSeconds(30)
        do {
            $task = Get-ScheduledTask -TaskName 'Runner' -TaskPath $script:AsterTaskPath -ErrorAction SilentlyContinue
            if ($null -eq $task -or $task.State.ToString() -ne 'Running') { break }
            Start-Sleep -Milliseconds 250
        } while ([DateTime]::UtcNow -lt $deadline)
        if ($null -ne $task -and $task.State.ToString() -eq 'Running') { Fail 'Runner did not stop before upgrade.' }
        $temporaryCurrent = Join-Path $installRoot ".current.$([guid]::NewGuid().ToString('N'))"
        $previousCurrent = Join-Path $installRoot ".previous-current.$([guid]::NewGuid().ToString('N'))"
        New-Item -ItemType Junction -Path $temporaryCurrent -Target $releaseDirectory | Out-Null
        try {
            Move-Item -LiteralPath $current -Destination $previousCurrent
            try {
                Move-Item -LiteralPath $temporaryCurrent -Destination $current
                $currentSwitched = $true
            } catch {
                Move-Item -LiteralPath $previousCurrent -Destination $current
                throw
            }
        } finally {
            if (Test-Path -LiteralPath $temporaryCurrent) { Remove-Item -LiteralPath $temporaryCurrent -Force }
        }
    } else {
        New-Item -ItemType Junction -Path $current -Target $releaseDirectory | Out-Null
    }
    try {
        Copy-Item -LiteralPath (Join-Path $releaseDirectory 'windows\service-launch.ps1') -Destination $launcher -Force
        Write-Utf8File (Join-Path $installRoot 'config\runner\install-role') "runner`n"
        Ensure-AsterTaskFolder
        Register-AsterTask ($script:AsterTaskPath + 'Runner') 'runner' 'startup' $launcher
        if (Test-Path -LiteralPath $identityPath -PathType Leaf) {
            Set-AsterTaskEnabled ($script:AsterTaskPath + 'Runner') $true
            Start-AsterTask ($script:AsterTaskPath + 'Runner')
        } else {
            Set-AsterTaskEnabled ($script:AsterTaskPath + 'Runner') $false
        }
    } catch {
        $upgradeError = $_
        if ($isUpgrade) {
            Stop-AsterTask ($script:AsterTaskPath + 'Runner')
            if ($currentSwitched -and (Test-Path -LiteralPath $previousCurrent)) {
                Remove-Item -LiteralPath $current -Force
                Move-Item -LiteralPath $previousCurrent -Destination $current
            }
            if ($null -ne $oldLauncher) {
                [System.IO.File]::WriteAllBytes($launcher, $oldLauncher)
            }
            try {
                Register-AsterTask ($script:AsterTaskPath + 'Runner') 'runner' 'startup' $launcher
                if (Test-Path -LiteralPath $identityPath -PathType Leaf) {
                    Set-AsterTaskEnabled ($script:AsterTaskPath + 'Runner') $true
                    Start-AsterTask ($script:AsterTaskPath + 'Runner')
                }
            } catch {
                Write-Warning "Runner rollback needs administrator attention: $($_.Exception.Message)"
            }
        }
        throw $upgradeError
    }
    if ($isUpgrade -and $currentSwitched -and (Test-Path -LiteralPath $previousCurrent)) {
        Remove-Item -LiteralPath $previousCurrent -Force
    }
    if (Test-Path -LiteralPath $identityPath -PathType Leaf) {
        Write-Output "Aster Team Runner upgraded to $version under $installRoot."
    } else {
        Write-Output "Aster Team Runner $version installed under $installRoot; enroll it before starting."
    }
    exit 0
}
$control = Join-Path $releaseDirectory 'bin\aster-control.exe'
if (-not $recoverPreserved) {
    foreach ($provider in @('openai', 'deepseek', 'glm')) {
        $incomingPlugin = Join-Path $installRoot "data\plugins\incoming\$provider.asterlua"
        if (-not (Test-Path -LiteralPath $incomingPlugin)) {
            Copy-Item -LiteralPath (Join-Path $releaseDirectory "plugins\$provider.asterlua") `
                -Destination $incomingPlugin
        }
    }
}
if (-not $recoverPreserved) {
    Write-Utf8File (Join-Path $installRoot 'config\control\install-role') "control`n"
    $secureCookies = if ($options.AccessProtocol -eq 'https') { 'true' } else { 'false' }
    Write-Utf8File (Join-Path $installRoot 'config\control\control.env') "ASTER_CONTROL_ALLOW_INSECURE_HTTP=false`nASTER_CONTROL_SECURE_COOKIES=$secureCookies`n"
    Write-Utf8File (Join-Path $installRoot 'config\control\control-blue.env') "ASTER_CONTROL_LISTEN=127.0.0.1:$($ports.blue_api)`nASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:$($ports.blue_member)`nASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:$($ports.blue_admin)`n"
    Write-Utf8File (Join-Path $installRoot 'config\control\control-green.env') "ASTER_CONTROL_LISTEN=127.0.0.1:$($ports.green_api)`nASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:$($ports.green_member)`nASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:$($ports.green_admin)`n"

    $parsedAddress = $null
    $addressKind = if ([System.Net.IPAddress]::TryParse($options.AccessHost, [ref]$parsedAddress)) { 'ip' } else { 'domain' }
    if ($addressKind -eq 'domain') {
        $domainPort = if ($options.AccessProtocol -eq 'https') { [int]$ports.domain_https } else { [int]$ports.domain_http }
        $defaultPort = if ($options.AccessProtocol -eq 'https') { 443 } else { 80 }
        $domainSuffix = if ($domainPort -eq $defaultPort) { '' } else { ":$domainPort" }
        $memberUrl = "$($options.AccessProtocol)://app.$($options.AccessHost)$domainSuffix"
        $adminUrl = "$($options.AccessProtocol)://admin.$($options.AccessHost)$domainSuffix"
        $apiUrl = "$($options.AccessProtocol)://api.$($options.AccessHost)$domainSuffix"
    } else {
        $memberUrl = "$($options.AccessProtocol)://$($options.AccessHost):$($ports.member)"
        $adminUrl = "$($options.AccessProtocol)://$($options.AccessHost):$($ports.admin)"
        $apiUrl = "$($options.AccessProtocol)://$($options.AccessHost):$($ports.api)"
    }
    $runnerWss = ($apiUrl -replace '^http:', 'ws:' -replace '^https:', 'wss:') + '/api/runner/channel'
    $access = [ordered]@{
        schema = 'aster.team.access/v1'; protocol = $options.AccessProtocol; address_kind = $addressKind
        host = $options.AccessHost; bind_address = $options.BindAddress; certificate_source = $options.CertificateSource
        caddy_enabled = $true; member_url = $memberUrl; admin_url = $adminUrl; api_url = $apiUrl; runner_websocket_url = $runnerWss
    }
    Write-Utf8File (Join-Path $installRoot 'config\control\access.json') (($access | ConvertTo-Json) + "`n")
    Write-Utf8File (Join-Path $installRoot 'config\caddy\upstreams.caddy') "(aster_api_upstream) {`n`treverse_proxy 127.0.0.1:$($ports.blue_api) {`n`t`tstream_close_delay 15m`n`t}`n}`n`n(aster_member_upstream) {`n`treverse_proxy 127.0.0.1:$($ports.blue_member) {`n`t`tstream_close_delay 15m`n`t}`n}`n`n(aster_admin_upstream) {`n`treverse_proxy 127.0.0.1:$($ports.blue_admin) {`n`t`tstream_close_delay 15m`n`t}`n}`n"

    $caddyConfigRoot = (Join-Path $installRoot 'config\caddy').Replace('\', '/')
    $tlsDirective = ''
    if ($options.AccessProtocol -eq 'https' -and $options.CertificateSource -eq 'caddy') { $tlsDirective = 'tls internal' }
    elseif ($options.AccessProtocol -eq 'https') {
        Copy-Item -LiteralPath $options.TlsCertificate -Destination (Join-Path $installRoot 'config\tls\server.crt') -Force
        Copy-Item -LiteralPath $options.TlsPrivateKey -Destination (Join-Path $installRoot 'config\tls\server.key') -Force
        $certificate = (Join-Path $installRoot 'config\tls\server.crt').Replace('\', '/')
        $privateKey = (Join-Path $installRoot 'config\tls\server.key').Replace('\', '/')
        $tlsDirective = "tls `"$certificate`" `"$privateKey`""
    }
    $caddy = "{`n`tadmin 127.0.0.1:$($ports.caddy_admin)`n`tpersist_config off`n`tauto_https disable_redirects`n}`n`nimport `"$caddyConfigRoot/upstreams.caddy`"`n`n"
    if ($addressKind -eq 'domain') {
        foreach ($entry in @(@("api.$($options.AccessHost)$domainSuffix", 'aster_api_upstream'), @("app.$($options.AccessHost)$domainSuffix", 'aster_member_upstream'), @("admin.$($options.AccessHost)$domainSuffix", 'aster_admin_upstream'))) {
            $caddy += "$($options.AccessProtocol)://$($entry[0]) {`n`tbind $($options.BindAddress)`n"
            if ($tlsDirective) { $caddy += "`t$tlsDirective`n" }
            $caddy += "`timport $($entry[1])`n}`n"
            if ($entry[1] -ne 'aster_admin_upstream') { $caddy += "`n" }
        }
    } else {
        foreach ($entry in @(@($ports.api, 'aster_api_upstream'), @($ports.member, 'aster_member_upstream'), @($ports.admin, 'aster_admin_upstream'))) {
            $caddy += "$($options.AccessProtocol)://$($options.AccessHost):$($entry[0]) {`n`tbind $($options.BindAddress)`n"
            if ($tlsDirective) { $caddy += "`t$tlsDirective`n" }
            $caddy += "`timport $($entry[1])`n}`n"
            if ($entry[1] -ne 'aster_admin_upstream') { $caddy += "`n" }
        }
    }
    Write-Utf8File (Join-Path $installRoot 'config\caddy\Caddyfile') $caddy

    Invoke-Checked $control @('initialize-installation') 'installation identity initialization'
    if ($useBundledFreeLicense) {
        # Use the verified immutable release copy and the same v2 install/history path.
        # Do this before database and owner initialization; recovery never replaces a License.
        $stagedFreeLicense = Join-Path $releaseDirectory 'licenses\free-license.json'
        Invoke-Checked $control @('verify-bundled-free-license', '--source', $stagedFreeLicense, '--minimum-valid-for-seconds', '900') 'staged free license verification'
        Invoke-Checked $control @('install-license', '--source', $stagedFreeLicense) 'bundled free license installation'
    }

    Invoke-Checked $control @('initialize-runner-task-key') 'Runner task key initialization'
    Invoke-Checked $control @('export-runner-task-keys') 'Runner task key export'
    Copy-Item -LiteralPath (Join-Path $installRoot 'config\control\runner-task-keys.json') -Destination (Join-Path $installRoot 'config\runner\task-keys.json') -Force
    $databaseKeyPath = Join-Path $installRoot 'config\keys\database.key'
    if (-not (Test-Path -LiteralPath $databaseKeyPath)) {
        $bytes = New-Object byte[] 32
        $random = [System.Security.Cryptography.RandomNumberGenerator]::Create()
        try { $random.GetBytes($bytes) } finally { $random.Dispose() }
        [System.IO.File]::WriteAllBytes($databaseKeyPath, $bytes)
    }
    Invoke-Checked $control @('initialize-database', '--database-driver', 'sqlcipher') 'database initialization'
    Invoke-Checked $control @('initialize-runtime-configuration', '--database-driver', 'sqlcipher', '--public-api-base-url', $apiUrl) 'runtime configuration initialization'
    if (-not $skipOwner) {
        Invoke-Checked $control @('initialize-owner', '--database-driver', 'sqlcipher', '--email', $options.OwnerEmail, '--password-file', $options.OwnerPasswordFile) 'Owner initialization'
        $password = (Get-Content -LiteralPath $options.OwnerPasswordFile -Raw).Trim()
        Write-Utf8File (Join-Path $installRoot 'config\control\initial-owner-credentials') "ASTER_OWNER_EMAIL=$($options.OwnerEmail)`nASTER_OWNER_TEMPORARY_PASSWORD=$password`n"
    }
    if ($installLocalRunner) {
        $identityPath = Join-Path $installRoot 'config\runner\identity.json'
        Invoke-Checked $control @('initialize-local-runner', '--database-driver', 'sqlcipher', '--owner-email', $options.OwnerEmail, '--name', 'local-runner', '--identity-output', $identityPath) 'local Runner initialization'
        $runnerEnvironment = "ASTER_RUNNER_CONTROL_WSS=$runnerWss`n"
        if ($options.AccessProtocol -eq 'http') { $runnerEnvironment += "ASTER_RUNNER_ALLOW_INSECURE_HTTP=true`n" }
        Write-Utf8File (Join-Path $installRoot 'config\runner\runner.env') $runnerEnvironment
    }
    Write-Utf8File (Join-Path $installRoot 'state\initialization-complete') "aster.team.initialization-complete/v1`n"
} else {
    Invoke-Checked $control @('verify-machine-identity') 'preserved installation identity verification'
    Invoke-Checked $control @('preflight', '--database-driver', 'sqlcipher', '--admin-assets', (Join-Path $releaseDirectory 'admin'), '--member-assets', (Join-Path $releaseDirectory 'member')) 'preserved installation preflight'
}

if (-not $recoverPreserved) {
    Invoke-AsterCaddy (Join-Path $releaseDirectory 'bin\caddy.exe') @('fmt', '--overwrite', (Join-Path $installRoot 'config\caddy\Caddyfile')) 'Caddyfile formatting' $installRoot
    Invoke-AsterCaddy (Join-Path $releaseDirectory 'bin\caddy.exe') @('fmt', '--overwrite', (Join-Path $installRoot 'config\caddy\upstreams.caddy')) 'Caddy upstream formatting' $installRoot
}
Invoke-AsterCaddy (Join-Path $releaseDirectory 'bin\caddy.exe') @('validate', '--config', (Join-Path $installRoot 'config\caddy\Caddyfile'), '--adapter', 'caddyfile') 'Caddy validation' $installRoot
Copy-Item -LiteralPath (Join-Path $releaseDirectory 'bin\caddy.exe') -Destination (Join-Path $installRoot 'bin\caddy.exe') -Force
Copy-Item -LiteralPath (Join-Path $releaseDirectory 'windows\service-launch.ps1') -Destination (Join-Path $installRoot 'config\services\service-launch.ps1') -Force
New-Item -ItemType Junction -Path $current -Target $releaseDirectory | Out-Null
New-Item -ItemType Junction -Path (Join-Path $installRoot 'state\slots\blue-release') -Target $releaseDirectory | Out-Null
Write-Utf8File (Join-Path $installRoot 'state\slots\active.json') (([ordered]@{ schema = 'aster.active-release-slot.v1'; slot = 'blue'; version = $version } | ConvertTo-Json) + "`n")

$launcher = Join-Path $installRoot 'config\services\service-launch.ps1'
$tasks = [ordered]@{
    ($script:AsterTaskPath + 'Control Blue') = 'control-blue'; ($script:AsterTaskPath + 'Control Green') = 'control-green'
    ($script:AsterTaskPath + 'Runner') = 'runner'; ($script:AsterTaskPath + 'Caddy') = 'caddy'; ($script:AsterTaskPath + 'Maintenance') = 'maintenance'
}
Ensure-AsterTaskFolder
foreach ($entry in $tasks.GetEnumerator()) {
    $schedule = if ($entry.Value -eq 'maintenance') { 'minute' } else { 'startup' }
    Register-AsterTask $entry.Key $entry.Value $schedule $launcher
}
foreach ($task in $tasks.Keys) { Set-AsterTaskEnabled $task $false }
foreach ($task in @(($script:AsterTaskPath + 'Control Blue'), ($script:AsterTaskPath + 'Caddy'), ($script:AsterTaskPath + 'Maintenance'))) { Set-AsterTaskEnabled $task $true }
if ($installLocalRunner -or (Test-Path -LiteralPath (Join-Path $installRoot 'config\runner\identity.json'))) {
    Set-AsterTaskEnabled ($script:AsterTaskPath + 'Runner') $true
}
Start-AsterTask ($script:AsterTaskPath + 'Control Blue')
Start-AsterTask ($script:AsterTaskPath + 'Caddy')
if ($installLocalRunner -or (Test-Path -LiteralPath (Join-Path $installRoot 'config\runner\identity.json') -PathType Leaf)) {
    Start-AsterTask ($script:AsterTaskPath + 'Runner')
}

Wait-AsterInstallationReady (Join-Path $installRoot 'bin\aster-team-cli.exe')

Write-Output "Aster Team $version installed under $installRoot."
