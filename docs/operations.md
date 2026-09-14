# Operations, backup, and upgrades

[简体中文](zh-CN/operations.md) · [Documentation](README.md)

Examples below use the Linux command entry point. On Windows, use the installed absolute `aster-team-cli.exe` path printed by `init.ps1`, omit `sudo`, and keep the same subcommands.

## Routine checks

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
sudo aster-team-cli logs control
sudo aster-team-cli logs runner
```

`status` summarizes service state and public addresses. `doctor` checks installation identity, database access, license state, public endpoints, and service preflight. A running operating-system service alone does not prove that provider requests can succeed.

Follow live logs when reproducing a problem:

```bash
sudo aster-team-cli logs control --follow
sudo aster-team-cli logs runner --follow
```

Control uses blue/green slots. The CLI resolves the active slot automatically; operators should not target a slot-specific service directly.

## Install a dedicated Runner

Use a signed package with the same version, platform, and architecture expected by Control. On Linux:

```bash
sudo ./init.sh
sudo aster-team-cli runner install
```

On Windows, run `init.ps1` as Administrator, then append `runner install` to the installed CLI command printed by the script.

Create a one-use enrollment token in **Admin → Runners** and follow the generated command. Protect the token file with administrator-only permissions. An HTTP LAN endpoint requires the explicit insecure-HTTP option shown by the UI; an HTTPS endpoint using an internal CA requires the public CA certificate.

## Backup

Create a backup before upgrades and material configuration changes:

```bash
sudo aster-team-cli backup create
```

Without an explicit output path, backups are stored under `<install-root>/backups`. Protect backups as sensitive data: they may contain encrypted databases, identity material, and operational configuration.

Before restoring, inspect the current command contract and verify that the backup belongs to the same installation identity:

```bash
sudo aster-team-cli backup restore --help
```

## Upgrade

The preferred path is **Admin → System upgrade**. Upload the signed package matching the current platform and architecture. Aster verifies the package, prepares the inactive slot, runs embedded migrations and health checks, and switches traffic only after the candidate is healthy. Failure leaves the current version serving traffic.

The host CLI alternative is:

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

Upgrade preserves the installation identity, local database, license, subscriptions and accounts, settings, and Runner identity. Database migrations move forward; do not manually switch to an older executable after a successful upgrade.

## Multiple Windows instances

For isolated test instances on one Windows host, choose a unique installation root, task namespace, and port range before the first `install` or `runner install`:

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

This example moves public ports `11080`–`11082` to `21080`–`21082` and isolates scheduled-task names. Use a different root, prefix, and free range for every instance. The saved installation metadata controls later service, upgrade, backup, restore, and uninstall operations.
