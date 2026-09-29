# 后端错误码

所有 HTTP JSON 错误使用统一响应结构：

```json
{
  "error": {
    "code": "RUNNER_NOT_READY",
    "message": "当前没有可用的 Runner",
    "number": 33003
  }
}
```

- `code` 是稳定的字符串业务码，供程序判断，例如 `UNAUTHENTICATED` 或 `RUNNER_NOT_READY`。
- `number` 是稳定的五位数值业务码，供用户反馈和文档检索；同一种业务错误每次返回同一个数字，禁止随机生成。
- `message` 是可以安全展示给用户的说明；内部堆栈和敏感上下文只写服务日志。
- `number` 同时写入 `X-Aster-Error-Number` 响应头。
- 后端不得把未登记的字符串码原样返回；若程序错误绕过目录校验，响应必须归一为已登记的 `INTERNAL_ERROR`，不能临时分配或拼出一个新数字。
- `contracts/catalogs/error-codes.yaml` 是唯一登记源；Go、Rust 和 TypeScript 常量由 `npm run generate:contracts` 生成，禁止直接修改生成文件。

## 编码规则

错误码格式为“两位系统/模块前缀 + 三位模块内序号”：

```text
11001
││└── 001：模块内固定递增序号
│└─── 1：该端中的认证模块
└──── 1：管理端
```

当前前缀：

| 前缀 | 系统或模块 |
|---|---|
| `11xxx` | 管理端认证 |
| `12xxx` | 管理端成员与额度 |
| `13xxx` | 管理端订阅/账号与模型 |
| `14xxx` | 管理端 Runner |
| `15xxx` | 管理端平台配置 |
| `21xxx` | 成员端身份 |
| `22xxx` | 成员端 API Key |
| `23xxx` | 成员端额度与兑换 |
| `24xxx` | 成员端用量查询 |
| `31xxx` | 对外模型 API 认证 |
| `32xxx` | 对外模型 API 请求校验 |
| `33xxx` | 对外模型 API 路由 |
| `34xxx` | 对外模型 API 结算 |
| `35xxx` | 对外模型 API 上游结果 |
| `41xxx` | Runner 连接与认证 |
| `42xxx` | Runner 运行状态 |
| `43xxx` | 离线安装和发布物 |
| `51xxx` | 客户许可证 |
| `61xxx`—`65xxx` | Operations 各业务模块 |
| `66xxx` | Operations Web Entry |
| `69xxx` | Operations 公共错误 |
| `91xxx` | 客户后端公共请求错误 |
| `99xxx` | 客户后端未预期故障 |

当前已登记的客户后端公共错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `91002` | `BACKEND_DATA_INTEGRITY_INVALID` | 身份、会话或其他关键数据的安装级完整性校验失败；必须拒绝请求 |
| `91003` | `BACKEND_INSTANCE_DRAINING` | 当前实例已停止接收新业务请求；返回 503 与 Retry-After，已有请求继续收尾 |
| `91004` | `BACKEND_REQUEST_DEADLINE_EXCEEDED` | 请求超过原始处理期限；响应头尚未返回时使用 504，已开始的后台写入继续收尾，重试前应确认操作结果 |
| `91005` | `BACKEND_DATABASE_CONNECTION_FAILED` | 数据库连接失败；检查数据库服务与连接配置 |
| `91006` | `BACKEND_DATABASE_SCHEMA_MISMATCH` | 数据库表、列或迁移版本与程序不匹配 |
| `91007` | `BACKEND_DATABASE_QUERY_FAILED` | 数据库查询或写入失败；具体原因见服务端日志 |
| `91008` | `BACKEND_LOCAL_STATE_FAILED` | 服务端本地状态文件或锁操作失败 |
| `91009` | `BACKEND_INTERNAL_OPERATION_FAILED` | 无法归入以上类别的内部操作失败；具体原因见服务端日志 |
| `91010` | `BACKEND_DATABASE_DECODE_FAILED` | 数据库字段读取时类型不匹配 |
| `91011` | `BACKEND_STORAGE_NOT_CONFIGURED` | 服务端没有配置数据库 |
| `91012` | `BACKEND_TIME_CONVERSION_FAILED` | 服务端时间无法转换为存储格式 |
| `91013` | `BACKEND_RANDOMNESS_UNAVAILABLE` | 操作系统随机数生成失败 |
| `91014` | `BACKEND_RUNNER_TASK_ISSUER_NOT_CONFIGURED` | Runner 任务签发配置缺失 |
| `91015` | `BACKEND_TASK_EXECUTION_FAILED` | 后台任务执行失败 |
| `91016` | `BACKEND_AUDIT_READ_CONFLICT` | 审计记录持续变化，稍后可重新读取 |

当前已登记的离线安装与发布物错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `43001` | `DELIVERY_ROOT_REQUIRED` | 修改主机状态的交付命令未使用 root 权限 |
| `43002` | `DELIVERY_RELEASE_NOT_SELECTED` | 尚未通过 `init.sh` 选择已签名发布包 |
| `43003` | `DELIVERY_RELEASE_INVALID` | Release 签名、完整文件树或目录结构校验失败 |
| `43004` | `DELIVERY_MAINTENANCE_BUSY` | 统一维护锁已由另一个安装、升级、恢复或维护操作持有 |
| `43005` | `DELIVERY_INSTALL_FAILED` | Control 或专用 Runner 首次安装失败 |
| `43006` | `DELIVERY_UPGRADE_FAILED` | 升级失败并已按回滚策略处理 |
| `43007` | `DELIVERY_INPUT_INVALID` | CLI 参数、交互输入或本机角色不符合命令要求 |
| `43008` | `DELIVERY_FILESYSTEM_FAILED` | 无法按受限权限安全读写交付状态或文件 |
| `43009` | `DELIVERY_COMMAND_FAILED` | 底层受信维护命令执行失败 |
| `43010` | `DELIVERY_PLATFORM_UNSUPPORTED` | 主机不是 Linux x86-64、未运行 systemd 或缺少必需系统命令 |
| `43011` | `DELIVERY_SERVICE_FAILED` | systemd 服务控制或日志读取失败 |
| `43012` | `DELIVERY_LICENSE_FAILED` | 离线许可证申请、安装或状态校验失败 |
| `43013` | `DELIVERY_BACKUP_FAILED` | 备份创建、预检、恢复或恢复回滚失败 |
| `43014` | `DELIVERY_RUNNER_FAILED` | 专用 Runner 注册、配置或状态管理失败 |
| `43015` | `DELIVERY_DIAGNOSTIC_FAILED` | 安装身份、数据库、TLS、Runner 配置或服务健康诊断未通过 |
| `43016` | `DELIVERY_UNINSTALL_FAILED` | 卸载边界检查或文件清理失败 |

当前已登记的客户许可证错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `51001` | `LICENSE_MISSING` | 尚未导入客户许可证，受保护业务入口拒绝执行 |
| `51002` | `LICENSE_EXPIRED` | 当前许可证已经超过有效期 |
| `51003` | `LICENSE_INVALID` | 许可证结构、签名、时间或其它声明无效 |
| `51004` | `LICENSE_TIME_INVALID` | 当前时间早于许可证生效时间或不满足可信时间状态 |
| `51005` | `LICENSE_MACHINE_MISMATCH` | 许可证中的安装身份或机器指纹与当前主机不一致 |
| `51006` | `LICENSE_VERSION_INVALID` | 当前 Rust 程序版本低于许可证要求的最低版本 |
| `51007` | `LICENSE_ROLLBACK_REJECTED` | 许可证签发序列、签发时间或本机单调状态发生回退 |
| `51008` | `FEATURE_NOT_LICENSED` | 许可证没有包含当前业务入口要求的功能 |
| `51009` | `MEMBER_SEAT_OVERAGE_EXPIRED` | 已占用成员席位超过许可证上限且宽限期已经结束 |

Operations Web Entry 当前登记：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `66001` | `API_UNAVAILABLE` | Operations Web Entry 暂时无法连接 Operations API |

Operations 认证与会话错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `61001` | `UNAUTHORIZED` | 未登录、会话无效或会话已过期 |
| `61002` | `INVALID_CREDENTIALS` | Operations 登录凭据错误 |
| `61003` | `ORIGIN_FORBIDDEN` | 请求来源不在受信范围 |
| `61004` | `CSRF_INVALID` | 写请求的 CSRF 校验失败 |
| `61005` | `CURRENT_PASSWORD_INVALID` | 当前密码错误 |
| `61006` | `PASSWORD_INVALID` | 新密码不符合安全要求 |
| `61007` | `REAUTHENTICATION_FAILED` | 高风险操作的当前密码复核失败 |
| `61008` | `LOGIN_FAILED` | 登录流程内部失败 |
| `61009` | `LOGOUT_FAILED` | 退出流程内部失败 |
| `61010` | `PASSWORD_CHANGE_FAILED` | 修改密码流程内部失败 |

Operations 客户模块错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `62001` | `CUSTOMERS_LIST_FAILED` | 读取客户列表失败 |
| `62002` | `CUSTOMER_CREATE_FAILED` | 创建客户失败 |
| `62003` | `CUSTOMER_PROFILE_FAILED` | 读取客户档案失败 |
| `62004` | `CUSTOMER_UPDATE_FAILED` | 更新客户资料失败 |
| `62005` | `CONTACT_CREATE_FAILED` | 创建客户联系人失败 |
| `62006` | `BILLING_PROFILE_UPDATE_FAILED` | 更新客户开票资料失败 |

Operations 商务模块错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `63001` | `PLANS_LIST_FAILED` | 读取套餐列表失败 |
| `63002` | `PLAN_CREATE_FAILED` | 创建套餐失败 |
| `63003` | `PLAN_PRICE_PUBLISH_FAILED` | 发布套餐价格版本失败 |
| `63004` | `ORDERS_LIST_FAILED` | 读取订单列表失败 |
| `63005` | `ORDER_CREATE_FAILED` | 创建订单失败 |
| `63006` | `PAYMENT_CONFIRM_FAILED` | 确认线下收款失败 |
| `63007` | `REFUND_NOTES_LIST_FAILED` | 读取退款记录失败 |
| `63008` | `REFUND_NOTE_CREATE_FAILED` | 创建退款记录失败 |
| `63009` | `TRIALS_LIST_FAILED` | 读取试用列表失败 |
| `63010` | `TRIAL_CREATE_FAILED` | 创建试用失败 |
| `63011` | `TRIAL_EXTENSION_FAILED` | 延长试用失败 |

Operations 许可证与交付模块错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `64001` | `FULFILLMENT_INVALID` | 交付状态或参数无效 |
| `64002` | `LICENSE_POLICY_CREATE_FAILED` | 创建许可证策略失败 |
| `64003` | `RELEASES_LIST_FAILED` | 读取发布物列表失败 |
| `64004` | `RELEASE_IMPORT_FAILED` | 导入发布物失败 |
| `64005` | `DELIVERIES_LIST_FAILED` | 读取交付记录失败 |
| `64006` | `DELIVERY_CREATE_FAILED` | 创建交付记录失败 |
| `64007` | `DELIVERY_RECEIPT_FAILED` | 读取交付回执失败 |
| `64008` | `LICENSES_QUERY_FAILED` | 读取许可证策略失败 |
| `64009` | `LICENSE_ISSUANCES_QUERY_FAILED` | 读取许可证签发记录失败 |
| `64010` | `LICENSE_ISSUANCE_FAILED` | 离线许可证签发失败 |

Operations 风险、备份与审计模块错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `65001` | `RISK_NOTES_LIST_FAILED` | 读取风险记录失败 |
| `65002` | `RISK_NOTE_CREATE_FAILED` | 创建风险记录失败 |
| `65003` | `BACKUP_HISTORY_FAILED` | 读取备份历史失败 |
| `65004` | `OPERATIONS_EXPORT_FAILED` | 导出运营数据失败 |
| `65005` | `AUDIT_LIST_FAILED` | 读取审计事件失败 |

Operations 公共错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `69001` | `INTERNAL_ERROR` | 未预期的 Operations 内部错误 |
| `69002` | `DATABASE_UNAVAILABLE` | Operations 核心数据库不可用 |
| `69003` | `BUSINESS_STORE_UNAVAILABLE` | Operations 业务存储不可用 |
| `69004` | `INVALID_JSON` | 请求 JSON 无法解析或不符合结构 |
| `69005` | `INVALID_LIMIT` | 分页数量超出允许范围 |
| `69006` | `VALIDATION_FAILED` | 通用业务输入校验失败 |
| `69007` | `NOT_FOUND` | 指定资源不存在 |
| `69008` | `OVERVIEW_FAILED` | 读取运营概览失败 |
| `69009` | `INVALID_OFFSET` | 分页偏移量超出允许范围或格式无效 |

当前已登记的管理端认证、成员管理与成员端身份错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `11001` | `ADMIN_INVALID_CREDENTIALS` | 管理员邮箱或密码错误 |
| `11002` | `ADMIN_UNAUTHENTICATED` | 管理员会话不存在、过期或无效 |
| `11003` | `ADMIN_PASSWORD_CHANGE_REQUIRED` | 管理员首次登录必须修改初始密码 |
| `11004` | `ADMIN_CURRENT_PASSWORD_INVALID` | 管理员当前密码不正确 |
| `11005` | `ADMIN_NEW_PASSWORD_INVALID` | 管理员新密码不符合安全要求或与当前密码相同 |
| `11006` | `ADMIN_PASSWORD_UPDATE_CONFLICT` | 管理员身份 revision 已变化，必须重新登录后改密 |
| `12001` | `MEMBER_INVALID` | 成员输入或状态无效 |
| `12002` | `MEMBER_EMAIL_EXISTS` | 成员邮箱已存在 |
| `12003` | `MEMBER_NOT_FOUND` | 成员不存在 |
| `12010` | `MEMBER_SEAT_LIMIT_REACHED` | 会新增模型消费身份的事务超过许可证成员席位上限 |
| `12011` | `MEMBER_QUOTA_UPDATE_CONFLICT` | 并发操作已经更新成员余额或账本链，本次额度变更未生效 |
| `12012` | `MEMBER_QUOTA_GRANT_DUPLICATE` | 同一发放凭据已入账，拒绝重复增加额度 |
| `12013` | `MEMBER_STATUS_UPDATE_CONFLICT` | 成员状态或签名席位登记表已被并发更新，请刷新后重试 |
| `12014` | `ADMIN_CONSUMPTION_QUERY_INVALID` | 管理端团队消费日志的时间、成员、Key 或分页参数无效 |
| `12015` | `MEMBER_MODEL_ACCESS_CONFLICT` | 成员模型权限已被其他操作更新，请刷新后重试 |
| `12016` | `MEMBER_QUOTA_ADJUSTMENT_INVALID` | 额度变更内容无效，检查 Token、图片张数和原因 |
| `12017` | `MEMBER_MONEY_ADJUSTMENT_INVALID` | 金额或调整原因无效 |
| `21001` | `MEMBER_INVALID_CREDENTIALS` | 成员邮箱或密码错误 |
| `21002` | `MEMBER_UNAUTHENTICATED` | 成员会话不存在、过期或无效 |
| `21003` | `MEMBER_PASSWORD_CHANGE_REQUIRED` | 首次登录必须修改管理员设置的初始密码 |
| `21004` | `MEMBER_CURRENT_PASSWORD_INVALID` | 成员当前密码不正确 |
| `21005` | `MEMBER_NEW_PASSWORD_INVALID` | 成员新密码不符合安全要求或与当前密码相同 |
| `21006` | `MEMBER_PASSWORD_UPDATE_CONFLICT` | 成员身份 revision 已变化，必须重新登录后改密 |
| `22001` | `API_KEY_INVALID` | API Key 名称或请求无效 |
| `22002` | `API_KEY_NOT_FOUND` | API Key 不存在、已撤销或不属于当前成员 |
| `22003` | `API_KEY_UPDATE_CONFLICT` | API Key revision 已变化，拒绝过期状态更新 |
| `22004` | `API_KEY_LIMIT_REACHED` | 当前成员的 active API Key 已达到许可证上限；撤销旧 Key 后可释放额度 |
| `23001` | `MEMBER_QUOTA_REQUEST_INVALID` | 额度申请金额、原因、状态筛选或审批参数无效 |
| `23002` | `MEMBER_QUOTA_REQUEST_PENDING_EXISTS` | 同一成员已有一条待审批申请，必须等待处理后再提交 |
| `23003` | `MEMBER_QUOTA_REQUEST_NOT_FOUND` | 指定的额度申请不存在 |
| `23004` | `MEMBER_QUOTA_REQUEST_REVIEW_CONFLICT` | 申请已被其他管理员审批，或签名余额/账本 revision 已变化，本次审批未生效 |
| `23005` | `VOUCHER_INVALID` | 兑换券创建参数、领取请求或公开券码格式无效 |
| `23006` | `VOUCHER_NOT_FOUND` | 管理操作指定的兑换券不存在 |
| `23007` | `VOUCHER_UNAVAILABLE` | 兑换券已停用、过期、领完，或定向券不属于当前成员 |
| `23008` | `VOUCHER_ALREADY_REDEEMED` | 当前成员已经兑换过同一张券，拒绝重复入账 |
| `23009` | `VOUCHER_REDEEM_CONFLICT` | 兑换券领取次数、定向投放状态或签名额度账本已被并发更新，本次兑换未生效 |
| `23010` | `VOUCHER_DELETE_CONFLICT` | 兑换券已有兑换记录，必须保留额度账本审计链路，不能删除 |
| `24001` | `MEMBER_USAGE_QUERY_INVALID` | 用量时间范围、分页、API Key、模型或协议筛选参数无效 |

对外模型 API 的认证与路由错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `31001` | `INVALID_API_KEY` | 缺少 API Key、格式错误、摘要不匹配、Key/成员状态失效或完整性校验失败 |
| `32001` | `INVALID_REQUEST` | 对外模型 API 的查询或请求参数无效 |
| `32002` | `INVALID_COMPATIBILITY_MODE` | 兼容模式请求头的取值无效 |
| `33001` | `MODEL_NOT_FOUND` | 模型不存在、未开放或当前没有有效账号映射 |
| `33002` | `MODEL_ACCOUNT_NOT_READY` | 模型存在，但没有可解密、可刷新且身份匹配的凭据实例 |
| `33005` | `MODEL_ACCESS_DENIED` | 模型存在且已开放，但未授权给当前用户 |

对外模型 API 的额度预留与结算错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `34001` | `INSUFFICIENT_QUOTA` | 扣除其它在途请求的预留额度后，可用额度不足 |
| `34002` | `QUOTA_RESERVATION_CONFLICT` | 并发请求已经更新余额或预留 revision；本次预留未生效 |
| `34003` | `USAGE_SETTLEMENT_INVALID` | 上游用量字段、倍率或计算结果无效，拒绝写入账本 |
| `34004` | `USAGE_SETTLEMENT_CONFLICT` | 该预留已经释放、结算或被其它事务更新，拒绝重复结算 |
| `34005` | `DUPLICATE_REQUEST` | `request_id` 已用于预留或记账，拒绝重复消费 |
| `34006` | `BILLING_PRICE_UNAVAILABLE` | 该模型尚未配置有效价格 |
| `34007` | `BILLING_CALCULATION_INVALID` | 用量无法按已配置价格计算 |

对外模型 API 的上游结果错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `35001` | `INVALID_UPSTREAM_RESPONSE` | Runner 帧顺序、任务 ID、状态或响应内容无法验证 |
| `35002` | `UPSTREAM_REQUEST_FAILED` | Runner 已接单但上游传输或执行失败 |
| `35003` | `UPSTREAM_RESPONSE_TOO_LARGE` | 上游响应超过 Control 允许处理的上限 |
| `35004` | `UPSTREAM_USAGE_INVALID` | 完成事件缺少合法 usage，拒绝伪造或猜测结算值 |
| `35005` | `IMAGE_DELIVERY_FAILED` | 图片已生成，但下载或交付图片失败；按图片结算规则记录结果 |

当前已登记的管理端订阅/账号与凭据错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `13001` | `UPSTREAM_CREDENTIAL_DUPLICATE` | 该凭据的稳定私有指纹已存在；复制同一 refresh token 不会形成独立凭据实例 |
| `13002` | `UPSTREAM_CREDENTIAL_NOT_FOUND` | 指定的凭据实例不存在 |
| `13003` | `UPSTREAM_CREDENTIAL_REFRESH_CONFLICT` | 该实例的 revision 已变化，丢弃过期刷新结果，禁止覆盖新凭据 |
| `13004` | `UPSTREAM_CREDENTIAL_INVALID` | 凭据上下文、密文信封或解密结果无效 |
| `13005` | `UPSTREAM_ACCOUNT_NOT_FOUND` | 指定的订阅/账号不存在 |
| `13006` | `UPSTREAM_CREDENTIAL_REFRESH_BUSY` | 同一凭据实例已有刷新任务持有短租约；其它凭据实例不受影响 |
| `13007` | `UPSTREAM_CREDENTIAL_REFRESH_LEASE_INVALID` | 刷新提交使用的租约不匹配、已消费或已过期 |
| `13008` | `UPSTREAM_OAUTH_SESSION_INVALID` | OAuth 会话、回调地址、state 或有效期无效；重新发起授权即可 |
| `13009` | `UPSTREAM_OAUTH_EXCHANGE_FAILED` | 上游拒绝授权码兑换或返回的 Token/账号身份无效 |
| `13010` | `UPSTREAM_OAUTH_SESSION_BUSY` | 同一 OAuth 会话正在完成，拒绝并发重复兑换授权码 |
| `13011` | `UPSTREAM_MODEL_SYNC_FAILED` | 上游模型接口失败、响应无效或没有返回可用模型 |
| `13012` | `UPSTREAM_ACCOUNT_INPUT_INVALID` | 订阅/账号启停或删除请求参数无效 |
| `13013` | `UPSTREAM_MODEL_NOT_FOUND` | 管理操作指定的上游模型不存在 |
| `13014` | `UPSTREAM_MODEL_INPUT_INVALID` | 上游模型启停请求参数或资源标识无效 |
| `13015` | `UPSTREAM_ACCOUNT_LIMIT_REACHED` | 已创建逻辑订阅/账号达到许可证上限；禁用或失效仍占用，删除后释放 |
| `13016` | `UPSTREAM_CONNECTION_REVISION_CONFLICT` | API Key 连接的 revision 已变化；刷新列表后重试修改或换 Key |
| `13017` | `UPSTREAM_PLUGIN_UNAVAILABLE` | 上游适配插件未配置或未激活；完成配置与激活后再操作连接或调用网关 |

当前已登记的 Runner 与路由错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `33003` | `RUNNER_NOT_READY` | 当前没有满足健康、容量和协议条件的 Runner |
| `33004` | `RUNNER_RETRY_FORBIDDEN` | 上游已经接收或已经输出流内容，禁止无感切换 Runner |
| `41001` | `RUNNER_PROTOCOL_INCOMPATIBLE` | Runner 与 Control 的协议版本不兼容 |
| `41002` | `RUNNER_TASK_TICKET_INVALID` | 单次任务票据的签名、绑定、摘要或有效期无效 |
| `41003` | `RUNNER_TASK_REPLAYED` | 单次任务票据已经消费，拒绝重放 |
| `41004` | `RUNNER_UNAUTHENTICATED` | Runner 凭据无效、已停用或握手身份不一致 |

当前已登记的管理端 Runner 错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `14001` | `RUNNER_ENROLLMENT_INVALID` | Runner 注册 Token 不存在、已过期或已使用；同一 Token 只能成功注册一次 |
| `14002` | `RUNNER_NOT_FOUND` | 管理操作指定的 Runner 不存在 |
| `14003` | `RUNNER_INPUT_INVALID` | Runner 名称、版本、平台、架构、协议版本或并发数无效 |
| `14004` | `RUNNER_LIMIT_REACHED` | 已创建 Runner 达到许可证上限；禁用或离线仍占用，删除后释放 |

当前已登记的平台配置错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `15001` | `PLATFORM_SETTINGS_INVALID` | 公共 API 地址或用量结算倍率无效 |
| `15002` | `PLATFORM_SETTINGS_UPDATE_CONFLICT` | 运行配置 revision 已被其他管理员更新，本次保存未生效 |
| `15003` | `PLATFORM_AUDIT_QUERY_INVALID` | 安全审计的分页、执行人、动作或结果筛选参数无效 |
| `15004` | `BILLING_PRICE_SYNC_UNSUPPORTED` | 该公开模型没有可可靠映射的官方标准价或内置参考价，需由管理员手动配置 |

当前已登记的系统升级与维护错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `15010` | `MAINTENANCE_INPUT_INVALID` | 上传的升级包、待删除版本或维护参数无效 |
| `15011` | `MAINTENANCE_BUSY` | 已有升级或版本清理任务等待执行或正在运行 |
| `15012` | `MAINTENANCE_UNAVAILABLE` | 主机升级队列或维护状态暂时不可用 |

当前已登记的 Operations 发布中心错误：

| 数值码 | 字符串码 | 含义 |
|---|---|---|
| `67001` | `RELEASE_PERMISSION_DENIED` | 当前运营账号没有 `release.read` 等所需发布中心权限 |
| `67002` | `RELEASE_PERMISSION_CHECK_FAILED` | 发布中心权限数据读取失败，拒绝按默认授权放行 |
| `67003` | `RELEASE_TASKS_LIST_FAILED` | 发布任务列表读取失败 |
| `67004` | `RELEASE_TASK_DETAIL_FAILED` | 指定发布任务的 Run、Job、Step 或产物详情读取失败 |
| `67005` | `RELEASE_TASK_INPUT_INVALID` | 发布任务 SemVer 或来源分支/commit 参数无效 |
| `67101` | `RELEASE_ORCHESTRATOR_UNAVAILABLE` | Operations 尚未配置完整的 GitHub App 发布编排 |
| `67102` | `RELEASE_DUPLICATE_ACTIVE` | 同版本、同来源 commit 已有未结束任务，拒绝重复触发 |
| `67103` | `RELEASE_TASK_CREATE_FAILED` | 发布任务创建或本地持久化失败 |
| `67104` | `RELEASE_RETRY_FAILED` | 失败任务重试创建失败 |
| `67201` | `GITHUB_PREFLIGHT_FAILED` | GitHub 来源、版本、Workflow、Environment Secret 或 Variable 预检失败 |
| `67202` | `GITHUB_DISPATCH_FAILED` | 固定 GitHub Workflow dispatch 失败 |
| `67203` | `GITHUB_SYNC_FAILED` | 绑定的 GitHub Run、Job、Step、日志或 Artifact 状态同步失败 |
| `67204` | `GITHUB_WORKFLOW_FAILED` | GitHub Workflow 以失败、取消或超时结论结束 |
| `67301` | `RELEASE_BUILD_FAILED` | Customer Release 构建、依赖审计、签名或打包阶段失败 |
| `67401` | `RELEASE_LINUX_VALIDATION_FAILED` | Linux Customer/Runner 安装、升级、恢复或回滚验收失败 |
| `67501` | `RELEASE_ARTIFACT_UNAVAILABLE` | GitHub Run 成功结束后仍未产生唯一、未过期的 Customer 安装包 Artifact |
| `67502` | `RELEASE_ARTIFACT_HASH_MISMATCH` | GitHub ZIP digest、安装包 SHA-256 或随包 `.sha256` 任一不一致 |
| `67503` | `RELEASE_PACKAGE_POLICY_FAILED` | ZIP/tar 路径、类型、权限、大小、完整文件树或交付边界不符合规则 |
| `67504` | `RELEASE_MANIFEST_INVALID` | `RELEASE.json` 不是严格的 `aster.release-manifest.v1` 文档或清单字段无效 |
| `67505` | `RELEASE_SIGNATURE_INVALID` | `RELEASE.json` 的 `key_id` 不在 Operations 独立公钥环内或 Ed25519 签名无效 |
| `67506` | `RELEASE_SBOM_INVALID` | 包内 CycloneDX 1.6 SBOM 缺失、格式无效或版本身份不匹配 |
| `67507` | `RELEASE_ABI_INCOMPATIBLE` | 必需 ELF 二进制不是 linux/amd64 musl 全静态程序，或仍含动态解释器/共享库依赖 |
| `67508` | `RELEASE_DOWNLOAD_FAILED` | Operations 内容寻址对象不存在、摘要损坏、大小不符或无法流式下载 |
| `67601` | `RELEASE_PUBLISHER_UNAVAILABLE` | Operations 尚未配置只拥有 Contents write 的独立 GitHub Publish App |
| `67602` | `RELEASE_PUBLISH_INPUT_INVALID` | 正式发布申请、审批或执行输入无效，或目标不是已独立复核的不可变发布物 |
| `67603` | `RELEASE_PUBLISH_REQUEST_FAILED` | 正式发布申请写入失败 |
| `67604` | `RELEASE_PUBLISH_DUPLICATE` | 相同版本/Tag 已有未结束或已发布的申请，拒绝重复发布 |
| `67605` | `RELEASE_PUBLISH_APPROVAL_FAILED` | 审批决定不符合状态机或未能写入追加审批记录 |
| `67606` | `RELEASE_PUBLISH_SELF_APPROVAL_FORBIDDEN` | 申请人与审批人相同，拒绝自审 |
| `67607` | `RELEASE_PUBLISH_PROVENANCE_INVALID` | 发布物版本、来源 commit、对象摘要或签名复核来源不完整或发生变化 |
| `67608` | `GITHUB_PUBLISH_FAILED` | 创建/核对受保护 Tag、草稿 Release、不可覆盖资产或公开 Release 失败 |
| `67609` | `RELEASE_PUBLISH_REQUESTS_LIST_FAILED` | 正式发布申请台账读取失败 |

| `67701` | `COMMERCIAL_PERMISSION_DENIED` | 已登录运营账号缺少相应套餐或订单读写权限 |
| `67702` | `COMMERCIAL_SNAPSHOT_CONFLICT` | 套餐版本已变化或操作标识对应其他内容；核对最新版本后再继续 |
| `67703` | `COMMERCIAL_PLAN_VERSION_CONFLICT` | 事务内确认本操作未保存且套餐版本已变化；可比较后以新基线保存 |
| `67704` | `COMMERCIAL_DISTRIBUTION_FAILED` | 免费分发记录、来源完整性或签发处理失败；保留记录并重新核对，不以当前套餐替换已批准内容 |
| `67705` | `COMMERCIAL_DISTRIBUTION_NOT_ISSUED` | 分发尚未完成签发，当前不能下载授权文件 |
| `67706` | `COMMERCIAL_V2_SIGNER_UNAVAILABLE` | 本次仍需签名但对应受限 v2 私钥不可用；不会回退旧 v1 密钥。已签发读取使用独立可信公钥 |
| `67707` | `COMMERCIAL_DRAFT_FAILED` | 套餐草稿读取、保存或生成固定版本失败；保留待确认操作并核对原请求 |
| `67708` | `COMMERCIAL_DRAFT_REVISION_CONFLICT` | 事务内确认本操作未提交且草稿当前修订已改变；比较最新修订后明确保留配置，再保存或生成版本 |
| `67709` | `COMMERCIAL_CATALOG_FAILED` | 公开目录预览、批准、读取或导出失败；保留原操作，核对批准记录和实际文件后重试 |
| `67710` | `COMMERCIAL_CATALOG_PREVIEW_CONFLICT` | 本次批准尚未保存，来源版本或公开摘要与核对内容不符；重新预览后明确批准 |
| `67711` | `COMMERCIAL_CATALOG_EXPORT_UNAVAILABLE` | 运营后端未配置公开目录根路径；不接受浏览器传入任意输出路径 |
| `67712` | `COMMERCIAL_CATALOG_NOT_EXPORTED` | 目录已批准但尚未记录本地导出，不能直接下载；导出不代表官网部署或上架 |
| `67713` | `COMMERCIAL_PUBLICATION_FAILED` | 发布候选、核对记录或受理操作失败；先读取原记录确认实际结果 |
| `67714` | `COMMERCIAL_PUBLICATION_UNAVAILABLE` | 对应环境未配置受控核对源；浏览器不能替换目标地址 |
| `67715` | `COMMERCIAL_PUBLICATION_UNVERIFIED` | 目标字节或证据未通过核对；保留原候选，读取失败历史后排查 |
| `67716` | `COMMERCIAL_PUBLICATION_PREPARE_REJECTED` | 服务端确认原请求未保存且基准或期限已变化；重新读取、选择清单并确认后建立新请求 |
| `67717` | `COMMERCIAL_PUBLICATION_FAILURE_NOT_RECORDED` | 本次故障记录未能持久保存；不能据此推断批准未提交，应读取原发布记录确认 |

| `67718` | `COMMERCIAL_FULFILLMENT_FAILED` | 付费履约批准、签发或读取失败；保留原请求和记录，先核对真实来源与状态 |
| `67719` | `COMMERCIAL_FULFILLMENT_NOT_ISSUED` | 付费履约尚未完成签发，当前不能下载授权文件 |
| `67720` | `COMMERCIAL_V2_VERIFIER_UNAVAILABLE` | 缺少原证书对应的可信公钥，无法验签或提供原文件；不能换 key 或跳过核验 |
| `67721` | `COMMERCIAL_CUSTOMER_REF_UNAVAILABLE` | 新批准需要的客户引用派生能力不可用；原操作恢复不会重新派生引用 |
| `67722` | `COMMERCIAL_FULFILLMENT_ENVIRONMENT` | 未配置可信履约环境，或当前环境与原批准、订单来源不一致；不推进签发 |
| `67723` | `COMMERCIAL_FULFILLMENT_TRANSFER_NOT_ISSUED` | 换机授权尚未完成签发，当前不能下载授权文件 |

模块内的字符串码和数值码必须在代码目录中一对一登记。新增已知错误时先分配下一个未使用序号并更新本文，不得按数组排序变化重新编号，也不得复用已经发布的号码。禁止用 `xx999`、时间戳或随机数字临时顶替目录登记；未登记的程序错误只能归一为已经发布的内部错误，发布检查必须阻止已知业务错误漏入目录。

## 错误码与请求追踪号

业务错误码和单次请求追踪号是两种不同数据：

- 业务错误码回答“发生了哪一类错误”，稳定且可写入帮助文档。
- 请求追踪号回答“是哪一次请求”，可以唯一生成并写入日志，但字段必须命名为 `request_id` 或 `trace_id`，不能命名或展示为“错误码”。

用户反馈问题时优先提供五位业务错误码、发生时间和操作入口。系统将来增加请求追踪号后，可以再提供 `request_id` 精确关联日志；不得为了日志关联而把随机长数字重新塞入 `number`。

对 OpenAI 和 Anthropic 协议兼容接口，`number` 位于各自标准的 `error` 对象中，并写入同一个响应头。

### 环境升级

| 错误号 | 代码 | 说明 |
| --- | --- | --- |
| `67801` | `ENVIRONMENT_UPGRADE_FAILED` | 环境配置、目标能力、探测或升级执行失败；核对目标安装身份、授权、制品与后台任务状态。 |
