# Codex 认证方式对比：ChatGPT 登录、OpenAI API Key 与 Aster 成员 Key

[English](../../guides/codex-authentication-comparison.md) · [开发者指南](README.md) · [Codex 配置](configure-codex-api-key.md)

Codex 可以通过不同的认证与 Provider 路径使用。它们不能互换：每种路径选择不同的凭据、端点、计费或权益来源及功能边界。

| | ChatGPT 登录 | OpenAI Platform API Key | Aster 成员 Key |
| --- | --- | --- | --- |
| 凭据 | OpenAI/ChatGPT 身份 | OpenAI Platform API Key | 每位成员独立的 Aster API Key |
| 端点 | OpenAI 托管 | OpenAI 托管 | 客户自己的 Aster 部署 |
| 权益或计费 | 符合条件的 ChatGPT 计划/工作区 | 标准 API 按量计费 | 客户控制的 Aster 额度和已连接上游能力 |
| Aster 成员额度与审计 | 无 | 无 | 有，按成员统计 |
| 是否向开发者提供上游凭据 | 开发者使用自己的 OpenAI 身份 | 开发者持有 OpenAI API Key | 不提供；开发者只使用 Aster 成员 Key |
| 本地 Codex 工作流 | 支持 | 支持 | 通过自定义 Provider 支持 |
| OpenAI Codex 云端/工作区功能 | 取决于 ChatGPT 计划或工作区 | 部分功能可能受限或不可用 | Aster 不提供；这是本地自定义 Provider 路径 |
| 生图与图片编辑 | 取决于 OpenAI 当前产品能力 | 取决于当前 API、客户端和模型支持 | 当 Aster 版本、策略、模型、Runner、账号和上游均支持时，由 Aster 本地路径支持 |

OpenAI 的[认证指南](https://learn.chatgpt.com/docs/auth)说明，API Key 认证支持本地 Codex 工作流并使用标准 API 计费，但部分 ChatGPT 工作区或云端功能可能受限或不可用。[Codex 配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)则说明了自定义 Provider 的 `base_url`、`env_key`、静态 Header 和 Responses Wire API。

## `codex login` 不等于配置 Aster Provider

`codex login --with-api-key` 使用 OpenAI Platform API Key 访问 OpenAI 端点。Aster 配置则会选择 `model_provider = "aster"`，把 `base_url` 指向客户自己的部署，并读取 `ASTER_API_KEY`。请使用[Aster 配置指南](configure-codex-api-key.md)，不要把两种流程视为同一件事。

## 应该选择哪一种？

- 需要 OpenAI 托管的订阅或工作区功能，且该会话不需要 Aster 治理时，使用 **ChatGPT 登录**。
- 需要直接使用 OpenAI API 及其计费时，使用 **OpenAI Platform API Key**。
- 组织需要私有化部署、管理员控制模型、独立成员额度、撤销、路由和用量记录时，使用 **Aster 成员 Key**。

Aster 成员 Key 不是 OpenAI API Key，也不包含账号、订阅、积分、网络服务或额外的上游权益。
