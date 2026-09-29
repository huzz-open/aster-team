---
title: "asterctl status codex"
description: "检查本地 Codex 配置完整性，不发送网络请求。"
---

# asterctl status codex

检查本地 Codex 配置完整性，不发送网络请求。

## 语法

```text
asterctl status codex
```

## 参数

无专用参数。

使用 `asterctl status codex --help` 查看当前安装版本的帮助。

## 示例

```powershell
asterctl status codex
```

## 配置与运行影响

读取配置、当前用户 Key 和模型目录，不改写配置；Key 仅显示脱敏结果。

## 执行结果

输出配置路径、提供方、地址、Key 状态和模型目录；完整时为 ready，不完整时为 incomplete 并以失败退出。

## 失败处理

配置不完整时先执行 setup codex。ready 仅代表本地配置完整；远端认证需用 doctor codex 检查。

[全部命令](/zh-cn/tools/asterctl/commands) · [工具概览](/zh-cn/guides/asterctl)
