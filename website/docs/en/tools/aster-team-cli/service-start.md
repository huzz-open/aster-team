---
title: "aster-team-cli service start"
description: "Start selected Aster services."
---

# aster-team-cli service start

Start selected Aster services.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli service start <TARGET>
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `TARGET` | Yes | — | control, runner, or all; concrete services are resolved from the installation. |

Use `aster-team-cli service start --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli service start all
```

## Configuration and runtime effects

Start installed services. The command coordinates using a maintenance lock and service ordering; it does not create new service configuration.

## Result

Print the state of affected services; follow with status to inspect dependencies.

## Troubleshooting

Inspect missing services, platform service failures, or an occupied maintenance lock rather than repeatedly issuing the command.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
