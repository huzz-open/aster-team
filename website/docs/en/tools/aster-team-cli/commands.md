---
title: "aster-team-cli command reference"
description: "Index of all public aster-team-cli subcommands, parameters, and use cases."
---

# aster-team-cli command reference

Choose a command by task; open its page for parameters, examples, and runtime effects.

## Installation and upgrades

- [`install`](/en/tools/aster-team-cli/install): Install a Control host from the initialized and selected release.
- [`upgrade`](/en/tools/aster-team-cli/upgrade): Upgrade an existing Control installation to the release selected by the new package.

## Runtime and troubleshooting

- [`info`](/en/tools/aster-team-cli/info): Inspect the host role, version, endpoints, and important paths.
- [`status`](/en/tools/aster-team-cli/status): Inspect service, license, and database runtime state.
- [`doctor`](/en/tools/aster-team-cli/doctor): Diagnose the host, installation identity, database, license, TLS, and services.
- [`logs`](/en/tools/aster-team-cli/logs): Read Control or Runner service logs.
- [`trace`](/en/tools/aster-team-cli/trace): Find a request in active Control service logs using its Aster request ID.
- [`version`](/en/tools/aster-team-cli/version): Show CLI build metadata and the installed product version.

## Service management

- [`service start`](/en/tools/aster-team-cli/service-start): Start selected Aster services.
- [`service stop`](/en/tools/aster-team-cli/service-stop): Stop selected Aster services.
- [`service restart`](/en/tools/aster-team-cli/service-restart): Restart selected Aster services.

## Licensing and administrators

- [`license request`](/en/tools/aster-team-cli/license-request): Generate an authorization request and QR code for the current Control host.
- [`license install`](/en/tools/aster-team-cli/license-install): Import an issued Control license.
- [`license status`](/en/tools/aster-team-cli/license-status): Inspect the current Control license state.
- [`password reset-admin`](/en/tools/aster-team-cli/password-reset-admin): Reset an administrator password on the Control host and revoke that account’s sessions.

## Backup and restore

- [`backup create`](/en/tools/aster-team-cli/backup-create): Create a backup of a Control installation using a local database.
- [`backup restore`](/en/tools/aster-team-cli/backup-restore): Restore a backup of a Control installation using a local database.

## Dedicated Runner

- [`runner install`](/en/tools/aster-team-cli/runner-install): Install a dedicated Runner host.
- [`runner enroll`](/en/tools/aster-team-cli/runner-enroll): Enroll a dedicated Runner with Control using a registration token.
- [`runner status`](/en/tools/aster-team-cli/runner-status): Inspect the local Runner service and the presence of identity files.
- [`runner upgrade`](/en/tools/aster-team-cli/runner-upgrade): Upgrade the release on a dedicated Runner host.
- [`runner backup create`](/en/tools/aster-team-cli/runner-backup-create): Create a backup of a dedicated Runner.
- [`runner backup restore`](/en/tools/aster-team-cli/runner-backup-restore): Restore a backup of a dedicated Runner.

## Uninstall

- [`uninstall`](/en/tools/aster-team-cli/uninstall): Uninstall services and binaries, preserving business data by default.

## Help and exit codes

```text
aster-team-cli --help
aster-team-cli install --help
aster-team-cli --version
```

Help and version queries return 0 on success; command execution failures return 1 and argument parsing failures normally return 2. Read the reported state as well: for example, trace succeeds even without matching records.
