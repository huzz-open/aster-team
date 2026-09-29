---
title: "asterctl overview and installation"
description: "Install the member client configuration tool, connect Codex or Claude, and find all commands and troubleshooting guidance."
---

# asterctl overview and installation

`asterctl` configures Codex Desktop or Claude Code on a member’s computer. Prepare the deployment’s **API URL** and an **Aster member API key**; the API URL may differ from the member portal URL. For server installation and operations, use [aster-team-cli](/en/tools/aster-team-cli/).

## Choose your next step

- [Quick setup](/en/tools/asterctl/quickstart): configure Codex or Claude for the first time.
- [Complete command reference](/en/tools/asterctl/commands): find subcommands, parameters, and examples by task.
- [Troubleshooting](/en/tools/asterctl/troubleshooting): diagnose configuration, authentication, catalog, and version issues.

## Get and run the tool

Obtain `asterctl.exe` for your deployment version from its administrator. The current distribution provides a Windows x64 artifact; automatic Codex setup currently supports Windows only. Install Codex or Claude Code before configuring it with this tool.

Open PowerShell in the directory containing `asterctl.exe` and check the version and command help:

```powershell
.\asterctl.exe version
.\asterctl.exe --help
```

### Add it to your user PATH

To run the tool from any directory, execute this script in the download directory. It copies the executable into your user directory and updates both the current shell and your persistent user PATH.

```powershell
$asterBin = Join-Path $env:USERPROFILE '.aster\bin'
New-Item -ItemType Directory -Path $asterBin -Force | Out-Null
$asterSource = (Resolve-Path -LiteralPath '.\asterctl.exe').Path
$asterTarget = Join-Path $asterBin 'asterctl.exe'
if ($asterSource -ne $asterTarget) {
    Copy-Item -LiteralPath $asterSource -Destination $asterTarget -Force
}
$asterUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($asterUserPath -split ';') -notcontains $asterBin) {
    [Environment]::SetEnvironmentVariable('Path', "$asterBin;$asterUserPath", 'User')
}
if (($env:Path -split ';') -notcontains $asterBin) {
    $env:Path = "$asterBin;$env:Path"
}
asterctl version
```

The examples below assume the tool is on PATH. Otherwise, run them from its directory and replace `asterctl` with `.\asterctl.exe`.

## Configuration scope

| Client | Scope | API URL format |
| --- | --- | --- |
| Codex Desktop | Current user, under CODEX_HOME or the user .codex directory | `https://aster.example.com/v1` |
| Claude Code | The selected project’s .claude/settings.local.json | `https://aster.example.com` |

Codex automatic setup, online diagnostics, and removal currently require Windows. Claude availability also depends on a shipped tool artifact and the installed client version; cross-platform source code does not imply a released platform. All model calls remain subject to installation capabilities and member permissions.
