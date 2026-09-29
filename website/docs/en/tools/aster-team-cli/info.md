---
title: "aster-team-cli info"
description: "Inspect the host role, version, endpoints, and important paths."
---

# aster-team-cli info

Inspect the host role, version, endpoints, and important paths.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli info
```

## Parameters

No command-specific parameters.

Use `aster-team-cli info --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli info
```

## Configuration and runtime effects

Read installation metadata without changing settings; output differs for Control and dedicated Runner hosts.

## Result

Identify the installation and its endpoints; this is not a service health check.

## Troubleshooting

Missing installation root, role marker, or current release causes failure; use the stable CLI for the correct root.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
