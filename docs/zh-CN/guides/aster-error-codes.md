# Aster Team 错误码与故障排查参考

[English](../../guides/aster-error-codes.md) · [部署指南](README.md) · [故障排查](../troubleshooting.md)

Aster API 与管理操作的错误包含稳定字符串码、稳定五位数字码和可安全展示的消息。数字码用于检索本文，Request ID 用于追踪某一次执行。

```json
{
  "error": {
    "code": "RUNNER_NOT_READY",
    "message": "当前没有满足条件的 Runner。",
    "number": 33003
  }
}
```

数字码同时位于 `X-Aster-Error-Number` 响应头。`X-Aster-Request-ID` 标识一次请求，不是错误码。

## 模型 API 错误

| 数字码 | 字符串码 | 含义与处理 |
| ---: | --- | --- |
| `31001` | `INVALID_API_KEY` | Key 缺失、无效、已撤销或成员已停用。创建或启用正确的成员 Key。 |
| `32001` | `INVALID_REQUEST` | 请求字段、查询参数或模型变体无效。对照 API 参考检查请求。 |
| `33001` | `MODEL_NOT_FOUND` | 模型未知、未启用或缺少账号映射。查询 `/v1/models` 并检查管理端模型设置。 |
| `33002` | `MODEL_ACCOUNT_NOT_READY` | 模型存在，但没有可用的已连接凭据。检查账号健康与刷新状态。 |
| `33003` | `RUNNER_NOT_READY` | 没有 Runner 同时满足健康、协议、心跳和容量条件。测试 Runner 连通性。 |
| `33004` | `RUNNER_RETRY_FORBIDDEN` | 上游执行或输出已经开始，Aster 不会静默在另一 Runner 重放。请判断操作是否可安全重试。 |
| `34001` | `INSUFFICIENT_QUOTA` | 扣除在途预占后，成员可用额度不足。检查额度与当前请求。 |
| `34005` | `DUPLICATE_REQUEST` | 计量请求 ID 已经预占或结算。确实是新消费时应生成新的操作 ID。 |
| `35001` | `INVALID_UPSTREAM_RESPONSE` | Runner 响应无法验证。保留 Request ID 并检查 Control 与 Runner 日志。 |
| `35002` | `UPSTREAM_REQUEST_FAILED` | Runner 已接单，但上游传输或执行失败。检查上游状态与 Trace。 |
| `35003` | `UPSTREAM_RESPONSE_TOO_LARGE` | 响应超过 Control 处理上限。减少请求输出。 |
| `35004` | `UPSTREAM_USAGE_INVALID` | 缺少可信 Usage 或格式无效，Aster 拒绝猜测结算值。追踪该请求。 |

## License 错误

| 数字码 | 字符串码 | 含义与处理 |
| ---: | --- | --- |
| `51001` | `LICENSE_MISSING` | 安装随包或已签发的签名 License。 |
| `51002` | `LICENSE_EXPIRED` | 导入续费 License，或整理资源后切换免费版。 |
| `51003` | `LICENSE_INVALID` | License 结构、签名、时间或声明无效。重新导入原始签发文件。 |
| `51004` | `LICENSE_TIME_INVALID` | 修正主机时间并检查可信时间状态。 |
| `51005` | `LICENSE_MACHINE_MISMATCH` | 获取为当前安装身份签发的 License。 |
| `51006` | `LICENSE_VERSION_INVALID` | 把 Aster 升级到 License 要求的最低版本。 |
| `51008` | `FEATURE_NOT_LICENSED` | 在“产品授权”检查当前功能集合。 |

## 安装与维护错误

| 数字码 | 字符串码 | 含义与处理 |
| ---: | --- | --- |
| `43003` | `DELIVERY_RELEASE_INVALID` | 安装包签名、文件树或目录结构校验失败。重新下载并验证 Release。 |
| `43004` | `DELIVERY_MAINTENANCE_BUSY` | 另一个安装、升级、恢复或维护任务持有锁。检查已有任务，不要删除锁状态。 |
| `43006` | `DELIVERY_UPGRADE_FAILED` | 升级失败且恢复策略已运行。检查最终服务健康状态与诊断。 |
| `43010` | `DELIVERY_PLATFORM_UNSUPPORTED` | 安装包或主机能力与当前机器不匹配。使用 Release 中受支持的制品。 |
| `43012` | `DELIVERY_LICENSE_FAILED` | License 申请、导入或状态校验失败。运行 `license status` 并保留原文件。 |
| `43013` | `DELIVERY_BACKUP_FAILED` | 备份预检、创建、恢复或恢复回滚失败。检查存储、权限、身份与诊断。 |
| `43014` | `DELIVERY_RUNNER_FAILED` | Runner 注册、配置或服务管理失败。检查 Runner 状态与日志。 |
| `43015` | `DELIVERY_DIAGNOSTIC_FAILED` | 安装、数据库、TLS、Runner 或服务健康检查未通过。运行 `doctor --verbose`。 |

## 管理与运行时错误

| 数字码 | 字符串码 | 含义与处理 |
| ---: | --- | --- |
| `12010` | `MEMBER_SEAT_LIMIT_REACHED` | 启用新的模型消费成员会超过 License 席位。停用不用的成员或更新 License。 |
| `13008` | `UPSTREAM_OAUTH_SESSION_INVALID` | OAuth 会话、回调、State 或有效期无效。重新发起授权。 |
| `13009` | `UPSTREAM_OAUTH_EXCHANGE_FAILED` | 上游拒绝授权兑换，或返回无效身份。检查账号授权和 Runner 连通性。 |
| `13011` | `UPSTREAM_MODEL_SYNC_FAILED` | 上游模型接口失败或没有返回可用模型。检查已有账号与 Runner 后重试同步。 |
| `13015` | `UPSTREAM_ACCOUNT_LIMIT_REACHED` | 逻辑账号达到上限。删除不用的账号或更新 License；停用不会释放容量。 |
| `14001` | `RUNNER_ENROLLMENT_INVALID` | 一次性注册 Token 缺失、过期或已经使用。创建新 Token。 |
| `14004` | `RUNNER_LIMIT_REACHED` | Runner 实体达到上限。删除不用的 Runner 或更新 License。 |
| `15011` | `MAINTENANCE_BUSY` | 另一个升级或清理任务正在排队或运行。继续观察已有任务。 |
| `22004` | `API_KEY_LIMIT_REACHED` | 当前成员达到 Active Key 上限。创建新 Key 前先撤销旧 Key。 |
| `91001` | `BACKEND_STORAGE_UNAVAILABLE` | 服务无法安全访问业务数据库。检查存储健康与权限，恢复后重试。 |
| `91002` | `BACKEND_DATA_INTEGRITY_INVALID` | 安装级数据完整性校验失败。停止写操作，从可信备份恢复或联系支持。 |
| `91003` | `BACKEND_INSTANCE_DRAINING` | 实例正在排空，以 `503` 和 `Retry-After` 拒绝新任务。等待维护完成。 |
| `91004` | `BACKEND_REQUEST_DEADLINE_EXCEEDED` | 原始处理期限已到。重试写操作前先确认原操作结果。 |

## 收集 Trace

模型请求失败时，复制 `X-Aster-Request-ID` 或 JSON 响应中的 Request ID，然后在 Control 运行：

```bash
sudo aster-team-cli trace
```

提交问题时提供准确时间与时区、端点、模型、五位数字码、字符串码，以及脱敏后的 `status` 和 `doctor` 输出。创建 Issue 前移除密码、API Key、Cookie、Access Token、Refresh Token、License 文件、私钥、Prompt 正文和响应正文。
