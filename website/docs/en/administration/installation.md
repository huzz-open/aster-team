---
manualSourceHash: c9b6e8c5f0b9e1dd5df40e92e496cd0d849236cf5a069440a6f2ddee7043ba5f
title: Install Aster Team
description: Install Control, select the installation root and access settings, and complete the first administrator login.
---

# Install Aster Team

## Linux

On a new Linux x86-64 server running systemd, with sudo access:

```bash
curl -fsSL https://aster.huzz.top/install.sh | bash
```

The installer follows GitHub's latest stable release, downloads the matching Linux package and checksum from `huzz-open/aster-team`, verifies the archive and starts the package's initialization and interactive installation. Download or checksum failure stops installation. Existing installations must use the [upgrade procedure](/en/administration/backup-upgrade).

The website can generate a command with a version, email, protocol and host:

```bash
curl -fsSL https://aster.huzz.top/install.sh | bash -s -- --version v2.1.1 --email owner@example.com --protocol https --host team.example.com
```

A version alone keeps installation interactive. Access/email options select unattended installation with a local Runner and a generated administrator password displayed once; save it. Missing email is requested in the terminal. HTTPS defaults to Caddy's internal CA. Use interactive or manual installation for custom certificates, database choices, Runner selection or an installation root.

For manual installation, replace the example version and the independently obtained digest:

```bash
archive=aster-team-2.0.0-linux-amd64.tar.gz
printf '%s  %s\n' '<independently obtained SHA-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd aster-team-2.0.0-linux-amd64
sudo ./init.sh
sudo aster-team-cli install
```

The default root is `/opt/aster-team`. Set a custom root only at initialization:

```bash
sudo ./init.sh --install-root /data/aster-team
sudo /data/aster-team/bin/aster-team-cli install
```

Initialization verifies the signed release and installs the stable CLI. `install` performs the business installation. A valid bundled free license is imported before creating the database and administrator; expect `license: active`. Recovery, upgrades and dedicated Runner installs do not overwrite existing authorization. Packages without a free license generate an offline request.

## Windows

Use administrator PowerShell and verify the matching Windows amd64 archive:

```powershell
$archive = 'aster-team-2.0.0-windows-amd64.tar.gz'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '<independently obtained SHA-256>') { throw 'Archive SHA-256 mismatch' }
tar.exe -xzf $archive
Set-Location 'aster-team-2.0.0-windows-amd64'
.\init.ps1
```

Run the `Next (Control)` command printed by initialization. The default root is `C:\ProgramData\Aster Team`; use `.\init.ps1 --install-root 'D:\Aster Team'` on first initialization to choose another root. Subsequently use the stable CLI's absolute path printed by the installer, not the executable in the extracted archive.

Multiple Windows instances must have separate roots, service namespaces and ports. Before the first `install` or `runner install`, set an instance prefix and port offset if needed:

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

This example uses tasks under `\Aster Team\lab\` and API/Member/Admin ports 21080/21081/21082. Internal slot and Caddy ports are offset too. Prefixes contain 1–32 lowercase letters, digits or hyphens and start with a letter. Choose an unused prefix and port range for each instance.

Explicit `ASTER_API_PORT`, `ASTER_MEMBER_PORT`, `ASTER_ADMIN_PORT`, `ASTER_BLUE_*_PORT`, `ASTER_GREEN_*_PORT`, `ASTER_CADDY_ADMIN_PORT`, `ASTER_DOMAIN_HTTP_PORT` and `ASTER_DOMAIN_HTTPS_PORT` override the offset. Final ports must be distinct and in 1–65535. Saved `install.json` settings govern subsequent service, backup, restore and upgrade operations; temporary environment changes do not switch instances. Do not edit metadata to move or rename an instance. Use separate browser sessions for installations sharing a hostname.

## Installation root and first login

Programs, versions, configuration, keys, database, state, temporary maintenance files, backups and logs remain under the selected root. System service registrations point into it. Interactive setup asks for the owner, protocol, address and local Runner. SQLCipher is the default; manual migration is not required.

Save the printed initial credentials. They are also in `<installation-root>/config/control/initial-owner-credentials`. After first Admin login, change the password and save the new one before removing this initial file.

Default trusted-LAN HTTP addresses are Member `http://SERVER_IP:11081`, Admin `http://SERVER_IP:11082` and Model API `http://SERVER_IP:11080`. Use HTTPS on public or untrusted networks. macOS packages are not currently delivered.

## External database

The implementation targets new Linux amd64 installations with MariaDB **11.8.6**, but this external-database matrix has not completed formal signed-package acceptance. Windows, macOS and MySQL are outside this matrix. Only maintenance upgrades are currently available; an external database does not promise uninterrupted upgrades.

Prepare a dedicated empty database and account, separate from Operations. Limit database privileges to SELECT, INSERT, UPDATE, DELETE, CREATE, ALTER, INDEX and REFERENCES. Example non-secret JSON:

```json
{"driver":"mariadb","host":"db.internal.example","port":3306,"database":"aster_team","username":"aster_team","tls":true,"custom_ca":true,"max_connections":10}
```

Keep the password in a root-owned UTF-8 file with mode 0600, separate from the JSON and command arguments. After initialization:

```bash
sudo aster-team-cli install --unattended \
  --owner-email owner@example.com --owner-password-file /root/owner.password \
  --database-config /root/aster-database.json \
  --database-password-file /root/mariadb.password \
  --database-ca-certificate /root/database-ca.pem --install-local-runner
```

For a public CA, set `custom_ca: false` and omit the CA argument. Remote databases require TLS with certificate/hostname verification; only an explicit loopback IP may disable it. Installation binds the database to the installation and does not migrate existing SQLCipher data. Back up the external database consistently with its native tools together with the installation's configuration, identity and keys; local `backup create` alone is insufficient.

## Forgotten administrator password

Log in to the Control host and run `sudo aster-team-cli password reset-admin`, or supply `--email admin@example.com`. Enter and confirm the new password through hidden input. This revokes that administrator's existing sessions. It does not depend on a vendor server or the Member page.

[Check product authorization](/en/administration/licensing) · [Full install parameters](/en/tools/aster-team-cli/install).
