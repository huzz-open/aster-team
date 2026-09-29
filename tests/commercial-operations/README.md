# 商业运营浏览器验证

使用实际 Operations API、独立 MariaDB 库和运营前端生产构建。只监听本机，不部署官网或访问 GitHub 发布接口；临时签名密钥仅为启动隔离 API，不产生正式 License。

从仓库根目录执行，前提是 Node、项目依赖、仓库 Go/Rust 工具链、Docker 与 Playwright Chromium 可用。端口 26216、26380、26390、26394 应为空闲；不要复用日常运营 API 或数据库。

## 启动独立环境

```powershell
docker run --rm --detach --name aster-commercial-ui-test --publish 127.0.0.1:26216:3306 --env MARIADB_ROOT_PASSWORD=aster-test-only --env MARIADB_DATABASE=aster_contracts_test mariadb:11.8.6
docker exec aster-commercial-ui-test healthcheck.sh --connect --innodb_initialized
node tests/commercial-operations/prepare-local.mjs
node scripts/run-go.mjs build -o dist/commercial-validation/operations-api.exe ./operations/backend/cmd/api
node tests/commercial-operations/serve-api.mjs
```

等待数据库 healthcheck 成功后启动 API。最后一条持续运行，在另一终端运行测试。`serve-api.mjs` 排除继承的 Operations 环境变量并核对本地库名和地址，避免采用其他运营配置；配置和临时密钥位于忽略的 `dist/commercial-validation/`。

API 单独创建 `aster_commercial_ui_test`。存储层的 `ASTER_COMMERCIAL_TEST_ADDR` 测试使用同容器里的 `aster_contracts_test`，包括故意损坏记录的反例，不得把这两个库合并。

## 执行

```powershell
npm run build --workspace @aster/operations-console
$env:ASTER_COMMERCIAL_BROWSER_TEST = '1'
npm run test:commercial:browser
```

测试自动启动和关闭 26380 端口的生产构建预览；拒绝复用已存在的 Web 服务。账号、客户、套餐和订单均为隔离示例数据。覆盖：

- 一键本地授权调用同一真实商业 API：模拟签发已提交但回包丢失后重试，保持单一订单及原签名字节，下载交给 Rust 验签。申请摘要变化和下载篡改拒绝；模拟 production/空/异常环境时只允许前置只读环境请求，没有业务写入。真实接口的环境来自服务端配置，并要求会话与签发权限。
- 真实并发修订产生 409，比较最新版本后明确保留输入，保存下一版本；原版本摘要不变。
- 服务端草稿并发编辑产生 67708，明确比较后保存 r3；历史 r1 保留且草稿尚不产生套餐版本。生成固定版本成功后丢弃响应，另存 r4，再刷新以原请求和 r3 恢复，只有原固定版本生效。
- 真正创建订单后丢弃响应，再注入一次 403，刷新页面并以原操作 ID 重试；数据库只保存一张订单，金额、权益与日期保持固定。
- 按客户咨询的目录编号精确读取已核对的报价来源并创建 v2 订单；保存后丢回包、临时 403 和刷新后恢复同一订单，核对原发布、套餐摘要和成交金额。实际 API 拒绝客户端环境覆盖与不同内容复用请求 ID，截图覆盖桌面与小屏。配置仅允许本地受理环境。
- 实际打开该次批准目录构建的官网，读取卡片的 revision、套餐版本和三年期限，核对显示金额及公开文件摘要。原报价订单继续到账、批准和签发，下载原始 License 交给 Rust 生产 `v2::verify`；验签后声明必须与原订单 ID、展示权益、日期及安装绑定一致，改动套餐版本的原签名文件必须拒绝。证据链与官网卡片截图保存在该用例输出目录，不使用新建手工订单代替。
- 批准固定免费套餐后丢弃响应、注入 403 并刷新，以原操作继续；每次重新输入当前密码，恢复记录不包含密码。批准后修订套餐，签发仍引用原版本。
- 用独立临时 v2 密钥真正签发测试免费证书，丢弃签发响应后读取服务器状态并下载文件；响应符合严格 schema，文件 SHA-256 和 Ed25519 签名验证通过，重复签发不改变内容。
- 选择和排序免费、年度价格、联系报价三种固定套餐，预览按文本显示说明；公开批准成功后丢弃响应，刷新后读取原记录，不重复批准。随后修订套餐仍保留批准时的版本和金额。
- 当前密码验证失败后可重试；本地导出成功后丢弃响应，再次导出同一目录并下载，核对运营主机实际文件、版本、SHA-256 和严格公开 schema。公开文件的额外字段与错误报价结构由本地 schema 反例拒绝；错误预览摘要由实际 API 返回 `409/67710`，空目录保持空数组。
- 全额到账先拒绝错误密码，真正保存后丢回包，再注入临时 403 并刷新恢复；原订单、金额、凭据及确认时间保持一致，密码不进入恢复记录。原操作改内容返回冲突，客户端覆盖金额拒绝。
- 从真实订单和全额到账进入付费批准，逐字节保留严格 v2 安装请求；批准与签发分别在提交成功后丢失响应并经刷新恢复同一操作和固定密钥。下载文件的严格 schema、SHA-256、Ed25519 签名、安装绑定、原套餐权益和合同日期均通过核对，原订单最终进入已履约状态。
- 桌面和小屏页面实际操作及截图。
- 从真实批准并导出的 Operations 目录字节构建官网，临时静态站点仅监听 26394；生产核对目标保持未配置。发布候选保存与批准回包丢失后恢复原 ID，实际 JS 字节被修改时拒绝核对并持久记录失败，刷新和之后成功仍保留该失败历史。
- 未保存请求因并发发布失去基准后，刷新重试取得服务端明确拒绝，页面保留目录、说明与期限；已保存候选发生基准冲突时，保留原候选及失败记录，明确确认后建立新 ID。批准结果未知时不能绕过原记录核对建立新候选。

重试记录按运营账号与标签页保存在 `sessionStorage`，覆盖刷新和路由切换，不跨关闭标签页承诺保留原请求；成功保存的草稿存于 MariaDB。免费和付费测试分别使用临时受限密钥及明确标识的示例套餐；不会签发正式证书或修改真实签发配置。公开目录仅导出至忽略的 `dist/commercial-validation/public-catalogs/local/`，实际官网构建位于 `dist/commercial-validation/publication-sites/`，临时站点在用例结束后关闭。人工全额到账使用隔离订单和示例凭据，不发生实际收款。完整官网、生产激活、正式发行及 Customer 安装不在本组测试的已验证范围。

同一数据库重跑时沿用已创建的临时密钥配置；只需重建并重启 API 时不要重新运行 `prepare-local.mjs`，否则旧签发记录会因密钥改变而无法验证。重建整个环境时同时使用新的隔离数据库和临时配置，不把测试密钥带入其他环境。

截图位于 `dist/commercial-validation/screenshots/`；失败结果和 trace 位于 `dist/commercial-validation/browser-results/`。这组验证不能替代仓库完整检查或正式包安装验收。

`npm run test:commercial:browser` 先构建 `aster-license-core` 的 `verify_issued_license` example，再运行原 Playwright 集合；命令参数继续传给 Playwright。Rust 验证器只接受本次隔离 API 的公开公钥/签发范围和实际下载字节，不读取生产密钥，不随 Customer 包交付，不安装或激活授权。它证明签名及权益声明的跨语言交接；运行时有效期、机器与业务执行由已有 Customer/系统测试验证。

## 清理

在 API 终端按 Ctrl+C 结束服务，再删除本测试创建的容器；没有挂载宿主机数据卷。

```powershell
docker rm -f aster-commercial-ui-test
```
