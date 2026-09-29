---
title: "aster-team-cli runner upgrade"
description: "Upgrade the release on a dedicated Runner host."
---

# aster-team-cli runner upgrade

Upgrade the release on a dedicated Runner host.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Verify the new package and initialize it against the existing root, then use the stable CLI.

## Syntax

```text
aster-team-cli runner upgrade
```

## Parameters

No command-specific parameters.

Use `aster-team-cli runner upgrade --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli runner upgrade
```

## Configuration and runtime effects

Perform the dedicated Runner upgrade; service may be interrupted, so choose an appropriate time.

## Result

Check version, runner status, and the online state in Control afterward.

## Troubleshooting

Stop on a role or selected-release mismatch; use upgrade on a Control host.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
