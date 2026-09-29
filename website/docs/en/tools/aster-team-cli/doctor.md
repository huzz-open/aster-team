---
title: "aster-team-cli doctor"
description: "Diagnose the host, installation identity, database, license, TLS, and services."
---

# aster-team-cli doctor

Diagnose the host, installation identity, database, license, TLS, and services.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli doctor [--verbose]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--verbose` | No | false | Include successful child-process output. |

Use `aster-team-cli doctor --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
```

## Configuration and runtime effects

Run diagnostics appropriate to the installed role and summarize results without automatic configuration repair.

## Result

Failed checks return a nonzero exit code; verbose adds output without changing the checks or repairing state.

## Troubleshooting

Use reported failures to locate permission, configuration, database, certificate, or service issues; remove secrets before sharing diagnostics.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
