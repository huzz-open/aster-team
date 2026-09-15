# How to back up, restore, and upgrade Aster Team

[简体中文](../zh-CN/guides/backup-restore-upgrade.md) · [Deployment guides](README.md) · [Operations reference](../operations.md)

This runbook covers routine health checks, backup creation, signed-package upgrades, and recovery for an Aster Team installation using the built-in local database.

## Check the deployment before maintenance

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

Resolve installation identity, database, license, endpoint, or service failures before beginning an upgrade. A running operating-system service alone does not prove the complete request path is healthy.

## Create and protect a backup

```bash
sudo aster-team-cli backup create
```

Without an explicit output path, the archive is stored under `<install-root>/backups`. The backup preserves the installation state needed for recovery, including the database, identity and key material, product license, access configuration, proxy state, and Runner identity.

Treat the archive as sensitive. Restrict access, copy it to an approved backup location, record its creation time and source installation, and test the recovery procedure in an isolated environment that uses a separate installation root and ports.

## Upgrade from Admin

The preferred path is **Admin → System upgrade**:

1. Download the signed package for the current platform and architecture.
2. Verify its SHA-256; Aster verifies the signed release tree before installation.
3. Create a current backup.
4. Schedule a maintenance window; local-database upgrades can interrupt UI and API access and may stop active streams.
5. Upload the package and monitor the existing task instead of submitting duplicate upgrades after a temporary browser disconnect.

Aster validates the package, prepares the candidate version, stops services when required, executes embedded forward migrations, checks candidate health, and restores the public entry point after success. If candidate startup fails, the maintenance workflow attempts to restore the previous executable state and reports whether service health was recovered.

## Upgrade from the host CLI

After verifying and extracting the new package:

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

If the installation uses a custom root, pass the same `--install-root` to `init.sh` and then use the stable CLI from that root. A successful upgrade preserves the installation identity, local database, license, subscriptions and accounts, settings, and Runner identity.

Database migrations move forward. Do not manually start an older executable against a database after a successful migration.

## Restore a backup

Inspect the command contract before recovery:

```bash
sudo aster-team-cli backup restore --help
```

Then use the explicit source and confirmation options shown by the installed CLI. Confirm that the archive belongs to the same installation identity, platform, installation root, and instance configuration. A restore is a recovery operation, not a general cross-machine migration mechanism.

After recovery, run `status` and `doctor --verbose`, then test Member UI, Admin UI, `/v1/models`, and one low-cost model request. Preserve the failed-maintenance diagnostics until the incident is closed.

For dedicated nodes, use the corresponding `runner backup` and `runner upgrade` commands exposed by the installed CLI.
