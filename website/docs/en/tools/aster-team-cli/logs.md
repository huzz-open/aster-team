---
title: "aster-team-cli logs"
description: "Read Control or Runner service logs."
---

# aster-team-cli logs

Read Control or Runner service logs.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli logs <TARGET> [--follow]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `TARGET` | Yes | — | control or runner. |
| `--follow` | No | false | Follow new entries until Ctrl+C. |

Use `aster-team-cli logs --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli logs control
sudo aster-team-cli logs runner --follow
```

## Configuration and runtime effects

Read-only; resolve the active service automatically. Linux shows the latest 200 lines by default; Windows reads the latest 200 lines from the relevant installation log.

## Result

Print existing entries; --follow remains running to receive new entries.

## Troubleshooting

Check the installed role and status when a service or log file is missing. Use trace to locate a single request.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
