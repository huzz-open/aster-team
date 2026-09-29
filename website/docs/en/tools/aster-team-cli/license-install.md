---
title: "aster-team-cli license install"
description: "Import an issued Control license."
---

# aster-team-cli license install

Import an issued Control license.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli license install --source <PATH>
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--source <PATH>` | Yes | — | The issued license JSON file. |

Use `aster-team-cli license install --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli license install --source ./license.json
```

## Configuration and runtime effects

Validate and update the installed license file; do not create a paid entitlement. If Control was running, stop it during import and start it again after the import attempt. Requests may be interrupted; inspect service state if restarting fails.

## Result

Run license status afterward to inspect the entitlement.

## Troubleshooting

Missing files, invalid signatures, or mismatched installation identities are rejected; verify the file belongs to this installation.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
