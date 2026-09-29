---
title: "aster-team-cli runner backup restore"
description: "Restore a backup of a dedicated Runner."
---

# aster-team-cli runner backup restore

Restore a backup of a dedicated Runner.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Use on a dedicated Runner host; on Control, use backup without the runner prefix.

## Syntax

```text
aster-team-cli runner backup restore --source <PATH> --confirm
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--source <PATH>` | Yes | — | Protected backup archive matching the host role and installation identity. |
| `--confirm` | Yes | false | Confirm replacement of live data; required for restore. |

Use `aster-team-cli runner backup restore --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli runner backup restore --source /root/aster-backup.tar.gz --confirm
```

## Configuration and runtime effects

Restore replaces live data and affects services. The installed restore engine validates the archive; an unrelated installation archive is not a general data import. Custom Windows instances must also match the root and instance configuration.

## Result

After completion, check status and doctor; also use runner status for a dedicated Runner.

## Troubleshooting

Inspect role mismatch, invalid archive permissions, an existing output path, or missing restore confirmation; do not bypass validation.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
