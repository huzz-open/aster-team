# How to install Aster Team on Linux

[简体中文](../zh-CN/guides/install-aster-team-linux.md) · [Deployment guides](README.md) · [Getting started](../getting-started.md)

This guide installs the Aster Team self-hosted AI gateway on a Linux amd64 server, opens the administrator and member interfaces, and prepares the first model request.

## Before you begin

Prepare a Linux package from [Aster Team Releases](https://github.com/huzz-open/aster-team/releases), its SHA-256 obtained through a trusted channel, a user with `sudo`, and an internal address reachable by administrators and developers. The server also needs organization-approved outbound HTTPS access to the AI services connected by your administrator.

Package names follow this pattern:

```text
aster-team-<version>-linux-amd64.tar.gz
```

## Verify and extract the package

Replace the placeholders with the release filename and trusted digest:

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<trusted-sha-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
```

The outer checksum verifies the downloaded archive. Initialization also verifies the signed files inside the release. Keep both checks in the deployment record. See [How to verify an Aster Team release](../release-verification.md).

## Initialize and install Control

```bash
sudo ./init.sh
sudo aster-team-cli install
```

The default installation root is `/opt/aster-team`. To use another root, select it during the first initialization:

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
```

Use the stable CLI installed into the selected root for later status, upgrade, backup, and recovery operations. Do not run later maintenance commands from an old extracted release directory.

## Complete the interactive setup

The installer asks for the initial owner, public access settings, and whether to install a same-host Runner. For a simple first deployment, installing the local Runner provides the shortest path to a working request.

The default IP-and-HTTP addresses are:

| Service | Default address |
| --- | --- |
| Model API | `http://SERVER_IP:11080` |
| Member UI | `http://SERVER_IP:11081` |
| Admin UI | `http://SERVER_IP:11082` |

Use plain HTTP only on a trusted private network. Use HTTPS before exposing an interface to an untrusted network.

The installer prints the initial owner credentials and stores a temporary copy at `<install-root>/config/control/initial-owner-credentials`. Sign in to Admin, change the password, store the new credential safely, and delete the temporary file.

## Finish the first-run checklist

1. Open **Product license** and confirm the bundled free license or imported license is active.
2. Open **Runners** and confirm at least one Runner is online.
3. Connect an authorized account in **Subscriptions & accounts**.
4. Synchronize models and enable the models developers may use.
5. Create a member and grant quota.
6. Sign in to Member UI, create a personal API key, and make a test request.

For the detailed account flow, see [Connect an authorized AI account and synchronize models](connect-ai-account-and-sync-models.md).

## Verify the installation

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

`status` reports services and public addresses. `doctor` validates the installation identity, database, license, endpoints, and service preflight. Continue with [Aster Team operations and backups](backup-restore-upgrade.md).
