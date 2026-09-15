# Codex authentication: ChatGPT sign-in vs OpenAI API key vs Aster member key

[简体中文](../zh-CN/guides/codex-authentication-comparison.md) · [Developer guides](README.md) · [Codex setup](configure-codex-api-key.md)

Codex can be used through different authentication and provider paths. They are not interchangeable: each path selects a credential, endpoint, billing or entitlement source, and feature boundary.

| | ChatGPT sign-in | OpenAI Platform API key | Aster member key |
| --- | --- | --- | --- |
| Credential | OpenAI/ChatGPT identity | OpenAI Platform API key | Per-member Aster API key |
| Endpoint | OpenAI-hosted | OpenAI-hosted | Customer Aster deployment |
| Entitlement or billing | Eligible ChatGPT plan/workspace | Standard API usage billing | Customer-controlled Aster grant plus connected provider capacity |
| Aster quota and audit | No | No | Yes, per member |
| Upstream credential given to developer | Developer uses own OpenAI identity | Developer holds the OpenAI API key | No; Aster member key is used instead |
| Local Codex workflows | Yes | Yes | Yes, through a custom provider |
| OpenAI Codex cloud/workspace features | According to the ChatGPT plan/workspace | Some features may be limited or unavailable | Not provided by Aster; this is a local custom-provider path |
| Image generation and editing | Depends on current OpenAI product capability | Depends on current API/client/model support | Supported by the Aster local path when the installed version, policy, model, Runner, account, and provider support it |

OpenAI's [authentication guide](https://learn.chatgpt.com/docs/auth) states that API-key authentication supports local Codex workflows and uses standard API pricing, while some ChatGPT workspace or cloud features may be limited or unavailable. The [Codex configuration reference](https://learn.chatgpt.com/docs/config-file/config-reference) documents custom providers, including `base_url`, `env_key`, static headers, and the Responses wire API.

## `codex login` is not Aster provider setup

`codex login --with-api-key` sends an OpenAI Platform API key to OpenAI's endpoint. Aster setup instead selects `model_provider = "aster"`, points `base_url` to the customer deployment, and reads `ASTER_API_KEY`. Use [the Aster setup guide](configure-codex-api-key.md) rather than treating the two flows as equivalent.

## Which path should I use?

- Use **ChatGPT sign-in** when you need OpenAI-hosted subscription or workspace features and do not need Aster governance for that session.
- Use an **OpenAI Platform API key** for direct OpenAI API usage and billing.
- Use an **Aster member key** when the organization needs private deployment, administrator-controlled model access, independent member quota, revocation, routing, and usage records.

An Aster member key is not an OpenAI API key and does not include an account, subscription, credit, network service, or additional upstream entitlement.
