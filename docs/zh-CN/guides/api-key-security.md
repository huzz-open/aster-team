# Aster Team API Key 安全最佳实践

[English](../../guides/api-key-security.md) · [开发者指南](README.md) · [成员指南](../member-guide.md)

Aster 成员 API Key 用于向私有化 AI 网关标识人员、设备、开发环境或自动化任务。独立 Key 可以改善额度归属、单独撤销和事件响应。

## 每个 Key 对应一个可归责身份

- 为每位人员和自动化任务创建独立 Aster 成员身份。
- 需要单独撤销时，为不同设备或环境创建不同 Key。
- 不要在团队成员之间复制同一个成员 Key，也不要把个人 Key 写入共享自动化。
- CI 使用专用自动化身份，并与人员访问分开轮换。

## 安全保存 Key

优先使用操作系统凭据库、组织批准的密钥管理器或受保护的环境变量。不要把 Key 放入 URL、Git 跟踪文件、截图、聊天、Issue、构建日志或会留在 Shell 历史中的命令参数。

Codex 可使用 `asterctl setup codex --set-key` 通过隐藏输入读取 Key，并将其保存为当前用户的 `ASTER_API_KEY`。Codex 配置只引用变量名，不包含秘密值。

Claude Code 的 `asterctl` 配置使用项目级文件。将以下路径加入 Git 忽略规则：

```gitignore
**/.claude/settings.local.json
**/.claude/.asterctl-state.json
**/.claude/settings.local.json.asterctl-*.bak
```

## 轮换或撤销 Key

1. 当前 License 允许同一成员拥有更多 Active Key 时，先创建替代 Key。
2. 更新客户端或密钥管理器，并验证一次请求。
3. 在成员端撤销旧 Key。
4. 检查近期用量中是否出现异常模型、时间、端点或请求数量。

已经达到每人 Active Key 上限时，应先撤销旧 Key，再创建新 Key。撤销会释放 Key 容量，并阻止该值继续完成新的认证。

## 处理疑似泄露

立即撤销 Key，不要等待确认它是否已被使用。保留大致泄露时间、Key 名称、成员、受影响环境和相关 Request ID。检查 Aster 用量与审计记录，轮换同一位置保存的其他秘密，并按组织事件流程清理 Git 历史或构建日志中的值。

完整 API Key 只在创建时显示，之后无法找回。丢失后应撤销对应记录并创建新 Key。

## 不携带凭据共享诊断信息

请求支持时使用稳定五位错误码和 `X-Aster-Request-ID`。对 Authorization Header、环境变量输出、本地配置、Cookie、上游 Token、Prompt 和响应正文进行脱敏。参见[错误码参考](aster-error-codes.md)。
