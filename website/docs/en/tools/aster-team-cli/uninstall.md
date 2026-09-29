---
title: "aster-team-cli uninstall"
description: "Uninstall services and binaries, preserving business data by default."
---

# aster-team-cli uninstall

Uninstall services and binaries, preserving business data by default.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli uninstall [--purge]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--purge` | No | false | Delete local configuration and data; requires typing PURGE in an interactive terminal. |

Use `aster-team-cli uninstall --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli uninstall
```

## Configuration and runtime effects

Normal uninstall removes services and binaries but preserves configuration, identity, keys, and data for recovery. --purge deletes installation-root data; manage external database data separately. Windows cleans up occupied program files after the CLI exits.

## Result

Services are unavailable afterward; reinstall with install --recover-preserved after initialization to recover retained data.

## Troubleshooting

Purge is rejected without an interactive terminal or matching confirmation; retain necessary backups before uninstalling.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
