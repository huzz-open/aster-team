# 架构与信任模型

[English](../architecture.md) · [中文文档](README.md)

Aster Team 将策略与身份控制同上游请求执行分离。

```text
开发者客户端
      │ 成员 API Key / HTTPS
      ▼
Control ── 签名单次任务 / WSS ──► Runner ── HTTPS ──► AI 上游
      ▲                              │
      └──────── 流式结果 ────────────┘
```

## 组件

- **Control** 负责成员和 API Key 鉴权、模型与额度策略、选择健康 Runner、记录用量元数据，并提供 Admin、Member 和模型 API。
- **Runner** 是由客户控制的执行节点。它主动通过 WSS 连接 Control，再向获准的 AI 上游发起 HTTPS 请求，不需要开放入站监听端口。
- **Admin UI** 用于管理许可证、订阅/账号、模型、Runner、成员、额度和审计记录。
- **Member UI** 供成员管理密码与 API Key、查看额度和用量，并取得不同客户端的接入说明。

## 凭据边界

成员只获得 Aster API Key，不会获得上游 Access Token、Refresh Token 或上游账号 ID。Control 在本地加密保存上游凭据实例，并按单次任务临时授权给 Runner。

掌握 Runner 操作系统的管理员能够观察该 Runner 处理的提示词和回答。Runner 应只运行在客户控制的主机上；Aster 不把不可信的成员设备描述为零信任执行节点。

## 任务完整性与路由

每个任务授权都具有短有效期、只能使用一次，并绑定 Runner、任务 ID、命令、凭据 revision、nonce 与载荷摘要。无效、过期、字段不匹配或重放的任务会被拒绝。

可用 Runner 组成共享池。Aster 优先复用合适的亲和节点，再选择健康且近期负载较低的候选。请求发送到上游前可以故障切换；流式输出开始后不会迁移正在执行的请求。

没有兼容 Runner 可用时，接口返回 `RUNNER_NOT_READY`（`33003`），且不扣除额度。

## 数据与审计边界

Aster 在本地数据库中保存鉴权、路由、额度记账、模型用量与审计所需的元数据。业务数据库不保存提示词和回答正文。应用或基础设施日志仍可能包含运行细节，对外提供前必须脱敏。

## 网络边界

- 在可信局域网之外，客户端应通过 HTTPS 访问 Control。
- Runner 通过 WSS 访问 Control，并通过 HTTPS 访问上游。
- Runner 到 Control 可以额外信任内部 CA，且不会改变访问上游时的信任根。
- 上游域名白名单只限制网络目的地，不检查或过滤提示词内容。
