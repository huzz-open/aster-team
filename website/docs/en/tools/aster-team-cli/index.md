---
title: aster-team-cli overview and installation
description: The deployment administrator CLI for Aster Team, including platform support, installation, services, licensing, backups, Runners, and the complete command reference.
---

# aster-team-cli overview and installation

`aster-team-cli` manages Aster Team installation, services, and data on the deployment host. To connect Codex or Claude on a member’s computer, use [asterctl](/en/guides/asterctl).

This documentation requires no sign-in. Management commands still require access to the target host as Linux root/with sudo or in elevated Windows PowerShell. A web administrator session does not grant host privileges.

For the complete process, use the [deployment roadmap](/en/administration/) or [troubleshooting guide](/en/administration/troubleshooting). Use the references below for individual commands and options.

## Choose an operation

- First deployment: [install](/en/tools/aster-team-cli/install).
- Routine checks: [status](/en/tools/aster-team-cli/status), [doctor](/en/tools/aster-team-cli/doctor).
- Failed requests: [trace](/en/tools/aster-team-cli/trace), [logs](/en/tools/aster-team-cli/logs).
- Upgrade preparation: [backup create](/en/tools/aster-team-cli/backup-create), [upgrade](/en/tools/aster-team-cli/upgrade).
- Other operations: [complete command reference](/en/tools/aster-team-cli/commands).

## Platforms and execution location

| Platform | Delivery status | Default installation root |
| --- | --- | --- |
| Linux amd64 | Recommended; systemd | `/opt/aster-team` |
| Windows amd64 | Experimental | `C:\ProgramData\Aster Team` |
| macOS | Not currently shipped | — |

The CLI ships in the platform release package. Verify the package before running its initialization script. Subsequently use the installed stable CLI instead of managing the installation from the extracted release directory. Available downloads and versions are determined by the release catalog.

## Linux initialization

Verify the archive using a SHA-256 obtained through an independent trusted channel, extract it, and enter the release directory. For the default installation root:

```bash
sudo ./init.sh
sudo aster-team-cli install
```

Choose a custom root during initialization, for example:

```bash
sudo ./init.sh --install-root /data/aster-team
sudo /data/aster-team/bin/aster-team-cli install
```

`init.sh` verifies/selects the release and installs the stable CLI; `install` performs the application installation. Use [upgrade](/en/tools/aster-team-cli/upgrade) for an existing installation, retaining its original root.

## Windows initialization

In elevated PowerShell, verify and extract the Windows package, enter the release directory, and run:

```powershell
.\init.ps1
```

Continue with the printed `Next (Control)` command. For the default root, invocation looks like this; for a custom root, use the actual path printed during initialization:

```powershell
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' install
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' status
```

Command reference examples using `sudo aster-team-cli ...` target Linux. In elevated PowerShell, replace that prefix with the absolute executable invocation above, retain the subcommand/options, and replace file paths with local ones.

Multiple Windows instances require separate roots, service prefixes, and ports. Set `ASTER_SERVICE_PREFIX` and `ASTER_PORT_OFFSET` before the first installation when needed. Subsequent operations use persisted instance configuration, not temporary environment variables. Invoke the CLI under the intended installation root to select that instance.

## Installation options and initial credentials

Interactive installation asks for the owner, protocol, address, and whether to install a local Runner. For automation, use the [unattended install options](/en/tools/aster-team-cli/install). Supply the initial owner password through a file, not a command-line password value.

Store the printed initial credentials. The credential file is `<installation-root>/config/control/initial-owner-credentials`; change the password after first sign-in and remove this file after storing the replacement securely.

A fresh Control installation imports a valid bundled free license when present; otherwise it generates a request. Inspect [license status](/en/tools/aster-team-cli/license-status) before deciding whether to request or import a license.

## External database scope

Local SQLCipher is the default. The current external database implementation targets Linux amd64 with MariaDB 11.8.6 and has not completed signed-package acceptance on that matrix. Windows, macOS, and MySQL are outside this scope. External database upgrades do not imply zero downtime.

Create a dedicated empty database and service account; do not reuse the Operations database. Example non-secret configuration:

```json
{
  "driver": "mariadb",
  "host": "db.internal.example",
  "port": 3306,
  "database": "aster_team",
  "username": "aster_team",
  "tls": true,
  "custom_ca": true,
  "max_connections": 10
}
```

Restrict account privileges to `SELECT`, `INSERT`, `UPDATE`, `DELETE`, `CREATE`, `ALTER`, `INDEX`, and `REFERENCES` on that database. Store the password separately in a root-owned mode-0600 file. Provide a CA PEM for an internal CA; with public trust, set `custom_ca: false` and omit the CA file. Remote databases require TLS; only explicit loopback IPs may disable it.

See [install](/en/tools/aster-team-cli/install) for file options. Installation binds the database to the installation identity and does not migrate existing SQLCipher data automatically. External database backups require a native consistent snapshot plus matching installation configuration, identity, and keys; local `backup create` is insufficient.

## Help and issue reporting

```bash
aster-team-cli version
aster-team-cli --help
aster-team-cli install --help
```

`version` shows build information and the installed version; `--version` prints the short CLI version. Web documentation follows its build version. If an older installation differs, consult that installation’s `--help`.

For request failures, collect the Aster request ID, time/time zone, specific error, and [status](/en/tools/aster-team-cli/status)/[doctor](/en/tools/aster-team-cli/doctor) results. Real credentials, license private keys, and backups containing keys do not belong in public documentation.
