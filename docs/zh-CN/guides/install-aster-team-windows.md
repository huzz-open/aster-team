# 如何在 Windows 上安装 Aster Team

[English](../../guides/install-aster-team-windows.md) · [部署指南](README.md) · [快速开始](../getting-started.md)

本指南介绍如何使用已签名的 Windows amd64 Release 安装 Aster Team 私有化 AI 网关。只有所选 [GitHub Release](https://github.com/huzz-open/aster-team/releases) 实际包含 Windows 包时才使用本流程。

## 校验并解压安装包

以管理员身份打开 PowerShell，把占位值替换成实际 Release 和可信 SHA-256：

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne '<trusted-sha-256>') { throw 'SHA-256 mismatch' }
tar.exe -xzf $archive
Set-Location 'aster-team-<version>-windows-amd64'
```

## 初始化安装根目录

```powershell
.\init.ps1
```

继续执行 `init.ps1` 最后打印的 **Next (Control)** 绝对路径命令。默认根目录是 `C:\ProgramData\Aster Team`。需要其他位置时，只在第一次初始化时选择：

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

程序、配置、密钥、本地数据、备份和维护状态都位于所选根目录。Windows 任务计划程序只保存指向该根目录的服务注册。后续维护始终使用脚本打印的根内 CLI 绝对路径。

## 打开服务入口

默认公开端口为：

| 服务 | 默认地址 |
| --- | --- |
| 模型 API | `http://SERVER_IP:11080` |
| 成员端 | `http://SERVER_IP:11081` |
| 管理端 | `http://SERVER_IP:11082` |

只在主机防火墙中放行必要端口。HTTP 仅用于可信内网，不可信网络应配置 HTTPS。

使用安装时打印的初始 Owner 凭据登录，立即修改密码；安全保存新凭据后，删除 `<install-root>\config\control\initial-owner-credentials`。

## 在同一主机隔离测试和正式实例

每个实例都应使用不同的安装根目录、服务前缀和端口段。在第一次执行 `install` 或 `runner install` 前设置：

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

该示例把公开端口 `11080`–`11082` 移到 `21080`–`21082`，同时隔离计划任务名称。每个新增实例必须使用不同根目录、前缀和未占用的偏移量。安装会保存这些值，后续服务、升级、备份、恢复和卸载都会继续使用。

服务前缀必须由 1–32 位小写字母、数字或连字符组成，且第一位是字母。显式设置 `ASTER_API_PORT`、`ASTER_MEMBER_PORT` 和 `ASTER_ADMIN_PORT` 时，对应端口优先于统一偏移。

## 检查安装

使用根内 CLI，不需要 `sudo`：

```powershell
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' status
& 'C:\ProgramData\Aster Team\bin\aster-team-cli.exe' doctor --verbose
```

如果使用了自定义根目录，请替换示例路径。然后继续[连接已授权 AI 账号并同步模型](connect-ai-account-and-sync-models.md)。
