# Aster Team

[English](README.md) · [中文文档](docs/zh-CN/README.md) · [下载](https://github.com/huzz-open/aster-team/releases) · [问题反馈](https://github.com/huzz-open/aster-team/issues)

Aster Team 是面向团队的私有化 AI 网关。管理员可以集中接入客户有权使用的 AI 账号或订阅，为成员签发独立 API Key、分配额度，通过客户控制的 Runner 转发请求并审计用量，同时不把上游凭据分发给成员。

**把客户已有并获授权的 AI 能力，转化为面向每位开发者的独立、可治理访问路径。**

## 核心能力

- **私有化部署**：Control、成员入口、本地数据和可选 Runner 均运行在客户管理的基础设施中。
- **统一接入**：开发者使用兼容 OpenAI 或 Anthropic 的接口，不必直接管理共享的上游凭据。
- **成员级控制**：每位成员拥有独立的 Key、额度、状态和用量记录。
- **可审计用量**：请求具有稳定的用量与错误语义；业务数据库不保存提示词和回答正文。
- **灵活执行**：Runner 主动通过 WSS 连接 Control，无需开放入站端口。
- **统一安装包**：免费版和付费版使用同一安装包。全新安装可以自动激活包内免费 License；导入付费签名 License 即可变更权益，无需重装。

## 独立的开发者访问

```text
客户已有并获授权的 AI 账号或订阅
                 │
                 ▼
             Aster Team
          ┌──────┼──────┐
          ▼      ▼      ▼
       开发者 A  开发者 B  CI 任务
       成员 Key  成员 Key  成员 Key
       独立额度  独立额度  独立额度
       独立用量  独立用量  独立用量
```

Aster 不会创建额外的服务商账号，而是基于客户已有并获授权的能力，为成员建立独立、可控制、可审计的访问路径。上游凭据仍由管理员控制，每位开发者只使用自己的 Aster API Key。

常用开发者指南：

- [使用 Aster API Key 配置 Codex](docs/zh-CN/guides/configure-codex-api-key.md)
- [通过 Aster 在 Codex 中生图和编辑图片](docs/zh-CN/guides/codex-image-generation-editing.md)
- [基于一个获授权订阅运行多个独立 Codex 客户端](docs/zh-CN/guides/multiple-independent-codex-clients.md)
- [比较 ChatGPT 登录、OpenAI API Key 与 Aster 成员 Key](docs/zh-CN/guides/codex-authentication-comparison.md)
- [使用 Aster API Key 配置 Claude Code](docs/zh-CN/guides/configure-claude-code-api-key.md)
- [了解开发者访问与数据隔离](docs/zh-CN/guides/developer-access-isolation.md)

部署与运维指南：

- [在 Linux 上安装 Aster Team](docs/zh-CN/guides/install-aster-team-linux.md)
- [在 Windows 上安装 Aster Team](docs/zh-CN/guides/install-aster-team-windows.md)
- [部署独立 Runner](docs/zh-CN/guides/deploy-dedicated-runner.md)
- [连接已授权 AI 账号并同步模型](docs/zh-CN/guides/connect-ai-account-and-sync-models.md)
- [了解免费、付费与离线 License](docs/zh-CN/guides/aster-team-license-guide.md)
- [备份、恢复和升级 Aster Team](docs/zh-CN/guides/backup-restore-upgrade.md)
- [理解成员额度与用量结算](docs/zh-CN/guides/quota-reservation-and-usage-accounting.md)
- [查询 Aster Team 错误码](docs/zh-CN/guides/aster-error-codes.md)

## Aster 不提供什么

Aster 不提供或转售第三方账号、订阅、API 额度或模型权益，也不是通用 VPN、互联网接入、网络出口或地域限制规避工具。客户需要自行准备有权使用的账号、获准访问服务商的网络条件，并承担相应的上游订阅或用量费用。完整说明见[服务边界与客户责任](docs/zh-CN/service-boundaries.md)。

## 开始使用

1. 阅读[架构与信任模型](docs/zh-CN/architecture.md)。
2. 从 [GitHub Releases](https://github.com/huzz-open/aster-team/releases) 下载安装包并完成[发行文件校验](docs/zh-CN/release-verification.md)。
3. 按照[安装与首次使用](docs/zh-CN/getting-started.md)部署 Control。
4. 接入组织有权使用的账号或订阅，并创建成员 API Key。
5. 使用[兼容模型 API](docs/zh-CN/api-reference.md)、[Codex](docs/zh-CN/codex.md)或 [Claude Code](docs/zh-CN/claude-code.md)。

> 只有出现在本仓库 Releases 页面的文件才是公开安装包。在首个 Release 发布前，请勿从第三方镜像下载 Aster Team。

## 文档

| 指南 | 内容 |
| --- | --- |
| [安装与首次使用](docs/zh-CN/getting-started.md) | 安装 Control、登录、接入账号并发起首个请求 |
| [架构与信任模型](docs/zh-CN/architecture.md) | 了解 Control、Runner、信任边界、存储和请求链路 |
| [服务边界](docs/zh-CN/service-boundaries.md) | 了解第三方账号、网络、费用与可用性责任 |
| [管理员指南](docs/zh-CN/admin-guide.md) | 管理许可证、账号、模型、成员、Key 和额度 |
| [成员指南](docs/zh-CN/member-guide.md) | 创建 Key、查找接口地址并使用接入页面 |
| [API 参考](docs/zh-CN/api-reference.md) | 兼容接口、鉴权、模型变体、错误和调用示例 |
| [Codex 接入](docs/zh-CN/codex.md) | 让 ChatGPT 桌面应用中的 Codex 或 Codex CLI 通过 Aster 请求 |
| [Claude Code 接入](docs/zh-CN/claude-code.md) | 使用 Anthropic 兼容接口配置 Claude Code |
| [任务型指南](docs/zh-CN/guides/README.md) | Linux、Windows、Runner、授权、账号、API Key、Codex、Claude Code 与故障排查 |
| [运行维护](docs/zh-CN/operations.md) | 健康检查、日志、备份、升级和恢复 |
| [故障排查](docs/zh-CN/troubleshooting.md) | 排查常见安装、路由和客户端问题 |

完整入口见[中文文档索引](docs/zh-CN/README.md)。

## 支持与 Issue 安全

提交 Issue 时，请勿附带密码、API Key、License 文件、私钥、Cookie、数据库、完整诊断包或未脱敏日志。

## 商标与兼容性

Aster Team 是独立产品。OpenAI、ChatGPT 与 Codex 是 OpenAI 的商标；Anthropic 与 Claude 是 Anthropic 的商标。文档中的名称仅用于说明兼容的客户端与协议，不代表相关公司背书。
