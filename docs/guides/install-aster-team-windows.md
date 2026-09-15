# How to install Aster Team on Windows

[简体中文](../zh-CN/guides/install-aster-team-windows.md) · [Deployment guides](README.md) · [Getting started](../getting-started.md)

This guide installs the Aster Team self-hosted AI gateway from a signed Windows amd64 release package. Use it only when the selected [GitHub Release](https://github.com/huzz-open/aster-team/releases) includes a Windows package.

## Verify and extract the package

Open PowerShell as Administrator, then replace the placeholders with the downloaded release and its trusted SHA-256:

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne '<trusted-sha-256>') { throw 'SHA-256 mismatch' }
tar.exe -xzf $archive
Set-Location 'aster-team-<version>-windows-amd64'
```

## Initialize the installation root

```powershell
.\init.ps1
```

Run the absolute **Next (Control)** command printed by `init.ps1`. The default root is `C:\ProgramData\Aster Team`. Choose another root only during the first initialization:

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

All application files, configuration, keys, local data, backups, and maintenance state live under the selected root. Windows Task Scheduler contains service registrations that point back to that root. Use the installed absolute CLI path printed by the script for later operations.

## Open the interfaces

The default public ports are:

| Service | Default address |
| --- | --- |
| Model API | `http://SERVER_IP:11080` |
| Member UI | `http://SERVER_IP:11081` |
| Admin UI | `http://SERVER_IP:11082` |

Allow only the required ports through the host firewall. Plain HTTP is suitable only for a trusted private network; configure HTTPS for untrusted networks.

Sign in with the initial owner credential printed during installation, change the password immediately, and delete `<install-root>\config\control\initial-owner-credentials` after saving the new credential.

## Run isolated test and production instances on one host

Before the first `install` or `runner install`, give each instance a different installation root, service prefix, and port range:

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

This example moves public ports `11080`–`11082` to `21080`–`21082` and isolates scheduled-task names. Use another root, prefix, and unused offset for every additional instance. The selected values are saved with the installation and reused by service, upgrade, backup, restore, and uninstall commands.

The service prefix must contain 1–32 lowercase letters, digits, or hyphens and must begin with a letter. Explicit `ASTER_API_PORT`, `ASTER_MEMBER_PORT`, and `ASTER_ADMIN_PORT` values override the corresponding offset ports.

## Diagnose the installation

Use the installed CLI path and omit `sudo`:

```powershell
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' status
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' doctor --verbose
```

If you selected a custom root, replace the example path. Then follow [Connect an authorized AI account and synchronize models](connect-ai-account-and-sync-models.md).
