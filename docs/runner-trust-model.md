# Runner 信任与网络模型

## 信任主体

首版 Runner 由客户管理员控制。普通成员只有 Aster API Key，不注册 Runner，也不会得到上游 Access Token、Refresh Token 或账号 ID。掌握 Runner 操作系统的人能够看到该 Runner 转发的提示词和响应，因此不把陌生成员设备包装成零信任算力。

## 通信与任务票据

Runner 主动建立到 Control 的 WSS 长连接，不开放入站端口：

```text
成员 ──HTTPS──► Control ──签名任务/WSS──► Runner ──HTTPS──► 上游
成员 ◄──────── Control ◄────流式结果──── Runner ◄───────── 上游
```

每个任务票据短期、单次使用，并绑定 Runner、任务 ID、命令、凭据实例、revision、nonce 和载荷 SHA-256。Runner 拒绝过期、重放、签名无效或字段错配的任务；管理员停用或删除 Runner 后不再下发任务。

当前实现使用 Runner 协议 v3 与 `aster.runner-task.v3` 票据。两分钟有效期只限制接收；签名中的 `execution_deadline_ms` 另外限制执行，单任务最多 600 秒，并与 Control 当前请求的剩余预算取较小值。重试、等待响应头与读取响应体消费同一个请求执行预算；完整端到端期限及收尾保证仍按内部升级方案逐步交付。Control 与 Runner 需要校准系统时间，不能利用时钟偏差延长本地单任务上限。

取消帧只通过已认证的 Control 通道发给该连接拥有的任务；Runner 先释放本地上游请求，再报告 `task_cancelled`。截止时间耗尽报告 `task_deadline_exceeded`。这些结果不表示上游确定停止执行或计费，不允许因此自动跨账号重试。Control 连接断开时，该会话的本地任务与写入任务一并清理，不迁移到重连后的新会话。

旧 v2 Schema 与测试向量保留为历史契约，不代表新二进制兼容旧 Runner。首次从 v2 过渡应使用维护窗口协调升级 Control 与所有实际承担流量的 Runner；远程 Runner 也需要更新。当前仍不开放蓝绿能力，不能把协议升级本身当作已经完成不停服交付。

默认使用内置公共 WebPKI 根验证 Control 的 HTTPS/WSS 证书。客户使用内部 CA 时，可通过 `--control-ca-certificate` 或 `ASTER_RUNNER_CONTROL_CA_CERTIFICATE` 额外提供只含 X.509 证书的 PEM；空文件、超大文件、无效证书或混入私钥的 PEM 会被拒绝。该 CA 只加入 Runner 到 Control 的注册与 WSS 客户端，不会加入 Runner 访问 OpenAI/Anthropic 的上游 HTTP 客户端。

Control API 可直接使用 Rustls 加载 `<安装根>/config/tls/server.crt` 与 `server.key`。证书和私钥必须成对、匹配且通过启动预检；没有 TLS 时 API 只能绑定回环地址。因此远程 WSS 不能因配置疏漏降级成明文 WS。

## M:N 路由

凭据属于逻辑账号，不属于 Runner。所有满足协议版本、在线且启用的 Runner 都可成为候选：

- 优先复用逻辑账号上次成功的 Runner；
- 亲和节点不可用时，选择近期转发负载较低的候选；
- 请求尚未开始时可故障切换一次；
- 流式响应开始后不跨 Runner 迁移当前生成；
- v2 License 可限制已创建 Runner 数量；禁用或离线 Runner 仍占用额度，删除后释放。当前产品入口不接受旧内部 v1 License。

Runner 不长期存储账号 Refresh Token。Control 保存经信封加密的凭据实例，按任务临时授权；凭据实例也没有 `runner_id` 字段。

## Refresh Token 并发

某些上游在刷新后会立即使旧 Refresh Token 失效。如果两个 Runner 同时刷新同一实例，它们会竞争同一旧 Token，后返回的结果还可能覆盖先返回的新 Token。因此 Control 对同一个 `credential_instance_id` 签发唯一短租约，并在提交时校验租约摘要、预期 revision 和有效期。

这不是账号与 Runner 的绑定：另一个独立 OAuth grant 会形成另一个凭据实例，拥有自己的 revision 和租约，可以同时在任意 Runner 上刷新。复制同一 Refresh Token 不是独立 grant，会被凭据 HMAC 唯一约束拒绝。

## 可用性与计费

- 没有 Runner 在线：固定返回 `RUNNER_NOT_READY`（`33003`），不扣额度；
- 请求未发送上游：不扣额度；
- 上游返回可信 usage：按 usage 结算；
- Runner 断线且无可信 usage：记录失败，不猜测消耗；
- 后续无状态请求携带完整历史时可以选择另一 Runner，当前流式请求不能无损续传。

候选槽的 `probe` 使用独立 `authorization.kind=probe` 的 v3 签名任务通道和 32 字节随机挑战，固定内部 provider／host，无凭据绑定，最多执行 3 秒。Runner 通过正常的验签、重放、容量及任务期限检查后仅回传挑战和无用量终态，不进行 DNS／HTTP 或真实模型调用。Control 必须校验指定 Runner、挑战与事件顺序；探测通过不等于上游网络或业务整体就绪。此命令属于当前尚未发布的 v3 变更，首次协议过渡仍按维护窗口协调全部 Control／Runner 更新。
