---
title: "aster-team-cli backup create"
description: "Create a backup of a Control installation using a local database."
---

# aster-team-cli backup create

Create a backup of a Control installation using a local database.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Local SQLCipher only. External databases require a database-native snapshot and matching installation configuration, identity, and keys; this command does not provide their full backup/restore.

## Syntax

```text
aster-team-cli backup create [--output <PATH>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--output <PATH>` | No | Timestamped archive under installation backups | Must be a new archive path that does not exist. |

Use `aster-team-cli backup create --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli backup create
```

## Configuration and runtime effects

Temporarily stop relevant active services, create a protected archive, then restart the previously active services. Store the archive securely because it contains configuration and keys.

## Result

Print the final archive path. A service stop, archive, or service restart failure can fail the command; check service state.

## Troubleshooting

Inspect role mismatch, invalid archive permissions, an existing output path, or missing restore confirmation; do not bypass validation.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
