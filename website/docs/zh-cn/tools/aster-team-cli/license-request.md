---
title: "aster-team-cli license request"
description: "生成当前 Control 主机的授权申请与二维码。"
---

# aster-team-cli license request

生成当前 Control 主机的授权申请与二维码。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli license request [--output <PATH>]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--output <PATH>` | 否 | 带时间戳的 JSON | 输出到可写的安全目录；不能写入签名发布树。 |

使用 `aster-team-cli license request --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli license request --output /root/aster-license-request.json
```

## 配置与运行影响

生成申请 JSON、二维码 PNG 和终端二维码，不签发或安装许可证。

## 执行结果

把申请交给授权运营人员，收到许可证后再执行 license install。

## 失败处理

只能在 Control 主机运行；输出路径或安装身份异常时按错误处理。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
