# 授权交付浏览器验证

在仓库根目录运行。需要已安装的 npm 依赖和 Chromium；缺少浏览器时使用 `npm run setup:delivery-tests`。

## 安装请求传输

`npm run test:license-delivery:browser` 在真实浏览器运行运营端的 v2 文件读取与二维码解码模块，覆盖 JSON 原文、二维码 PNG 和终端截图。旧内部 v1 签发页面及其模拟签发测试已移除；该命令不代表真实签名交付。付费批准、签发和 Rust 验签由 `npm run test:commercial:browser` 及下方 Rust 后端浏览器流程单独验证。

## 成员授权提示与恢复

```powershell
npm run test:license-delivery:member
```

命令先构建真实 Member 前端，再通过本地 `127.0.0.1:26481` 预览运行四条浏览器用例。端口必须空闲，测试不复用已有服务，结束后由 Playwright 关闭预览。

- 1366×648 和 390×720 下，只有 Runner 权益的有效授权进入成员功能缺失页，重新检查不会反复跳转登录。
- 授权变为不可用时更新提示；恢复成员权益后进入正常认证流程。
- 旧 Control 响应缺少 `features` 时保留原有成员认证兼容行为。
- 重新检查遇到网络错误时保留当前页面，显示错误并允许重试。

HTTP 响应是显式的测试 fixture，只隔离前端路由。它不验证签名、不证明业务扩权或免费安装成功。真实签名导入、机器绑定和成员业务拒绝由 Rust Control 测试覆盖；完整安装包仍需发行级检查。

截图和失败 trace 保存在忽略的 `dist/license-currentness/browser-results/`，包括两个视口的 `member-unlicensed-*.png`。这些是成员端行为截图，不是官网效果图或官网验收。

## 管理端授权故障恢复

```powershell
npm run test:license-delivery:admin
```

构建真实 Admin 前端并在 `127.0.0.1:26482` 运行，端口独占且结束后关闭。九条用例覆盖桌面/手机在状态读取失败时仍能上传、同一文件失败后重试成功、过期恢复不误报激活、保存后读回失败时清除旧有效状态，以及首次安装的正常激活流程。上传区、CLI 回退和全局授权提示一起核对。

其中三条权益页面用例核对只有 Runner 功能时，未授权页面不会挂载或请求业务接口；同标签页导航会更新功能投影，授权不可用时仍能进入恢复页；兑换券接收人勾选和消费日志成员筛选使用窄查询，不请求完整用户或设置接口。HTTP fixture 只证明前端使用权益快照的行为，不能替代 V08–V10 的真实最小签名授权、服务层执行与完整入口验收。

Control 的 `tests/admin_projections.rs` 使用真实 v2 签名、授权历史、SQLite 身份与会话，核对最小功能许可、首次改密、角色和伪造上下文拒绝，以及严格响应字段。接收人只列启用成员并与实际投放接口一致；历史筛选保留停用成员、排除已删除成员与管理员。成员身份被直接改库时查询失败，不返回未验证标签。

HTTP fixture 不执行签发或激活真实产品。真实文件重命名失败、跨到期精确字节重试、历史拒绝及成员额度路径由 Control 的 sqlite-dev 测试覆盖；license-state 测试另验证更新各阶段中断、独立进程锁释放和恢复。Windows 文件占用及临时属性测试仅在 Windows 运行，不视作 Linux 安装或系统掉电实测。

截图和失败 trace 位于忽略的 `dist/license-recovery/browser-results/`。这些是产品恢复流程截图，不是官网验收。

## 真实最小授权联动

```powershell
npm run test:license-delivery:real
```

命令构建生产 Admin/Member 前端及独立 Rust `license_browser_fixture`，然后执行浏览器用例。每条用例启动新的真实 Control、SQLite 数据库、签名 License、授权历史和管理员；服务仅绑定 `127.0.0.1` 的系统分配端口，不复用已有服务或数据库，不使用 HTTP 响应拦截。管理员及成员均通过实际页面登录和首次改密。

测试证书使用公开测试种子和向量，不是正式证书；免费与付费使用独立的受限测试签发者。固定时间为 `2026-09-07T00:00:00Z`，数据库、License 和业务时间保持一致；`member`、`runner`、`gateway` 及空能力场景分别使用明确的测试权益，不决定正式套餐。初始免费证书经过实际文件安装与授权历史流程，后续导入走生产 HTTP 接口。专用 binary 必须同时启用 `local-demo` 与 `sqlite-dev`，正式 Customer 的 `--no-default-features --features sqlcipher` 构建不包含它。

- 成员能力：实际创建成员、辅助选择器投放兑换券、消费日志筛选、成员登录改密、创建两枚 Key 和第三枚越限拒绝。
- 成员范围：两个真实成员分别持有 Key，伪造身份查询或撤销别人的 Key 被隔离；原 Key 保持有效，本人撤销后可创建替代 Key。
- Runner 能力：节点列表及专用连接信息查询无需完整设置或成员能力；实际生成 Token、注册、停用仍占额度、删除释放额度、超限不消耗待用 Token。已消费 Token 在腾出额度后仍以明确错误拒绝重用。
- Gateway 能力：模型页面、上游账号和提供方查询可独立读取；缺少 Runner 时新增账号按钮提前禁用并说明共享能力要求，绕过页面直接发起接入也以 51008 拒绝，不让用户先登录第三方再失败。
- 空能力及反例：页面锁定、无关能力请求拒绝、伪造业务上下文拒绝、成员会话及改名会话不能访问管理员接口。
- 授权切换：通过页面选择实际签名文件并导入，从免绑定免费证书切换到绑定付费证书；保持已有管理员与成员会话，验证新增功能与更高 Key 额度、原成员和 Key 保留。篡改证书以及重导入旧免费证书的反例必须保留当前授权和数据。这不定义主动商业降级规则。
- 支撑页面与账号维护：`real-support.spec.mjs` 使用 member-only 签名权益及预先存储的模型、账号和加密凭据，实际操作模型搜索/表格/厂商筛选、零使用状态的 Key/日期筛选、账户资料与余额、配置持久化及成员接入地址、直接发放与历史、停用再启用、重置密码和两端主动改密。改密前的 Cookie 必须先可用、改密后由独立请求证实失效，旧密码返回精确认证错误；余额和原 Key 保留。安全审计页面按动作和结果筛选真实事件。该用例不伪造用量或响应，不代表非零模型趋势筛选已验证。

父进程通过 stdin 请求服务器正常关闭；启动失败、异常退出或超时强杀均以失败退出，等待有界。每条测试使用专有临时目录，关闭后回收数据库和就绪文件。截图、失败 trace 和服务端输出位于 `dist/license-real/browser-results/`。命令以明确退出码报告结果，不把人工截图审阅作为必要门槛。

上述用例不证明完整 V08–V10：Runner 注册后的实际执行、Gateway 维护写入、所有业务入口及其他授权切换场景仍需继续扩展，也不替代真实安装包验收或正式发行。Runner 注册用例调用真实注册接口，但没有启动实际 Runner 进程或执行外部模型请求。

## Windows 实际客户端工具下载

```powershell
npm run test:license-delivery:downloads:windows
```

要求原生 Windows x64、Visual Studio C++ Build Tools、Rust、Node 与 Chromium。命令调用仓库的 `build-asterctl-windows.mjs` 从当前源码构建静态 CRT 的真实 `asterctl.exe`，然后构建 Admin/Member 与隔离 Control。此项有明确的平台要求，独立于跨平台的 `test:license-delivery:real`；缺少环境或制品时失败，不跳过。

测试在专属临时安装目录中放置真实工具和临时 Ed25519 签名的 `RELEASE.json`，通过 `current` junction 使用生产安装目录解析、清单验签、文件哈希和流式下载路径。临时公钥仅交给 `local-demo` 测试进程，私钥只在父进程内存中存在，显式清除继承的开发制品兜底环境变量。该目录只提供下载所需制品，不是完整 Customer 发行包。

- 真实 member-only 授权下完成管理员/成员登录和首次改密，打开接入文档与客户端配置。
- 从主下载按钮和平台版本表格下载，核对清单元数据、响应头、完整字节的 SHA-256 和长度，实际执行下载文件的 `--version`。不执行安装或修改用户的 Codex/Claude 配置。
- 匿名和管理员会话不能下载成员制品；未知制品返回 404。保持文件长度不变的篡改及未重新签名的清单修改均应拒绝，恢复后重新下载必须与原摘要一致。
- 导入不含 member 权益的有效付费测试证书后，存量成员会话读取文档、制品清单与下载均应返回功能未授权，页面进入权益提示。此场景不定义商业主动降级政策。

构建制品及浏览器证据位于忽略的 `dist/license-downloads/`。测试进程正常关闭后清除专属安装目录、测试证书和数据库；不修改生产信任配置，不使用正式签名，不访问供应商服务，不发布 Release 或部署官网。

## 最小授权的服务端业务链

```powershell
cargo test --locked -p aster-control --no-default-features --features sqlite-dev --lib minimal_ -j 1 -- --test-threads=1
```

这组测试补充上述浏览器范围，使用隔离 SQLite、实际管理员登录与首次改密、验签后的 v2 License、授权历史和加密凭据。套餐数值及签名种子均为测试数据，不定义正式商业权益。

- `gateway + runner` 且不含 `member`：通过实际 HTTP 接入账号、同步模型与人工刷新凭据；检查持久账号、两个模型及路由、凭据修订号与新 Token、刷新租约释放，以及具有实际操作者和目标的完整审计链。第二个不同账号仍以签名额度错误 `13015` 拒绝。
- 仅 `gateway`：预置存量账号、加密凭据和模型，通过实际 HTTP 读取元数据、启停账号和模型、删除账号；核对数据库结果及审计。同步和人工刷新对真实资源仍返回 `51008`，不修改凭据、占用刷新租约或记录成功审计。
- 同一过滤命令也覆盖已有最小成员与 Runner 辅助查询用例。缺少组合能力时提前拒绝的完整反例另由 `upstream_external_operations_reject_incomplete_rights_before_preparation` 覆盖。

OAuth、模型发现及刷新任务经过真实派发与任务签名校验，但响应由 Runner Hub 测试夹具提供。等待任务和 HTTP 完成有超时上限。本组不启动真实 Runner 进程，不访问第三方网络，不验证 Runner 的 HTTP/TLS 执行，也不替代浏览器、正式安装包或完整 V08–V10 验收。

## 既有授权交付流程

```powershell
npm run test:license-delivery:api
npm run test:license-delivery:browser
```

既有浏览器流程使用 Operations 和 Admin 的隔离 HTTP fixture，覆盖授权申请传输及导入交互。`npm run test:license-delivery` 依次执行 API、传输、既有浏览器、成员端、管理端和真实最小授权浏览器检查。任何一组通过均不能替代 Linux/Windows 实际包安装、跨机恢复或正式发行验证。
