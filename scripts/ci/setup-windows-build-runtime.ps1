[CmdletBinding()]
param(
    [string]$DestinationRoot = ''
)

$ErrorActionPreference = 'Stop'
Set-StrictMode -Version Latest

function Fail([string]$Message) {
    throw $Message
}

function Get-Sha256([string]$Path) {
    $stream = [IO.File]::OpenRead($Path)
    $algorithm = [Security.Cryptography.SHA256]::Create()
    try {
        $bytes = $algorithm.ComputeHash($stream)
        return ([BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
    }
    finally {
        $algorithm.Dispose()
        $stream.Dispose()
    }
}

$repositoryRoot = [IO.Path]::GetFullPath((Join-Path $PSScriptRoot '..\..'))
$contractPath = Join-Path $repositoryRoot 'tools\windows-build-runtime.json'
$contract = Get-Content -LiteralPath $contractPath -Raw | ConvertFrom-Json
if ($contract.schema -ne 'aster.windows-build-runtime.v1') {
    Fail 'Windows build runtime contract schema is invalid.'
}
$runtime = $contract.strawberry_perl
if ($runtime.version -notmatch '^\d+\.\d+\.\d+\.\d+$' -or
    $runtime.url -notmatch '^https://github\.com/StrawberryPerl/Perl-Dist-Strawberry/releases/download/' -or
    $runtime.sha256 -notmatch '^[0-9a-f]{64}$') {
    Fail 'Pinned Strawberry Perl metadata is invalid.'
}

if ([string]::IsNullOrWhiteSpace($DestinationRoot)) {
    $DestinationRoot = if ([string]::IsNullOrWhiteSpace($env:RUNNER_TEMP)) {
        Join-Path $repositoryRoot 'target\toolchains'
    } else {
        Join-Path $env:RUNNER_TEMP 'aster-windows-build-runtime'
    }
}
$destination = [IO.Path]::GetFullPath($DestinationRoot)
New-Item -ItemType Directory -Force -Path $destination | Out-Null
$archiveName = [IO.Path]::GetFileName(([Uri]$runtime.url).AbsolutePath)
if ([string]::IsNullOrWhiteSpace($archiveName) -or -not $archiveName.EndsWith('.zip', [StringComparison]::OrdinalIgnoreCase)) {
    Fail 'Pinned Strawberry Perl archive name is invalid.'
}
$archive = Join-Path $destination $archiveName
$runtimeDirectory = Join-Path $destination "strawberry-perl-$($runtime.version)"
$perl = Join-Path $runtimeDirectory 'perl\bin\perl.exe'

if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
    $partial = Join-Path $destination ".$archiveName.$PID.partial"
    if (Test-Path -LiteralPath $partial) { Fail "Temporary download path already exists: $partial" }
    & curl.exe --fail --location --retry 4 --output $partial $runtime.url
    if ($LASTEXITCODE -ne 0) { Fail 'Pinned Strawberry Perl download failed.' }
    $partialHash = Get-Sha256 $partial
    if ($partialHash -ne $runtime.sha256) {
        Remove-Item -LiteralPath $partial -Force
        Fail "Pinned Strawberry Perl SHA-256 mismatch: $partialHash"
    }
    Move-Item -LiteralPath $partial -Destination $archive
}

$archiveHash = Get-Sha256 $archive
if ($archiveHash -ne $runtime.sha256) {
    Fail "Cached Strawberry Perl SHA-256 mismatch: $archiveHash"
}
if (-not (Test-Path -LiteralPath $perl -PathType Leaf)) {
    if (Test-Path -LiteralPath $runtimeDirectory) {
        Fail "Incomplete Strawberry Perl runtime already exists: $runtimeDirectory"
    }
    $staging = Join-Path $destination ".strawberry-perl-$($runtime.version)-$PID"
    if (Test-Path -LiteralPath $staging) { Fail "Temporary extraction path already exists: $staging" }
    Expand-Archive -LiteralPath $archive -DestinationPath $staging
    $stagedPerl = Join-Path $staging 'perl\bin\perl.exe'
    if (-not (Test-Path -LiteralPath $stagedPerl -PathType Leaf)) {
        Fail 'Extracted Strawberry Perl runtime is incomplete.'
    }
    Move-Item -LiteralPath $staging -Destination $runtimeDirectory
}

& $perl -MLocale::Maketext::Simple -e 1
if ($LASTEXITCODE -ne 0) { Fail 'Pinned Strawberry Perl is missing required core modules.' }
$perlBin = Split-Path -Parent $perl
if (-not [string]::IsNullOrWhiteSpace($env:GITHUB_PATH)) {
    [IO.File]::AppendAllText($env:GITHUB_PATH, "$perlBin`n", [Text.UTF8Encoding]::new($false))
}
Write-Output "Pinned Strawberry Perl $($runtime.version): $perlBin"
