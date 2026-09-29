---
title: "aster-team-cli install"
description: "从已初始化并选中的发布包安装 Control 主机。"
---

# aster-team-cli install

从已初始化并选中的发布包安装 Control 主机。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

先校验并解压对应平台的发布包，运行包内 init.sh 或 init.ps1，使用它安装的稳定 CLI。已有安装升级应使用 upgrade。

## 语法

```text
aster-team-cli install [OPTIONS]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--unattended` | 否 | false | 关闭交互提问；必须提供初始管理员邮箱和密码文件。 |
| `--recover-preserved` | 否 | false | 恢复保留数据的安装，不更改原凭据；不能与新管理员、数据库、访问或同机 Runner 配置参数组合。 |
| `--owner-email <EMAIL>` | 条件必填 | — | 无人值守安装的管理员邮箱。 |
| `--owner-password-file <PATH>` | 条件必填 | — | 无人值守安装的密码文件；Linux 要求 root 所有、0600。 |
| `--install-local-runner` | 否 | false | 无人值守安装时同时安装本机 Runner。 |

使用 `aster-team-cli install --help` 查看当前安装版本的帮助。

### 访问配置参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--access-protocol <VALUE>` | 否 | http | 无人值守使用；http 或 https。 |
| `--access-host <HOST>` | 否 | 主 LAN IPv4 | 无人值守使用；访问 IP 或基础域名。 |
| `--bind-address <IP>` | 否 | 安装器选择 | 无人值守使用；服务绑定地址。 |
| `--certificate-source <VALUE>` | 否 | HTTPS 默认 caddy | 无人值守使用；caddy 使用内部 CA，provided 使用自备 PEM。 |
| `--tls-certificate <PATH>` | 条件必填 | — | provided 模式的 PEM 证书链。 |
| `--tls-private-key <PATH>` | 条件必填 | — | provided 模式的未加密 PEM 私钥。 |

### 外部数据库参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--database-config <PATH>` | 否 | 本机 SQLCipher | 无人值守使用；Linux amd64 新安装的非敏感数据库 JSON。 |
| `--database-password-file <PATH>` | 条件必填 | — | 配合 --database-config；root 所有、0600。 |
| `--database-ca-certificate <PATH>` | 条件必填 | — | 数据库配置启用 custom_ca 时提供 CA PEM。 |

## 示例

```bash
sudo aster-team-cli install
sudo aster-team-cli install --unattended --owner-email owner@example.com --owner-password-file /root/owner.password --install-local-runner
```

## 配置与运行影响

创建安装配置、身份、数据库和服务，执行所需数据库迁移。默认使用本地 SQLCipher；外部数据库的支持状态、JSON 示例和备份边界见安装概览。首次安装包内有效免费许可证会自动导入，已有许可证不被升级流程替换。

## 执行结果

输出访问入口和初始管理员信息；保管初始凭据。使用 status 检查安装结果。

## 失败处理

发布包未选择、权限不足、参数互斥或数据库身份不匹配时停止。恢复保留数据应使用 --recover-preserved，不要通过重新创建管理员覆盖原安装。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
