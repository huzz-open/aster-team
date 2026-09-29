---
title: "aster-team-cli license request"
description: "Generate an authorization request and QR code for the current Control host."
---

# aster-team-cli license request

Generate an authorization request and QR code for the current Control host.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli license request [--output <PATH>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--output <PATH>` | No | Timestamped JSON | Write to a safe writable directory outside the signed release tree. |

Use `aster-team-cli license request --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli license request --output /root/aster-license-request.json
```

## Configuration and runtime effects

Generate a request JSON, QR PNG, and terminal QR; do not issue or install a license.

## Result

Send the request to the licensing operator, then use license install with the issued file.

## Troubleshooting

Control hosts only; inspect output-path or installation-identity errors.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
