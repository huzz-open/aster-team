---
title: "aster-team-cli runner enroll"
description: "Enroll a dedicated Runner with Control using a registration token."
---

# aster-team-cli runner enroll

Enroll a dedicated Runner with Control using a registration token.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Obtain a registration token from the admin interface and copy it to the dedicated Runner host; prepare the Control CA if needed.

## Syntax

```text
aster-team-cli runner enroll --control-url <URL> --token-file <PATH> [--control-ca-certificate <PATH>] [--allow-insecure-http]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--control-url <URL>` | Yes | — | Control connection URL; prefer HTTPS. |
| `--token-file <PATH>` | Yes | — | Admin-generated registration token file; root-owned mode 0600 on Linux. |
| `--control-ca-certificate <PATH>` | No | System trust | For an internal CA, use config/runner/control-ca.pem under the installation root; other storage paths are rejected. |
| `--allow-insecure-http` | No | false | Explicitly allow HTTP; use only when required on a controlled network. |

Use `aster-team-cli runner enroll --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli runner enroll --control-url https://api.aster.example.com --token-file /root/runner-token
```

## Configuration and runtime effects

Update Runner connection configuration, register identity and task keys, and enable/start the Runner service.

## Result

Report successful enrollment and service start; confirm the online state in the admin interface.

## Troubleshooting

Dedicated Runners only; invalid tokens, untrusted TLS, invalid CA paths, or HTTP without explicit permission cause failure.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
