---
title: "asterctl 概览与安装"
description: "安装成员客户端配置工具，选择 Codex 或 Claude 接入流程，查看全部命令与故障排查。"
---

# asterctl 概览与安装

`asterctl` 在成员电脑上配置 Codex Desktop 或 Claude Code。准备部署的 **API 地址**和 **Aster 成员 API Key**；API 地址可能与成员网页地址不同。服务端安装与运维使用另一套工具 [aster-team-cli](/zh-cn/tools/aster-team-cli/)。

## 选择下一步

- [快速接入](/zh-cn/tools/asterctl/quickstart)：首次配置 Codex 或 Claude。
- [完整命令参考](/zh-cn/tools/asterctl/commands)：按任务查找子命令、参数及示例。
- [故障排查](/zh-cn/tools/asterctl/troubleshooting)：定位配置、认证、目录和版本问题。

## 获取并运行工具

向部署管理员获取当前部署版本的 `asterctl.exe`。当前交付提供 Windows x64 制品；Codex 自动配置目前仅支持 Windows。工具不负责安装 Codex 或 Claude Code，请先安装要使用的客户端。

在保存 `asterctl.exe` 的目录打开 PowerShell，检查版本和命令帮助：

```powershell
.\asterctl.exe version
.\asterctl.exe --help
```

### 加入用户 PATH

需要在任意目录运行时，在工具保存目录执行下列脚本。它会把工具复制到用户目录，并更新当前终端与用户级 PATH。

```powershell
$asterBin = Join-Path $env:USERPROFILE '.aster\bin'
New-Item -ItemType Directory -Path $asterBin -Force | Out-Null
$asterSource = (Resolve-Path -LiteralPath '.\asterctl.exe').Path
$asterTarget = Join-Path $asterBin 'asterctl.exe'
if ($asterSource -ne $asterTarget) {
    Copy-Item -LiteralPath $asterSource -Destination $asterTarget -Force
}
$asterUserPath = [Environment]::GetEnvironmentVariable('Path', 'User')
if (($asterUserPath -split ';') -notcontains $asterBin) {
    [Environment]::SetEnvironmentVariable('Path', "$asterBin;$asterUserPath", 'User')
}
if (($env:Path -split ';') -notcontains $asterBin) {
    $env:Path = "$asterBin;$env:Path"
}
asterctl version
```

下文以已加入 PATH 为例。未安装到 PATH 时，在工具所在目录把 `asterctl` 换成 `.\asterctl.exe` 即可。

## 配置范围

| 客户端 | 配置范围 | API 地址格式 |
| --- | --- | --- |
| Codex Desktop | 当前用户，默认位于 CODEX_HOME 或用户 .codex 目录 | `https://aster.example.com/v1` |
| Claude Code | 指定项目的 .claude/settings.local.json | `https://aster.example.com` |

Codex 的自动配置、在线诊断和移除流程目前要求 Windows。Claude 功能是否可用还取决于已交付的平台制品及已安装客户端版本，不能把源码中的跨平台实现等同于已发布支持。所有模型调用仍受实例能力和成员模型权限控制。
