# 如何使用 Aster API Key 配置 Codex

[English](../../guides/configure-codex-api-key.md) · [开发者指南](README.md) · [Codex 参考文档](../codex.md)

本指南介绍如何为 Codex 配置 Aster Team 提供的自定义 API Key、自定义 API Base URL 和自定义模型提供商。当组织希望每位开发者使用独立的 Aster 成员 Key，同时由管理员控制额度、路由和用量记录时，这是推荐的接入方式。

## 选择正确的认证路径

以下三种路径并不相同：

| 路径 | 凭据与端点 | 适用场景 |
| --- | --- | --- |
| ChatGPT 登录 | OpenAI 的 ChatGPT 账号 | 使用 OpenAI 托管的订阅与工作区功能 |
| `codex login --with-api-key` | OpenAI Platform API Key，连接 OpenAI | 直接使用按量计费的 OpenAI API |
| Aster 自定义 Provider | Aster 成员 Key，连接客户自己的 Aster 地址 | 客户自托管接入、成员额度、路由与审计 |

OpenAI 文档说明，API Key 认证可用于本地 Codex 工作流，但部分依赖 ChatGPT 工作区或云端的功能可能受限或不可用。Aster 使用 Codex 官方提供的自定义 Provider 配置能力，不会替代或修改 OpenAI 托管的登录流程。参见 OpenAI 的[认证指南](https://learn.chatgpt.com/docs/auth)和[配置参考](https://learn.chatgpt.com/docs/config-file/config-reference)。

## 前置条件

- 安装官方 Codex 应用或 Codex CLI。
- 在 Aster 成员端创建个人 API Key。
- 获取管理员提供的 Aster 公网 API 地址。
- 确认已有管理员启用的模型和健康的 Runner。
- 修改配置前彻底退出 Codex。

## 使用 `asterctl` 配置（推荐）

从 **成员端 → API 文档 → asterctl 工具** 下载 `asterctl`，然后运行：

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
```

在隐藏输入框中填写成员 Key，避免 Key 进入命令历史。`asterctl` 会校验地址和 Key，为当前用户保存 `ASTER_API_KEY`，把 Aster Provider 合并到既有 Codex 配置，并检查最终生效的 Provider。不希望自动启动 Codex 时可移除 `--launch`。

验证结果：

```powershell
asterctl status codex
asterctl doctor codex
```

## 手动配置

无法使用 `asterctl` 时，把成员 Key 存入 `ASTER_API_KEY` 环境变量，并将以下顶层配置加入 Codex 的 `config.toml`：

```toml
model_provider = "aster"

[model_providers]

[model_providers.aster]
base_url = "https://aster.example.com/v1"
env_key = "ASTER_API_KEY"
name = "Aster Team"
wire_api = "responses"

[model_providers.aster.http_headers]
x-openai-actor-authorization = "aster-proxy"
```

Windows 默认文件为 `%USERPROFILE%\.codex\config.toml`；设置了 `CODEX_HOME` 时，应使用该目录。`ASTER_API_KEY` 才是秘密凭据；`x-openai-actor-authorization` 是非秘密兼容标记，不能替代 API Key 认证。

## 常见问题

- **Codex 仍使用旧 Provider：** 彻底退出所有 Codex 进程，确认顶层存在 `model_provider = "aster"`，然后重新打开。
- **读取不到 Key：** 设置用户环境变量后打开新终端，再运行 `asterctl doctor codex`。
- **模型不可用：** 请管理员检查模型启用状态、账号就绪状态和 Runner 健康状态。
- **地址错误：** Codex 应使用以 `/v1` 结尾的 Aster OpenAI 兼容地址。
- **期望使用 Codex 云端功能：** Aster 自定义 Provider 面向本地 Codex 工作流，并不替代 OpenAI 的 Codex 云端或 ChatGPT 工作区功能。

如需使用图片能力，请继续阅读[通过 Aster 在 Codex 中生图和编辑图片](codex-image-generation-editing.md)。完整命令说明见[通过 Aster 使用 Codex](../codex.md)。
