---
manualSourceHash: 49d278697504e15e7e924eea296d53b95c723a7fea5a141a67441492c50eedae
title: Backup, upgrade and restore
description: Prepare a maintenance window, keep installation identity and data, and verify recovery.
---

# Backup, upgrade and restore

## Before upgrading

Plan a maintenance window and verify the signed package for the target platform/architecture. Protocol transitions require compatible Control and all local/remote Runners: the current source uses Runner v3; the first transition from v2 must coordinate their upgrade. Source/component validation is not proof of released-package acceptance or uninterrupted operation.

Create a backup; without an output argument it goes to `<installation-root>/backups`:

```bash
sudo aster-team-cli backup create
```

External databases also need their native consistent snapshot together with installation configuration, identity and keys. Local `backup create` alone is insufficient. See [external database preparation](/en/administration/installation#external-database).

## Maintenance upgrade

Embedded SQLite/SQLCipher deployments **do not support uninterrupted upgrades**. In Admin's system upgrade page, inspect capabilities and upload the signed `.tar.gz`. The system validates and stages the package, stops the local Runner and both Control slots, confirms they stopped, migrates and starts the candidate, then restores the public entrypoint, stable CLI and previously running local Runner after health checks.

Admin, Member and API calls can be unavailable and streams may be interrupted. If the page disconnects, wait for reconnection and read the task result; do not repeatedly upload the package. An external database alone does not establish blue-green support: use only modes actually offered by the backend.

For a first transition from an older release without the capability-aware upgrade interface, verify/extract the new signed package and use its CLI path:

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

Pass the original `--install-root` when custom, then use that root's stable CLI. On Windows use `init.ps1` and the printed absolute CLI path. Older launchers may still run the CLI from `current`; verify the launcher before expecting interruption recovery, and use the verified new CLI to recover new tasks.

## Failure and recovery

If a candidate fails, the executor stops it and attempts to restore the previous program and service health. Failure to restore requires checking services; it does not guarantee the previous release is always online. The database is not automatically rolled back or replaced with an old snapshot. Unfinished service recovery retains the task for executor retry and blocks another upgrade. Validation/unpacking-only interruptions do not themselves restart business services.

Migrations move forward and are checksummed; do not run SQL manually, change `schema_migrations`, or switch `current` to bypass a failure. After successful upgrade, verify `version`, `status`, `doctor`, login and an actual model request. Remove non-current versions/snapshots only after confirming stability.

Upgrades preserve installation identity, database, license, accounts, settings and Runner identity. Before restoring, confirm that the backup belongs to this installation and read:

```bash
sudo aster-team-cli backup restore --help
```

[Create backup](/en/tools/aster-team-cli/backup-create) · [Restore backup](/en/tools/aster-team-cli/backup-restore) · [Upgrade reference](/en/tools/aster-team-cli/upgrade).
