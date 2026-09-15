# 成员指南

[English](../member-guide.md) · [中文文档](README.md)

Member UI 是开发者的自助入口，提供平台地址、已开放模型、额度和用量、API Key 管理以及不同客户端的接入命令。

Key 保存、轮换、CI 身份与泄露处理见 [API Key 安全最佳实践](guides/api-key-security.md)；已发放、预占、已消耗和可用额度见[额度与用量结算](guides/quota-reservation-and-usage-accounting.md)。

Aster 成员 API Key 只获得管理员策略允许的网关访问权，不是第三方服务商账号、订阅或额度。实际可用性仍取决于客户接入的账号、Runner 网络以及服务商状态，详见[服务边界](service-boundaries.md)。

## 首次登录

使用管理员安全交付的临时密码登录，并立即修改。密码和完整 API Key 都无法找回；Key 丢失后应撤销并重新创建。

## 创建 API Key

建议为每台设备、开发环境或自动化身份创建单独的 Key。只在创建时复制完整值，并保存到操作系统凭据库或密钥管理器；不要写入源码、截图、Shell 历史或 Issue。

OpenAI 兼容 Base URL 为：

```text
https://aster.example.com/v1
```

Anthropic 兼容 Base URL 不带 `/v1`：

```text
https://aster.example.com
```

实际使用 Member UI 显示的平台地址。

## 选择模型

请求 `/v1/models`，或查看接入页面中的模型列表。列表只包含管理员已开放的模型。部分客户端还可以使用表示速度或推理强度的 Aster 模型变体，详见[模型变体](api-reference.md#模型变体)。

## 客户端接入

- 使用页面生成的 curl 示例进行协议级联通测试。
- 在“API 文档 → asterctl 工具”下载 `asterctl`，以配置 [Codex](codex.md) 或 [Claude Code](claude-code.md)。
- 即使多个工具由同一人使用，也应保持一人一 Key；需要独立撤销时可以继续拆分 Key。

## 用量与错误

Member UI 会显示已发放、已预留和已消费额度，以及逐请求用量记录。反馈故障时，在完成脱敏后提供发生时间、接口、模型、`X-Request-ID` 和 Aster 五位错误编号。
