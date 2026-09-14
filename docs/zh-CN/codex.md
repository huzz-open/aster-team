# 通过 Aster 使用 Codex

[English](../codex.md) · [中文文档](README.md)

Aster 可以作为 ChatGPT 桌面应用中 Codex 和 Codex CLI 的自定义 Responses API Provider。它**不会**把普通 ChatGPT 网页对话转发到 Aster：常规 ChatGPT 聊天界面不提供任意 API Base URL 配置。

Codex 官方支持定义包含 `base_url`、`env_key`、静态请求头和 `responses` 协议的自定义模型 Provider，详见 OpenAI [Codex 配置参考](https://developers.openai.com/codex/config-reference/)。

## 配置前准备

1. 根据使用环境安装官方 ChatGPT 桌面应用或 Codex CLI。
2. 在 Aster Member UI 创建个人 API Key。
3. 打开“API 文档 → asterctl 工具”，下载与当前设备匹配的制品并运行页面给出的一行安装命令。
4. 修改配置前完全退出 Codex。

在 Windows 上，OpenAI 将 Codex 放在 ChatGPT 桌面应用中分发。可以使用[官方 Windows 应用指南](https://developers.openai.com/codex/app/windows/)，或在 PowerShell 中安装同一个 Microsoft Store 产品：

```powershell
winget install --id 9PLM9XGG6VKS -s msstore
```

安装后应用名称显示为 **ChatGPT** 是正常情况，并不是另一款 Aster 或第三方客户端。

## 推荐配置方式

运行 Member UI 生成的命令。典型 Windows 示例为：

```powershell
asterctl setup codex --base-url "https://aster.example.com/v1" --set-key --launch
```

在隐藏输入提示中填写成员 API Key。`asterctl` 会验证平台地址与 Key，为当前用户保存 `ASTER_API_KEY`，把 Aster Provider 合并进既有 Codex 配置，校验实际生效的 Provider，并按需启动 Codex。Key 不会进入命令历史或 Aster 状态文件。

省略 `--launch` 时只配置，不启动 Codex。

## 检查或撤销配置

```powershell
asterctl status codex
asterctl doctor codex
asterctl remove codex
```

`remove codex` 只恢复仍与 `asterctl` 写入值相同的字段，并保留无关配置或用户后来修改的内容。它会保留 `ASTER_API_KEY`；不再使用时，应另行删除该用户环境变量。

## 高级手动配置

仅在 `asterctl` 不可用或需要诊断时使用。先把成员 API Key 保存为用户环境变量，再把以下 Provider 配置放在 Codex `config.toml` 的顶层：

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

Windows 默认路径为 `%USERPROFILE%\.codex\config.toml`。设置了 `CODEX_HOME` 时，应使用该目录。

`ASTER_API_KEY` 才是真实凭据。`x-openai-actor-authorization` 是非密钥兼容标记，不能代替 API Key 鉴权。此方案不依赖 `OPENAI_API_KEY` 或 `OPENAI_BASE_URL`。

## 模型目录行为

`asterctl` 会保留既有模型目录，在支持时增加 Aster 管理的 Fast 条目，并且不会修改默认模型。实际可用模型仍取决于 Aster 管理员开放的范围与上游能力。Codex 模型目录升级后，可以重新运行 setup 刷新合并目录，无需再次输入 Key。
