# Aster Team Windows package

Windows amd64 is an experimental channel. Stability is not promised and bug fixes may take longer. Linux amd64 is recommended for production use.

这是 Windows 实验版，不承诺稳定，缺陷修复可能较慢，正式使用建议选择 Linux 版本。

This is a signed, offline Windows x64 package. Run PowerShell as Administrator in the extracted package directory:

```powershell
.\init.ps1
# 执行 init.ps1 最后打印的 Next (Control) 命令
```

The default installation root is defined by the signed product layout contract. To choose another absolute directory, use it only during bootstrap:

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

Programs, releases, configuration, keys, SQLCipher data, runtime state, logs, staging files and backups all remain below that root. Windows Task Scheduler contains only service registrations that launch files below the selected root.

With the default root, the printed lifecycle command is:

```powershell
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' install
```

For a custom root, always use the absolute command printed by `init.ps1`; do not run the package copy of the CLI for later lifecycle operations.

Use the printed root-local CLI path with `--help` for lifecycle, license, service, log, backup, restore and uninstall commands.

## Optional isolated instances

Before the **first** `install` (or `runner install`), set process environment variables in the same elevated PowerShell. Give every concurrent instance a different root, prefix and unused ports:

```powershell
.\init.ps1 --install-root 'D:\Aster Lab'
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
& 'D:\Aster Lab\bin\aster-team-cli.exe' install
```

This example uses `\Aster Team\lab\` in Task Scheduler and public ports 21080/21081/21082, Blue ports 21380–21382, Green ports 21480–21482 and Caddy administration port 12019. Domain HTTP/HTTPS ports become 10080/10443. Service prefixes must start with a lowercase letter and contain only lowercase letters, digits and hyphens (maximum 32 characters).

Individual overrides take precedence over the offset:

| Environment variable | Default |
| --- | --- |
| `ASTER_API_PORT`, `ASTER_MEMBER_PORT`, `ASTER_ADMIN_PORT` | 11080, 11081, 11082 |
| `ASTER_BLUE_API_PORT`, `ASTER_BLUE_MEMBER_PORT`, `ASTER_BLUE_ADMIN_PORT` | 11380, 11381, 11382 |
| `ASTER_GREEN_API_PORT`, `ASTER_GREEN_MEMBER_PORT`, `ASTER_GREEN_ADMIN_PORT` | 11480, 11481, 11482 |
| `ASTER_CADDY_ADMIN_PORT` | 2019 |
| `ASTER_DOMAIN_HTTP_PORT`, `ASTER_DOMAIN_HTTPS_PORT` | 80, 443 |

All resolved ports must be unique and between 1 and 65535. With no variables, existing defaults are unchanged. Installation rejects occupied ports or a task namespace owned by another root; it does not stop the other instance.

Resolved settings are persisted in `install.json`. Changing terminal variables afterwards does not retarget status, services, upgrade, restore or uninstall. Do not edit this marker to rename or move an installed instance. Restore requires the same root and instance settings; custom instances reject packages that do not understand these settings. Normal uninstall retains the settings together with customer data. Separate browser/incognito sessions are needed when using the same hostname with different ports.

A rejected fresh preflight can be retried with new environment values before a role has been installed. Once the Control or Runner role exists, settings remain fixed. Windows upgrades do not replace byte-identical service assets. If a changed executable is still running, its old image is retained under a unique `.maintenance-*.previous` name until that process exits; the maintenance task then retries cleanup. The new executable is used on the next start of that service.

## Unified free and paid package

A public package includes a pre-signed `licenses/free-license.json`. The first fresh Control installation verifies and imports it through the same v2 License flow before initializing the database or owner. No network activation or signing private key is needed. A paid License can subsequently be imported into the same installation. Recovery, upgrade, rollback and dedicated Runner installation do not reset an existing License to free.

免费与付费使用同一个安装包。首次全新安装自动验证并导入包内免费证书；免费额度为 3 个可使用模型的成员席位、1 个 Runner、1 个订阅/账号、每人 1 个有效 Key。免费额度由已验签证书控制。付费后私下交付 License，在原安装中导入即可。
