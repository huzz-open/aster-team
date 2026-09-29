[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$ControlUrl,
    [Parameter(Mandatory = $true)][string]$Token,
    [switch]$AllowInsecureHttp,
    [string]$Version = '',
    [string]$InvocationRoot = '',
    [string]$InstallerUrl = ''
)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Set-StrictMode -Version Latest

function Fail([string]$Message) { throw "Aster Team Runner install: $Message" }

function ConvertTo-PowerShellLiteral([string]$Value) {
    return "'" + $Value.Replace("'", "''") + "'"
}

function Resolve-InvocationRoot([string]$Candidate) {
    if ([string]::IsNullOrWhiteSpace($Candidate)) {
        $location = Get-Location
        if ($location.Provider.Name -ne 'FileSystem') {
            Fail 'Run this command from a file system directory.'
        }
        $Candidate = $location.Path
    }
    if (-not [IO.Path]::IsPathRooted($Candidate)) {
        Fail "The command directory must be an absolute path: $Candidate"
    }
    $resolved = [IO.Path]::GetFullPath($Candidate)
    if (-not (Test-Path -LiteralPath $resolved -PathType Container)) {
        Fail "The command directory does not exist: $resolved"
    }
    return $resolved
}

function Test-RepairableRunnerInstall([string]$Path) {
    if (-not (Test-Path -LiteralPath $Path -PathType Container) -or
        (Test-Path -LiteralPath (Join-Path $Path 'current')) -or
        (Test-Path -LiteralPath (Join-Path $Path 'config\control\install-role')) -or
        (Test-Path -LiteralPath (Join-Path $Path 'config\runner\install-role'))) {
        return $false
    }
    $marker = Join-Path $Path 'install.json'
    if (-not (Test-Path -LiteralPath $marker -PathType Leaf)) { return $false }
    try {
        $document = Get-Content -LiteralPath $marker -Raw | ConvertFrom-Json
        if ($document.schema -ne 'aster.installation-root.v1' -or
            -not [string]::Equals(
                [IO.Path]::GetFullPath([string]$document.root).TrimEnd('\'),
                [IO.Path]::GetFullPath($Path).TrimEnd('\'),
                [StringComparison]::OrdinalIgnoreCase
            )) {
            return $false
        }
    } catch { return $false }
    $knownEntries = @('backups', 'bin', 'config', 'data', 'install.json', 'logs', 'releases', 'staging', 'state')
    return @(
        Get-ChildItem -LiteralPath $Path -Force |
            Where-Object { $_.Name -notin $knownEntries }
    ).Count -eq 0
}

function Test-CompletedRunnerInstall([string]$Path) {
    $role = Join-Path $Path 'config\runner\install-role'
    return (Test-Path -LiteralPath (Join-Path $Path 'current')) -and
        (Test-Path -LiteralPath $role -PathType Leaf) -and
        (Get-Content -LiteralPath $role -Raw).Trim() -eq 'runner'
}

function Test-ControlInstall([string]$Path) {
    $role = Join-Path $Path 'config\control\install-role'
    return (Test-Path -LiteralPath $role -PathType Leaf) -and
        (Get-Content -LiteralPath $role -Raw).Trim() -eq 'control'
}

function Resolve-RunnerInstallDirectory([string]$Candidate) {
    if (-not (Test-Path -LiteralPath $Candidate)) { return $Candidate }
    if (-not (Test-Path -LiteralPath $Candidate -PathType Container)) { return $null }
    if (@(Get-ChildItem -LiteralPath $Candidate -Force).Count -eq 0) { return $Candidate }
    if (Test-RepairableRunnerInstall $Candidate) {
        Write-Host "Continuing the incomplete Runner installation in: $Candidate"
        return $Candidate
    }
    return $null
}

function Remove-EmptyLegacyControlDirectories([string]$InstallRoot) {
    $relativePaths = @(
        'data\plugins\incoming', 'data\plugins\versions', 'data\plugins',
        'state\upgrades\queued', 'state\upgrades\running', 'state\upgrades\completed', 'state\upgrades',
        'config\control', 'config\caddy', 'config\keys', 'config\license', 'config\tls',
        'data\database', 'data\caddy', 'data\settlements', 'state\migrations', 'state\slots'
    )
    foreach ($relative in $relativePaths) {
        $path = Join-Path $InstallRoot $relative
        if ((Test-Path -LiteralPath $path -PathType Container) -and
            @(Get-ChildItem -LiteralPath $path -Force).Count -eq 0) {
            Remove-Item -LiteralPath $path -Force
        }
    }
}

function Get-HttpStatusCode($ErrorRecord) {
    try { return [int]$ErrorRecord.Exception.Response.StatusCode }
    catch { return 0 }
}

function Invoke-ExistingRunnerReenrollment(
    [string]$InstallRoot,
    [string]$Cli,
    [string]$TokenFile,
    [string]$TargetControlUrl,
    [bool]$PermitInsecureHttp
) {
    $paths = @(
        (Join-Path $InstallRoot 'config\runner\identity.json'),
        (Join-Path $InstallRoot 'config\runner\task-keys.json'),
        (Join-Path $InstallRoot 'config\runner\runner.env')
    )
    $backups = @{}
    $originallyPresent = @{}
    foreach ($path in $paths) {
        $originallyPresent[$path] = Test-Path -LiteralPath $path -PathType Leaf
    }
    $serviceStopped = $false
    try {
        & $Cli service stop runner
        if ($LASTEXITCODE -ne 0) { Fail "Could not stop the existing Runner service (status $LASTEXITCODE)." }
        $serviceStopped = $true
        foreach ($path in $paths) {
            if (Test-Path -LiteralPath $path -PathType Leaf) {
                $backup = "$path.reenroll-$([guid]::NewGuid().ToString('N')).bak"
                Move-Item -LiteralPath $path -Destination $backup
                $backups[$path] = $backup
            }
        }
        $arguments = @('runner', 'enroll', '--control-url', $TargetControlUrl, '--token-file', $TokenFile)
        if ($PermitInsecureHttp) { $arguments += '--allow-insecure-http' }
        & $Cli @arguments
        if ($LASTEXITCODE -ne 0) { Fail "Runner re-enrollment exited with status $LASTEXITCODE." }
        foreach ($backup in $backups.Values) {
            Remove-Item -LiteralPath $backup -Force
        }
        Write-Output 'Aster Team Runner is reconnected.'
    } catch {
        $failure = $_
        foreach ($path in $paths) {
            if ($backups.ContainsKey($path) -and (Test-Path -LiteralPath $backups[$path] -PathType Leaf)) {
                if (Test-Path -LiteralPath $path -PathType Leaf) {
                    Remove-Item -LiteralPath $path -Force
                }
                Move-Item -LiteralPath $backups[$path] -Destination $path
            } elseif (-not $originallyPresent[$path] -and (Test-Path -LiteralPath $path -PathType Leaf)) {
                Remove-Item -LiteralPath $path -Force
            }
        }
        if ($serviceStopped) { & $Cli service start runner | Out-Null }
        throw $failure
    }
}

function Select-RunnerInstallRoot([string]$DefaultParentRoot) {
    $defaultRoot = Join-Path $DefaultParentRoot 'Aster Team'
    Write-Host "Default Runner install directory: $defaultRoot"
    while ($true) {
        $selection = (Read-Host 'Press Enter to use the default, or enter another existing root directory').Trim()
        if ([string]::IsNullOrWhiteSpace($selection) -or $selection -match '^(?i:y|yes)$') {
            $candidate = $defaultRoot
        } else {
            $inputPath = $selection.Trim('"').Trim("'")
            if (-not [IO.Path]::IsPathRooted($inputPath)) {
                Write-Warning "The selected root directory must be an absolute path: $inputPath"
                continue
            }
            $parentRoot = [IO.Path]::GetFullPath($inputPath)
            if (-not (Test-Path -LiteralPath $parentRoot -PathType Container)) {
                Write-Warning "The selected root directory does not exist: $parentRoot"
                continue
            }
            $candidate = Join-Path $parentRoot 'Aster Team'
        }
        if (Test-CompletedRunnerInstall $candidate) {
            $script:ExistingRunnerInstall = $true
            Write-Host "Reconnecting the existing Runner installation in: $candidate"
            return $candidate
        }
        if (Test-ControlInstall $candidate) {
            Fail "Aster Team Control is already installed at: $candidate. Choose another root directory for this Runner."
        }
        $resolved = Resolve-RunnerInstallDirectory $candidate
        if ($null -eq $resolved) {
            Write-Warning "The Runner install directory is already occupied: $candidate"
            continue
        }
        Write-Host "Runner install directory: $candidate"
        return $resolved
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
    Fail 'Aster Team Runner requires 64-bit Windows and 64-bit PowerShell.'
}

$controlUri = $null
if (-not [Uri]::TryCreate($ControlUrl, [UriKind]::Absolute, [ref]$controlUri) -or
    ($controlUri.Scheme -ne 'https' -and -not ($AllowInsecureHttp -and $controlUri.Scheme -eq 'http'))) {
    Fail 'Control URL must use HTTPS, or HTTP with -AllowInsecureHttp.'
}
if ([string]::IsNullOrWhiteSpace($Token) -or $Token -match '\s') { Fail 'Enrollment token is invalid.' }
if ($Version -and $Version -notmatch '^v\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$') { Fail 'Version must use vX.Y.Z format.' }
$invocationRoot = Resolve-InvocationRoot $InvocationRoot
$installerUri = $null
if ([string]::IsNullOrWhiteSpace($InstallerUrl)) {
    $installerUri = [Uri]::new($controlUri, '/install-runner.ps1')
} elseif (-not [Uri]::TryCreate($InstallerUrl, [UriKind]::Absolute, [ref]$installerUri) -or
    ($installerUri.Scheme -ne 'https' -and -not ($AllowInsecureHttp -and $installerUri.Scheme -eq 'http'))) {
    Fail 'Installer URL must use HTTPS, or HTTP with -AllowInsecureHttp.'
}

if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    $installerUriText = $installerUri.AbsoluteUri
    $elevatedCommand = "`$installerSource = try { Invoke-RestMethod -UseBasicParsing -Uri $(ConvertTo-PowerShellLiteral $installerUriText) } catch { throw $(ConvertTo-PowerShellLiteral "Could not download the Runner installer from: $installerUriText") }; & ([scriptblock]::Create([string]`$installerSource)) -InstallerUrl $(ConvertTo-PowerShellLiteral $installerUriText) -ControlUrl $(ConvertTo-PowerShellLiteral $ControlUrl) -Token $(ConvertTo-PowerShellLiteral $Token) -InvocationRoot $(ConvertTo-PowerShellLiteral $invocationRoot)"
    if ($AllowInsecureHttp) { $elevatedCommand += ' -AllowInsecureHttp' }
    $failureFile = Join-Path ([IO.Path]::GetTempPath()) ('aster-runner-install-error-' + [guid]::NewGuid().ToString('N') + '.txt')
    $elevatedCommand = @"
try {
    $elevatedCommand
} catch {
    `$message = `$_.Exception.Message
    [IO.File]::WriteAllText($(ConvertTo-PowerShellLiteral $failureFile), `$message, [Text.UTF8Encoding]::new(`$false))
    Write-Host ''
    Write-Host ('Runner installation failed: ' + `$message) -ForegroundColor Red
    [void](Read-Host 'Press Enter to close this window')
    exit 1
}
"@
    $encodedCommand = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($elevatedCommand))
    $powerShellExecutable = if ($PSVersionTable.PSEdition -eq 'Core') {
        Join-Path $PSHOME 'pwsh.exe'
    } else {
        Join-Path $PSHOME 'powershell.exe'
    }
    try {
        $elevated = Start-Process -FilePath $powerShellExecutable -Verb RunAs -ArgumentList @(
            '-NoProfile', '-ExecutionPolicy', 'Bypass', '-EncodedCommand', $encodedCommand
        ) -Wait -PassThru
    } catch {
        Write-Host 'Administrator approval was canceled or could not be started.' -ForegroundColor Red
        return
    }
    if ($elevated.ExitCode -ne 0 -and -not (Test-Path -LiteralPath $failureFile -PathType Leaf)) {
        Write-Host "Runner installation failed: Elevated installer exited with status $($elevated.ExitCode)." -ForegroundColor Red
    }
    Remove-Item -LiteralPath $failureFile -Force -ErrorAction SilentlyContinue
    return
}

$script:ExistingRunnerInstall = $false
$installRoot = Select-RunnerInstallRoot $invocationRoot
if (-not $script:ExistingRunnerInstall -and (Test-Path -LiteralPath (Join-Path $installRoot 'current'))) {
    Fail 'An Aster Team installation already exists on this host.'
}
if (Test-RepairableRunnerInstall $installRoot) {
    Remove-EmptyLegacyControlDirectories $installRoot
}

$logDirectory = Join-Path $installRoot 'logs'
New-Item -ItemType Directory -Path $logDirectory -Force | Out-Null
$installLog = Join-Path $logDirectory 'install-runner.log'
$transcriptStarted = $false
try {
    Start-Transcript -LiteralPath $installLog -Append | Out-Null
    $transcriptStarted = $true
} catch {
    Fail "Could not open the install log: $installLog"
}
Write-Host "Install log: $installLog"

$temporaryDirectory = Join-Path ([IO.Path]::GetTempPath()) ('aster-team-runner-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $temporaryDirectory | Out-Null
try {
    $packageUri = [Uri]::new($controlUri, '/api/runner/install-package/windows/amd64').AbsoluteUri
    $checksumUri = "$packageUri/checksum"
    $downloadHeaders = @{ Authorization = "Bearer $Token" }
    try {
        $checksumLine = ([string](Invoke-RestMethod -UseBasicParsing -Uri $checksumUri -Headers $downloadHeaders)).Trim()
    } catch {
        $statusCode = Get-HttpStatusCode $_
        if ($statusCode -eq 401) {
            Fail 'The Runner enrollment token is invalid, expired, or already used. Generate a new installation command and retry.'
        }
        if ($statusCode -eq 404) {
            Fail "The Runner installation endpoint was not found at: $checksumUri. Copy a newly generated command from the current Admin page and retry."
        }
        $status = if ($statusCode) { "HTTP $statusCode" } else { $_.Exception.Message }
        Fail "Could not request Runner package information from Control: $status"
    }
    $checksumMatch = [regex]::Match(
        $checksumLine,
        '^([a-f0-9]{64})  (aster-team-runner-(\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?)-windows-amd64\.tar\.gz)$'
    )
    if (-not $checksumMatch.Success) { Fail 'Control returned an invalid Runner package checksum.' }
    $archiveName = $checksumMatch.Groups[2].Value
    $releaseVersion = $checksumMatch.Groups[3].Value
    $archive = Join-Path $temporaryDirectory $archiveName

    $tokenFile = Join-Path $temporaryDirectory 'enrollment-token'
    [IO.File]::WriteAllText($tokenFile, $Token, [Text.UTF8Encoding]::new($false))
    & icacls.exe $tokenFile '/inheritance:r' '/grant:r' '*S-1-5-18:F' '*S-1-5-32-544:F' | Out-Null
    if ($LASTEXITCODE -ne 0) { Fail 'Could not protect the enrollment token.' }

    if ($script:ExistingRunnerInstall) {
        $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
        if (-not (Test-Path -LiteralPath $cli -PathType Leaf)) { Fail 'The installed Runner CLI is missing.' }
        Invoke-ExistingRunnerReenrollment $installRoot $cli $tokenFile $ControlUrl $AllowInsecureHttp.IsPresent
        return
    }

    Write-Output "Downloading Aster Team Runner $releaseVersion for Windows amd64..."
    try {
        Invoke-WebRequest -UseBasicParsing -Uri $packageUri -Headers $downloadHeaders -OutFile $archive
    } catch {
        $statusCode = Get-HttpStatusCode $_
        $status = if ($statusCode) { "HTTP $statusCode" } else { $_.Exception.Message }
        Fail "Could not download the Runner package from Control: $status"
    }
    $actualSha256 = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actualSha256 -ne $checksumMatch.Groups[1].Value) {
        Fail 'Downloaded package SHA-256 does not match Control.'
    }

    & tar.exe -xzf $archive -C $temporaryDirectory
    if ($LASTEXITCODE -ne 0) { Fail "Package extraction exited with status $LASTEXITCODE." }
    $releaseDirectory = Join-Path $temporaryDirectory "aster-team-runner-$releaseVersion-windows-amd64"
    $initializer = Join-Path $releaseDirectory 'init.ps1'
    if (-not (Test-Path -LiteralPath $initializer -PathType Leaf)) { Fail 'The release package is incomplete.' }
    & $initializer --install-root $installRoot --runner-only
    if ($LASTEXITCODE -ne 0) { Fail "Package initialization exited with status $LASTEXITCODE." }

    $cli = Join-Path $installRoot 'bin\aster-team-cli.exe'
    & $cli runner install
    if ($LASTEXITCODE -ne 0) { Fail "Runner installation exited with status $LASTEXITCODE." }

    $enrollmentArguments = @('runner', 'enroll', '--control-url', $ControlUrl, '--token-file', $tokenFile)
    if ($AllowInsecureHttp) { $enrollmentArguments += '--allow-insecure-http' }
    & $cli @enrollmentArguments
    if ($LASTEXITCODE -ne 0) { Fail "Runner enrollment exited with status $LASTEXITCODE." }
    Write-Output 'Aster Team Runner is installed and connected.'
} catch {
    throw "$($_.Exception.Message)`nInstall log: $installLog"
} finally {
    Remove-Item -LiteralPath $temporaryDirectory -Recurse -Force -ErrorAction SilentlyContinue
    if ($transcriptStarted) { Stop-Transcript | Out-Null }
}
