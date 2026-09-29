---
title: "aster-team-cli trace"
description: "Find a request in active Control service logs using its Aster request ID."
---

# aster-team-cli trace

Find a request in active Control service logs using its Aster request ID.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli trace [REQUEST_ID] [--hours <HOURS>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `REQUEST_ID` | No | Prompt | 8–160 letters, digits, dots, underscores, or hyphens, from the error response or X-Aster-Request-ID. |
| `--hours <HOURS>` | No | 24 | Search the last 1–720 hours. |

Use `aster-team-cli trace --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli trace
sudo aster-team-cli trace --hours 72
```

## Configuration and runtime effects

Read Control logs only; no model request is resubmitted and no quota is changed.

## Result

Print matching entries; no matches also returns success with a not-found message.

## Troubleshooting

Reject invalid ID formats or hour ranges. With no matches, verify the time window, log retention, and Control host.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
