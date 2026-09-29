---
title: "aster-team-cli status"
description: "Inspect service, license, and database runtime state."
---

# aster-team-cli status

Inspect service, license, and database runtime state.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli status
```

## Parameters

No command-specific parameters.

Use `aster-team-cli status --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli status
```

## Configuration and runtime effects

Inspect the current host role without starting or repairing services.

## Result

Control checks its services and runtime dependencies; dedicated Runner checks the Runner. Critical failures result in a failure exit.

## Troubleshooting

Investigate unhealthy items with doctor and logs; stopped services do not imply reinstallation is needed.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
