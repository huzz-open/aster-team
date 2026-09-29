---
title: "aster-team-cli password reset-admin"
description: "Reset an administrator password on the Control host and revoke that account’s sessions."
---

# aster-team-cli password reset-admin

Reset an administrator password on the Control host and revoke that account’s sessions.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Syntax

```text
aster-team-cli password reset-admin [--email <EMAIL>]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--email <EMAIL>` | No | Prompt | Target active administrator email; use the terminal prompt when omitted. |

Use `aster-team-cli password reset-admin --help` to inspect help for the installed version.

## Examples

```bash
sudo aster-team-cli password reset-admin
sudo aster-team-cli password reset-admin --email admin@example.com
```

## Configuration and runtime effects

Prompt twice without echo, update the password hash, and revoke all sessions of the target administrator. Passwords must be 12–1024 bytes. Reset member passwords through the admin interface.

## Result

Sign in again with the new password after success; the command does not require a web session.

## Troubleshooting

Control hosts only; an unavailable account, mismatched password confirmation, or database access failure prevents success.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
