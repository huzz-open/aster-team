# Aster Team

[English](README.md) · [中文文档](docs/zh-CN/README.md) · [下载](https://github.com/huzz-open/aster-team/releases) · [问题反馈](https://github.com/huzz-open/aster-team/issues)

Aster Team 是面向团队的私有化 AI 网关。管理员可以集中接入 AI 订阅账号，为成员签发独立 API Key、分配额度，通过客户控制的 Runner 转发请求并审计用量，同时不把上游凭据分发给成员。

## 核心能力

- **私有化部署**：Control、成员入口、本地数据和可选 Runner 均运行在客户管理的基础设施中。
- **统一接入**：开发者使用兼容 OpenAI 或 Anthropic 的接口，不必直接管理共享的上游凭据。
- **成员级控制**：每位成员拥有独立的 Key、额度、状态和用量记录。
- **可审计用量**：请求具有稳定的用量与错误语义；业务数据库不保存提示词和回答正文。
- **灵活执行**：Runner 主动通过 WSS 连接 Control，无需开放入站端口。
- **统一安装包**：免费版和付费版使用同一安装包。全新安装可以自动激活包内免费 License；导入付费签名 License 即可变更权益，无需重装。

## 开始使用

1. 阅读[架构与信任模型](docs/zh-CN/architecture.md)。
2. 从 [GitHub Releases](https://github.com/huzz-open/aster-team/releases) 下载安装包并完成[发行文件校验](docs/zh-CN/release-verification.md)。
3. 按照[安装与首次使用](docs/zh-CN/getting-started.md)部署 Control。
4. 接入订阅账号并创建成员 API Key。
5. 使用[兼容模型 API](docs/zh-CN/api-reference.md)、[Codex](docs/zh-CN/codex.md)或 [Claude Code](docs/zh-CN/claude-code.md)。

> 只有出现在本仓库 Releases 页面的文件才是公开安装包。在首个 Release 发布前，请勿从第三方镜像下载 Aster Team。

## 文档

| 指南 | 内容 |
| --- | --- |
| [安装与首次使用](docs/zh-CN/getting-started.md) | 安装 Control、登录、接入账号并发起首个请求 |
| [架构与信任模型](docs/zh-CN/architecture.md) | 了解 Control、Runner、信任边界、存储和请求链路 |
| [管理员指南](docs/zh-CN/admin-guide.md) | 管理许可证、账号、模型、成员、Key 和额度 |
| [成员指南](docs/zh-CN/member-guide.md) | 创建 Key、查找接口地址并使用接入页面 |
| [API 参考](docs/zh-CN/api-reference.md) | 兼容接口、鉴权、模型变体、错误和调用示例 |
| [Codex 接入](docs/zh-CN/codex.md) | 让 ChatGPT 桌面应用中的 Codex 或 Codex CLI 通过 Aster 请求 |
| [Claude Code 接入](docs/zh-CN/claude-code.md) | 使用 Anthropic 兼容接口配置 Claude Code |
| [运行维护](docs/zh-CN/operations.md) | 健康检查、日志、备份、升级和恢复 |
| [故障排查](docs/zh-CN/troubleshooting.md) | 排查常见安装、路由和客户端问题 |

完整入口见[中文文档索引](docs/zh-CN/README.md)。

## 仓库边界

本公开仓库只提供发行包、校验值、开发者与用户文档以及公开问题跟踪，不包含 Aster Team 产品源码、签名材料、客户数据、许可证或内部运营文档。

提交 Issue 时，请勿附带密码、API Key、License 文件、私钥、Cookie、数据库、完整诊断包或未脱敏日志。

## 商标与兼容性

Aster Team 是独立产品。OpenAI、ChatGPT 与 Codex 是 OpenAI 的商标；Anthropic 与 Claude 是 Anthropic 的商标。文档中的名称仅用于说明兼容的客户端与协议，不代表相关公司背书。
