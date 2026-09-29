---
title: "aster-team-cli runner install"
description: "Install a dedicated Runner host."
---

# aster-team-cli runner install

Install a dedicated Runner host.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Initialize a verified Runner release first and run on the dedicated host.

## Syntax

```text
aster-team-cli runner install
```

## Parameters

No command-specific parameters.

Use `aster-team-cli runner install --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli runner install
```

## Configuration and runtime effects

Install Runner binaries, configuration, and service. Use runner enroll to join Control; this does not install a Control host.

## Result

After installation, enroll using a registration token and inspect runner status.

## Troubleshooting

Stop on an unselected release, unsupported platform, or role conflict; check the release and host.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
