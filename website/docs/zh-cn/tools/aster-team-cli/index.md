---
title: aster-team-cli 概览与安装
description: Aster Team 部署管理员的命令行入口，涵盖安装准备、平台支持、服务、许可证、备份、Runner 与全部命令。
---

# aster-team-cli 概览与安装

`aster-team-cli` 在部署主机上管理 Aster Team 的安装、服务和数据。成员电脑上的 Codex／Claude 接入使用 [asterctl](/zh-cn/guides/asterctl)。

文档无需登录。执行管理命令仍需登录目标主机，使用 Linux root／sudo 或 Windows 管理员 PowerShell；网页管理员会话不能代替主机权限。

完整部署流程见[部署路线](/zh-cn/administration/)，按现象排错见[故障排查](/zh-cn/administration/troubleshooting)。下列页面供查询单条命令和参数。

## 选择操作

- 首次部署：[install](/zh-cn/tools/aster-team-cli/install)。
- 日常检查：[status](/zh-cn/tools/aster-team-cli/status)、[doctor](/zh-cn/tools/aster-team-cli/doctor)。
- 请求失败：[trace](/zh-cn/tools/aster-team-cli/trace)、[logs](/zh-cn/tools/aster-team-cli/logs)。
- 升级前准备：[backup create](/zh-cn/tools/aster-team-cli/backup-create)、[upgrade](/zh-cn/tools/aster-team-cli/upgrade)。
- 查找其他操作：[完整命令参考](/zh-cn/tools/aster-team-cli/commands)。

## 平台与执行位置

| 平台 | 交付状态 | 默认安装根 |
| --- | --- | --- |
| Linux amd64 | 推荐；使用 systemd | `/opt/aster-team` |
| Windows amd64 | 实验版 | `C:\ProgramData\Aster Team` |
| macOS | 暂不交付 | — |

工具随对应平台的安装包交付。先验证发布包，再运行其中的初始化脚本；后续使用初始化脚本安装的稳定 CLI，不要继续从解压目录管理实例。具体可下载平台和版本以发行清单为准。

## Linux 初始化

先用从独立可信渠道取得的 SHA-256 校验压缩包，再解压并进入发布包目录。默认根目录执行：

```bash
sudo ./init.sh
sudo aster-team-cli install
```

自定义安装根只在初始化时指定，例如：

```bash
sudo ./init.sh --install-root /data/aster-team
sudo /data/aster-team/bin/aster-team-cli install
```

`init.sh` 校验并选择发布包、安装稳定 CLI；`install` 才执行业务安装。已有安装请使用 [upgrade](/zh-cn/tools/aster-team-cli/upgrade)，并保持原安装根。

## Windows 初始化

在管理员 PowerShell 中校验并解压 Windows 包，进入发布包目录：

```powershell
.\init.ps1
```

继续执行终端打印的 `Next (Control)` 命令。默认根的调用方式如下；自定义根应使用初始化时打印的实际路径：

```powershell
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' install
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' status
```

命令参考中的 `sudo aster-team-cli ...` 是 Linux 示例。Windows 在管理员 PowerShell 中换成上述绝对路径调用，保留后面的子命令和选项，并将文件路径替换为本机路径。

Windows 多实例必须隔离安装根、服务前缀和端口。首次安装前可设置 `ASTER_SERVICE_PREFIX` 和 `ASTER_PORT_OFFSET`；安装后使用保存的实例配置，不通过临时环境变量切换实例。选择正确安装根的 CLI 是管理对应实例的入口。

## 安装参数与初始凭据

默认交互安装会确认管理员、访问协议、地址和是否安装同机 Runner。自动化部署使用 [install 的无人值守参数](/zh-cn/tools/aster-team-cli/install)。管理员密码通过文件传入，不直接写进命令行。

安装完成后保存打印的初始凭据。凭据文件位于 `<安装根>/config/control/initial-owner-credentials`；首次登录后修改密码，确认新密码已保存后再移除初始凭据文件。

新包若携带有效免费许可证，首次 Control 安装会自动导入；否则生成授权申请。先用 [license status](/zh-cn/tools/aster-team-cli/license-status) 检查，再决定是否申请或导入许可证。

## 外部数据库的范围

默认数据库为本机 SQLCipher。当前外部数据库实现面向 Linux amd64 和 MariaDB 11.8.6，尚未完成该矩阵的正式签名包实机验收；Windows、macOS 和 MySQL 不在此支持范围。外部数据库升级不等同于不停服承诺。

使用前创建专用空库和服务账号，不复用 Operations 业务库。非敏感配置示例：

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

账号权限限定到专用库的 `SELECT`、`INSERT`、`UPDATE`、`DELETE`、`CREATE`、`ALTER`、`INDEX`、`REFERENCES`。密码单独保存为 root 所有、0600 的文件。使用内部 CA 时准备对应 CA PEM；使用公共 CA 时设 `custom_ca: false` 并省略 CA 文件。远程数据库必须启用 TLS，只有明确的回环 IP 允许关闭。

文件参数见 [install](/zh-cn/tools/aster-team-cli/install)。安装会绑定数据库和安装身份，不自动迁移已有 SQLCipher 数据。外部数据库备份需要数据库原生一致性快照，以及同一安装的配置、身份和密钥；不能只运行本地 `backup create`。

## 帮助与故障信息

```bash
aster-team-cli version
aster-team-cli --help
aster-team-cli install --help
```

`version` 展示 CLI 构建信息及已安装版本；`--version` 为简短 CLI 版本。网页文档随构建版本更新，旧安装遇到参数差异时以该安装的 `--help` 为准。

请求故障优先收集 Aster request ID、时间和时区、具体错误，以及 [status](/zh-cn/tools/aster-team-cli/status)／[doctor](/zh-cn/tools/aster-team-cli/doctor) 结果。实际凭据、许可证私钥和含密钥的备份不属于公开文档内容。
