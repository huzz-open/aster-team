---
title: "aster-team-cli license status"
description: "Inspect the current Control license state."
---

# aster-team-cli license status

Inspect the current Control license state.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli license status
```

## Parameters

No command-specific parameters.

Use `aster-team-cli license status --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli license status
```

## Configuration and runtime effects

Read license state without changing entitlements. Dedicated Runners do not install independent licenses.

## Result

Show current licensing information; missing means no license is installed and a request/import is needed.

## Troubleshooting

Inspect wrong-host-role or license read/validation errors; read the state text together with the exit result.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
