---
title: Aster Team 文档
description: 安装和管理 Aster Team，处理授权、升级与故障，接入客户端和模型 API。
---

# Aster Team 文档

部署 Aster Team、管理团队模型接入，或使用现有实例开始调用。

## 部署与管理

- [首次部署](/zh-cn/administration/)：检查环境，安装服务并完成首次登录。
- [产品授权](/zh-cn/administration/licensing)：导入授权、续期或切换免费版。
- [管理与使用](/zh-cn/administration/management)：接入上游、开放模型并配置成员。
- [备份、升级与恢复](/zh-cn/administration/backup-upgrade)：安排维护窗口并确认升级结果。
- [故障排查](/zh-cn/administration/troubleshooting)：按现象检查服务、Runner、授权和请求。

## 连接与调用

- [客户端工具](/zh-cn/guides/asterctl)：配置 Codex 或 Claude。

- [第一次调用](/zh-cn/start/quickstart)：设置 Base URL、Key 和模型。
- [Responses API](/zh-cn/api/responses)：输入、输出、工具、流式与错误。
- [模型速查](/zh-cn/models/)：切换模型后，查看字段支持情况与映射。
- [图片接口](/zh-cn/api/images)：区分生图与编辑，以及按张计量。
- [OpenAPI 3.1 规范](/openapi.yaml)：供客户端生成与接口速查使用的请求、响应结构。

公共参数规则可供接入参考。你的可用模型还取决于管理员授予的模型权限、当前实例启用状态与接入通道。
