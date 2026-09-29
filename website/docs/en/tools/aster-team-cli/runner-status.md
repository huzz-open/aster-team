---
title: "aster-team-cli runner status"
description: "Inspect the local Runner service and the presence of identity files."
---

# aster-team-cli runner status

Inspect the local Runner service and the presence of identity files.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli runner status
```

## Parameters

No command-specific parameters.

Use `aster-team-cli runner status --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli runner status
```

## Configuration and runtime effects

Read service state and check identity/task-key file presence without restarting anything.

## Result

Print Runner state and present/missing for identity and task keys. File presence does not prove connectivity to Control.

## Troubleshooting

A missing service causes an error; inspect connectivity with doctor, logs runner, and the admin online state.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
