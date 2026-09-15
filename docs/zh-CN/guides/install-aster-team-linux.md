# 如何在 Linux 上安装 Aster Team

[English](../../guides/install-aster-team-linux.md) · [部署指南](README.md) · [快速开始](../getting-started.md)

本指南介绍如何在 Linux amd64 服务器上安装 Aster Team 私有化 AI 网关，打开管理端和成员端，并准备第一次模型请求。

## 安装前准备

从 [Aster Team Releases](https://github.com/huzz-open/aster-team/releases) 获取 Linux 安装包，通过可信渠道取得其 SHA-256，并准备具有 `sudo` 权限的账号和管理员、开发者可访问的内网地址。服务器还需要通过组织批准的网络路径访问管理员准备连接的 AI 服务。

安装包名称采用以下格式：

```text
aster-team-<version>-linux-amd64.tar.gz
```

## 校验并解压安装包

把占位值替换成实际文件名与可信摘要：

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<trusted-sha-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
```

外层摘要用于验证下载的压缩包，初始化还会验证 Release 内部已签名的文件树。部署记录中应保留两项结果，详见[校验 Aster Team Release](../release-verification.md)。

## 初始化并安装 Control

```bash
sudo ./init.sh
sudo aster-team-cli install
```

默认安装根目录为 `/opt/aster-team`。如需更改，只在首次初始化时指定：

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
```

后续状态检查、升级、备份和恢复应使用所选根目录中的稳定 CLI，不要继续从旧版解压目录执行维护命令。

## 完成交互安装

安装器会依次询问初始 Owner、公开访问方式，以及是否安装同机 Runner。对于简单的首次部署，安装本地 Runner 可以最快完成端到端请求。

默认 IP + HTTP 地址为：

| 服务 | 默认地址 |
| --- | --- |
| 模型 API | `http://SERVER_IP:11080` |
| 成员端 | `http://SERVER_IP:11081` |
| 管理端 | `http://SERVER_IP:11082` |

HTTP 只适合可信内网；任何不可信网络都应使用 HTTPS。

安装器会打印初始 Owner 凭据，并把临时副本保存在 `<install-root>/config/control/initial-owner-credentials`。登录管理端后立即改密，安全保存新密码，然后删除临时文件。

## 完成首次使用

1. 打开“产品授权”，确认随包免费 License 或导入的 License 已生效。
2. 打开“Runner 节点”，确认至少一个 Runner 在线。
3. 在“订阅/账号”中连接有权使用的账号。
4. 同步模型并启用允许开发者使用的模型。
5. 创建成员并发放额度。
6. 登录成员端，创建个人 API Key 并发起测试请求。

账号操作详见[连接已授权 AI 账号并同步模型](connect-ai-account-and-sync-models.md)。

## 验证安装

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

`status` 显示服务状态和公开地址；`doctor` 检查安装身份、数据库、License、入口和服务预检。后续维护见[Aster Team 备份、恢复与升级](backup-restore-upgrade.md)。
