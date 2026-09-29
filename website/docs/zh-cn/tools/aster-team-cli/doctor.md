---
title: "aster-team-cli doctor"
description: "诊断主机、安装身份、数据库、许可证、TLS 与服务。"
---

# aster-team-cli doctor

诊断主机、安装身份、数据库、许可证、TLS 与服务。

在目标主机以 Linux root／sudo 或 Windows 管理员 PowerShell 执行。Windows 使用安装概览中的绝对 CLI 路径；下方示例使用 Linux。

## 语法

```text
aster-team-cli doctor [--verbose]
```

## 参数

| 参数 | 必填 | 默认值 | 说明 |
| --- | --- | --- | --- |
| `--verbose` | 否 | false | 同时显示成功子进程的输出。 |

使用 `aster-team-cli doctor --help` 查看当前安装版本的帮助。

## 示例

```bash
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
```

## 配置与运行影响

按安装角色执行诊断并汇总结果，不自动修复配置。

## 执行结果

检查失败会返回非零退出码；verbose 只增加输出，不改变检查范围或修复状态。

## 失败处理

按报告定位权限、配置、数据库、证书或服务问题；提交日志前移除凭据和私密信息。

[全部命令](/zh-cn/tools/aster-team-cli/commands) · [工具概览](/zh-cn/tools/aster-team-cli/)
