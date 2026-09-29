---
title: "aster-team-cli backup restore"
description: "Restore a backup of a Control installation using a local database."
---

# aster-team-cli backup restore

Restore a backup of a Control installation using a local database.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Local SQLCipher only. External databases require a database-native snapshot and matching installation configuration, identity, and keys; this command does not provide their full backup/restore.

## Syntax

```text
aster-team-cli backup restore --source <PATH> --confirm
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--source <PATH>` | Yes | — | Protected backup archive matching the host role and installation identity. |
| `--confirm` | Yes | false | Confirm replacement of live data; required for restore. |

Use `aster-team-cli backup restore --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli backup restore --source /root/aster-backup.tar.gz --confirm
```

## Configuration and runtime effects

Restore replaces live data and affects services. The installed restore engine validates the archive; an unrelated installation archive is not a general data import. Custom Windows instances must also match the root and instance configuration.

## Result

After completion, check status and doctor; also use runner status for a dedicated Runner.

## Troubleshooting

Inspect role mismatch, invalid archive permissions, an existing output path, or missing restore confirmation; do not bypass validation.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
