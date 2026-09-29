---
title: "aster-team-cli install"
description: "Install a Control host from the initialized and selected release."
---

# aster-team-cli install

Install a Control host from the initialized and selected release.

Run on the target host as Linux root/with sudo or in elevated Windows PowerShell. On Windows use the absolute CLI path from the installation overview; examples below use Linux.

## Before running

Verify and extract the release for the host platform, run init.sh or init.ps1, and use the installed stable CLI. Use upgrade for an existing installation.

## Syntax

```text
aster-team-cli install [OPTIONS]
```

## Parameters

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--unattended` | No | false | Disable prompts; require initial owner email and password file. |
| `--recover-preserved` | No | false | Recover a preserved installation without changing credentials; cannot combine with new owner, database, access, or local Runner configuration. |
| `--owner-email <EMAIL>` | Conditional | — | Owner email for unattended installation. |
| `--owner-password-file <PATH>` | Conditional | — | Password file for unattended installation; root-owned with mode 0600 on Linux. |
| `--install-local-runner` | No | false | Install a local Runner during unattended installation. |

Use `aster-team-cli install --help` to inspect help for the installed version.

### Access options

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--access-protocol <VALUE>` | No | http | Unattended only; http or https. |
| `--access-host <HOST>` | No | Primary LAN IPv4 | Unattended only; public IP or base domain. |
| `--bind-address <IP>` | No | Installer-selected | Unattended only; service bind address. |
| `--certificate-source <VALUE>` | No | caddy for HTTPS | Unattended only; caddy uses an internal CA, provided uses supplied PEM files. |
| `--tls-certificate <PATH>` | Conditional | — | PEM certificate chain for provided mode. |
| `--tls-private-key <PATH>` | Conditional | — | Unencrypted PEM private key for provided mode. |

### External database options

| Parameter | Required | Default | Description |
| --- | --- | --- | --- |
| `--database-config <PATH>` | No | Local SQLCipher | Unattended only; non-secret database JSON for a new Linux amd64 installation. |
| `--database-password-file <PATH>` | Conditional | — | Pair with --database-config; root-owned, mode 0600. |
| `--database-ca-certificate <PATH>` | Conditional | — | CA PEM when custom_ca is enabled in database configuration. |

## Examples

```bash
sudo aster-team-cli install
sudo aster-team-cli install --unattended --owner-email owner@example.com --owner-password-file /root/owner.password --install-local-runner
```

## Configuration and runtime effects

Create installation configuration, identity, database, and services, and run required migrations. Local SQLCipher is the default; see the installation overview for external database support, JSON, and backup boundaries. A valid bundled free license is imported on a fresh install; upgrades preserve existing licensing.

## Result

Print access endpoints and initial owner information. Store the credentials and inspect the installation with status.

## Troubleshooting

Stop for an unselected release, insufficient privileges, conflicting options, or mismatched database identity. Use --recover-preserved for retained data instead of creating a new owner over an existing installation.

[All commands](/en/tools/aster-team-cli/commands) · [Tool overview](/en/tools/aster-team-cli/)
