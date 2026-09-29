---
title: 快速开始：调用 Aster 模型网关
description: 配置 Aster API Key、Base URL 和公开模型 ID，发起第一个 Responses 请求。
---

# 第一次调用

在成员页面创建 Aster API Key，并从成员模型列表复制**公开模型 ID**；它可能与厂商原生型号不同。当前实例的 API 地址显示如下。

<InstanceApiBase language="zh" />

下面的 `ASTER_BASE_URL` 应设为上方显示的 API 地址；在官网阅读时，请从成员端获取该地址。`ASTER_API_KEY` 使用你创建的成员 Key。

<InstanceCurlExample language="zh" kind="responses" />

通用模板：

```bash
curl "$ASTER_BASE_URL/v1/responses" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_PUBLIC_MODEL","input":"你好"}'
```

API Key 是 Aster 成员 Key，不是 OpenAI、DeepSeek 或 GLM 的上游 Key。

## 切换模型

基础文本调用通常只需改变 `model`。如请求中使用了思考等级、工具强制选择、严格 JSON Schema 或图片，请先查[模型速查](/zh-cn/models/)中的字段规则。无法保留原语义的参数会返回请求错误；删除必要约束可能改变程序行为。

如果客户端只能填写模型 ID，可以在已开放的基础模型后追加执行选项：`-fast` 指定快速处理，`-high` 指定高推理强度，两者同时使用时按 `-fast-high` 的顺序拼接。例如 `gpt-5.6-terra-fast-high`。其他推理强度可使用 `-none`、`-low`、`-medium`、`-xhigh` 或 `-max`；处理速度也可显式写为 `-standard`。模型列表只显示基础模型，变体需要在客户端手动填写。具体组合是否可用取决于该模型的上游能力。

## 下一步

- [asterctl 工具](/zh-cn/guides/asterctl)：配置 Codex 或 Claude Code，检查接入状态。
- [Responses](/zh-cn/api/responses)：统一文本与工具调用。
- [Chat Completions](/zh-cn/api/chat-completions)：兼容常见 Chat SDK。
- [Anthropic Messages](/zh-cn/api/messages)：使用 Messages 形状的客户端。
