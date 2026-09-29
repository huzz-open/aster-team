$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw $Message
}

$installRoot = ''
$runnerOnly = $false
for ($index = 0; $index -lt $args.Count; $index++) {
    switch ([string]$args[$index]) {
        '--install-root' {
            if ($index + 1 -ge $args.Count) { Fail '--install-root requires a value.' }
            $index++
            $installRoot = [string]$args[$index]
        }
        '--runner-only' { $runnerOnly = $true }
        '-h' {
            Write-Output 'Usage: .\init.ps1 [--install-root ABSOLUTE_PATH] [--runner-only]'
            exit 0
        }
        '--help' {
            Write-Output 'Usage: .\init.ps1 [--install-root ABSOLUTE_PATH] [--runner-only]'
            exit 0
        }
        default { Fail "Unknown argument: $($args[$index])" }
    }
}

$identity = [Security.Principal.WindowsIdentity]::GetCurrent()
$principal = New-Object Security.Principal.WindowsPrincipal($identity)
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    Fail 'Run init.ps1 from an elevated PowerShell terminal.'
}
if (-not [Environment]::Is64BitOperatingSystem -or -not [Environment]::Is64BitProcess) {
    Fail 'Aster Team for Windows requires 64-bit Windows and 64-bit PowerShell.'
}

$bundleRoot = [System.IO.Path]::GetFullPath((Join-Path $PSScriptRoot '.'))
$cli = Join-Path $bundleRoot 'bin\aster-team-cli.exe'
foreach ($relative in @('RELEASE.json', 'VERSION', 'bin\aster-team-cli.exe')) {
    $path = Join-Path $bundleRoot $relative
    if (-not (Test-Path -LiteralPath $path -PathType Leaf)) {
        Fail "Signed package is incomplete: $relative"
    }
}
$bootstrap = @('bootstrap', '--release-root', $bundleRoot)
if ($runnerOnly) { $bootstrap += '--runner-only' }
if (-not [string]::IsNullOrWhiteSpace($installRoot)) {
    if (-not [System.IO.Path]::IsPathRooted($installRoot)) {
        Fail '--install-root must be an absolute Windows path.'
    }
    $bootstrap += @('--install-root', [System.IO.Path]::GetFullPath($installRoot))
}
& $cli @bootstrap
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
