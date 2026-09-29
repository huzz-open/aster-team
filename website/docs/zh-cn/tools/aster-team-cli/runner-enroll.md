---
title: "aster-team-cli runner enroll"
description: "使用注册令牌把独立 Runner 加入 Control。"
---

# aster-team-cli runner enroll

使用注册令牌把独立 Runner 加入 Control。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 执行前

在管理端取得注册令牌文件，复制到独立 Runner 主机；按需准备 Control CA。

## 语法

```text
aster-team-cli runner enroll --control-url <URL> --token-file <PATH> [--control-ca-certificate <PATH>] [--allow-insecure-http]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--control-url <URL>` | 是 | — | Control 连接地址；应优先使用 HTTPS。 |
| `--token-file <PATH>` | 是 | — | 管理员生成的注册令牌文件；Linux 使用 root 所有的 0600 文件。 |
| `--control-ca-certificate <PATH>` | 否 | 系统信任 | 内部 CA 时使用安装根 config/runner/control-ca.pem，不能任意选择其他保存位置。 |
| `--allow-insecure-http` | 否 | false | 显式允许 HTTP；仅在确认需要的受控网络使用。 |

使用 `aster-team-cli runner enroll --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli runner enroll --control-url https://api.aster.example.com --token-file /root/runner-token
```

## 配置与运行影响

更新 Runner 连接配置，登记身份和任务密钥，启用并启动 Runner 服务。

## 执行结果

显示注册完成并已启动；在管理端确认在线状态。

## 失败处理

仅支持独立 Runner；令牌无效、TLS 不可信、CA 路径错误或未显式允许 HTTP 时会失败。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
