---
title: "aster-team-cli license install"
description: "导入已经签发的 Control 许可证。"
---

# aster-team-cli license install

导入已经签发的 Control 许可证。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli license install --source <PATH>
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--source <PATH>` | 是 | — | 收到的许可证 JSON 文件。 |

使用 `aster-team-cli license install --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli license install --source ./license.json
```

## 配置与运行影响

验证许可证并更新安装中的许可证文件；不会凭空创建付费授权。若 Control 原先正在运行，导入期间会停止该服务，完成导入尝试后再启动，可能中断请求。启动失败时需要检查服务状态。

## 执行结果

随后执行 license status 核对授权状态。

## 失败处理

文件缺失、签名无效或安装身份不匹配会被拒绝；确认文件属于当前安装。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
