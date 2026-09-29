---
title: "asterctl doctor codex"
description: "检查 Codex 配置、成员 Key、Aster 连接及本地模型目录。"
---

# asterctl doctor codex

检查 Codex 配置、成员 Key、Aster 连接及本地模型目录。

## 执行前

在 Windows 上完成 setup codex 后执行。

## 语法

```text
asterctl doctor codex
```

## 参数

无专用参数。

使用 `asterctl doctor codex --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl doctor codex
```

## 配置与运行影响

读取本地配置，使用保存的 Key 请求模型接口，并调用本地 Codex 检查提供方与目录；不发送生成请求，不自动修复配置。

## 执行结果

全部检查通过时退出码为 0；任何检查失败时退出码为 1，并打印失败原因。

## 失败处理

Key 缺失或认证失败时重新设置 Key；地址或 TLS 错误时检查部署入口与证书；目录校验失败时核对 Codex 版本。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
