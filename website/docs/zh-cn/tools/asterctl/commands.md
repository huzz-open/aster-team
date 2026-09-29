---
title: "asterctl 命令参考"
description: "asterctl 的完整公开子命令、参数和使用场景索引。"
---

# asterctl 命令参考

按任务选择命令；点击命令名称查看参数、示例和运行影响。

## 接入配置

- [`setup codex`](/zh-cn/tools/asterctl/setup-codex)：配置 Windows Codex Desktop 的 Aster 提供方、成员 Key 和模型目录。
- [`setup claude`](/zh-cn/tools/asterctl/setup-claude)：为指定项目配置 Claude Code 的 Aster 地址、成员 Key 和模型映射。

## 状态与诊断

- [`status codex`](/zh-cn/tools/asterctl/status-codex)：检查本地 Codex 配置完整性，不发送网络请求。
- [`status claude`](/zh-cn/tools/asterctl/status-claude)：检查指定项目的 Claude 托管状态与文件完整性，不发送网络请求。
- [`doctor codex`](/zh-cn/tools/asterctl/doctor-codex)：检查 Codex 配置、成员 Key、Aster 连接及本地模型目录。
- [`doctor claude`](/zh-cn/tools/asterctl/doctor-claude)：检查 Claude 项目配置、认证、客户端版本和平台模型映射。

## 移除与版本

- [`remove codex`](/zh-cn/tools/asterctl/remove-codex)：撤销 asterctl 管理的 Codex 字段与生成模型目录。
- [`remove claude`](/zh-cn/tools/asterctl/remove-claude)：撤销指定项目的 Claude 设置，并在有备份时恢复原文件。
- [`version`](/zh-cn/tools/asterctl/version)：显示 asterctl 的版本和构建提交。

## 帮助与退出码

```text
asterctl --help
asterctl setup codex --help
asterctl --version
```

帮助和版本查询正常结束时返回 0，命令运行失败返回 1，参数解析失败通常返回 2。业务状态仍需阅读输出，例如取消 Claude 配置覆盖会成功退出，但不更新配置。
