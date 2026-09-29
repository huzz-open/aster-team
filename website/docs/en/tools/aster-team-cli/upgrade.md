---
title: "aster-team-cli upgrade"
description: "Upgrade an existing Control installation to the release selected by the new package."
---

# aster-team-cli upgrade

Upgrade an existing Control installation to the release selected by the new package.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Back up and verify the new package, then run init.sh from it with the existing --install-root when customized. On Windows, use the stable CLI path printed by init.ps1.

## Syntax

```text
aster-team-cli upgrade
```

## Parameters

No command-specific parameters.

Use `aster-team-cli upgrade --help` to inspect help for the installed version.

## Examples

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

## Configuration and runtime effects

Run the upgrade flow supported by the deployment, preserving identity, credentials, license, and business data. Services may stop; this command does not guarantee zero downtime. Migrations move forward only; manually switching back to an old binary after upgrade is unsupported.

## Result

After completion, use version, status, and doctor to check the version and runtime.

## Troubleshooting

Preserve errors and logs when release selection, verification, or migration fails. Do not edit schema_migrations or manually replace current to bypass failure.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
