# 如何通过 Aster 在 Codex 中生图和编辑图片

[English](../../guides/codex-image-generation-editing.md) · [开发者指南](README.md) · [兼容 API](../api-reference.md)

Aster 为本地 Codex 提供经过支持的文本、生图和图片编辑路径，请求通过客户自建网关执行。API Key 或自定义 Provider 的功能集合可能与 ChatGPT 登录不同，因此本文专门说明可以直接测试和审计的 Aster 路径。

OpenAI 说明 API Key 认证支持本地 Codex 工作流，同时部分 ChatGPT 工作区或云端功能可能受限或不可用。这并不等于“API Key 登录一定不能生图”。实际能力取决于客户端、Provider 配置、启用的模型和端点支持。参见 OpenAI 的[认证指南](https://learn.chatgpt.com/docs/auth)。

## Aster 路径如何工作

Codex 会把 Responses API 流量发送到已配置的 Aster Provider。配置包含：

- 客户 Aster 的 `/v1` Base URL；
- 开发者自己的 `ASTER_API_KEY`；
- `responses` Wire API；
- 非秘密兼容标记 `x-openai-actor-authorization = "aster-proxy"`。

Aster 验证成员身份、检查额度与策略、选择健康的 Runner 和已授权账号，并记录用量。当已安装的 Aster 版本、管理员启用能力、所选模型、Runner 和客户已连接账号均支持时，图片生成与编辑会通过 Aster 图片端点执行。

## 前置条件

1. 完成[使用 Aster API Key 配置 Codex](configure-codex-api-key.md)。
2. 确认 `asterctl doctor codex` 执行成功。
3. 请管理员确认已经启用支持图片的模型。
4. 确认兼容 Runner 和客户已授权账号处于就绪状态。
5. 配置变更后彻底重启 Codex。

## 测试图片生成

新建一个本地 Codex 任务，给出明确请求，例如：

> 生成一张 1024 × 1024 的 AI 私有化网关产品插图，采用透明感的技术架构图风格，并把结果保存到当前工作区。

确认 Codex 已创建图片文件，并检查这次请求是否以你的成员身份出现在 Aster 用量记录中。

## 测试图片编辑

在新的本地 Codex 任务中附加 PNG 或 JPEG，并要求进行边界明确的修改，例如：

> 只把背景替换成柔和的蓝色渐变，保留产品主体、构图、尺寸和所有前景文字。

确认编辑后的文件和对应 Aster 用量记录都已生成。清晰的约束有助于判断这是成功编辑，而不是重新生成了一张无关图片。

## 直接调用 API

应用也可以直接调用兼容端点：

- `POST /v1/images/generations`
- `POST /v1/images/edits`

生成接口接收 JSON；编辑接口接收 multipart 表单，包括一个或多个 `image[]` 文件。只使用成员端当前显示或 Aster 模型目录返回的模型 ID。认证方式和错误语义见[兼容模型 API](../api-reference.md)。

## 常见问题

- **没有出现图片工具：** 检查 Codex 是否使用 Aster Provider、静态兼容 Header 是否存在，以及新启动的进程能否读取 `ASTER_API_KEY`。
- **`31001 INVALID_API_KEY`：** 创建或复制有效的成员 Key，然后重新配置。
- **`33003 RUNNER_NOT_READY`：** 当前没有健康且兼容的 Runner。
- **`34001 INSUFFICIENT_QUOTA`：** 请管理员增加成员额度。
- **`35002 UPSTREAM_REQUEST_FAILED`：** Runner 已接收任务，但上游服务失败；请保留 Request ID 供排查。
- **文本可用但图片不可用：** 检查 Aster 版本、管理员启用能力、图片模型、已连接账号权益和上游可用性。

本文描述的是 Aster 支持的本地工作流，不承诺全部 Codex 云端功能、全部上游图片模型或第三方能力永久可用。
