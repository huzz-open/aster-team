# System E2E

这套测试从空白 Docker 卷开始创建 Operations、MariaDB、上游模拟服务和三台 Customer Linux 节点，并由 Windows 或 Linux 主机上的 Playwright 浏览器完成真实页面操作。测试只调用正式二进制、正式 HTTP 接口和正式 UI，不依赖 Agent，也不在产品代码中加入测试后门。

## 固定流程

1. 生成临时 Release/License 测试密钥，构建并验收全新的 Customer Linux 安装包，同时生成明确标记为 `test-build` 的 `aster.release-candidate.v1` 清单并重新校验全部摘要。
2. 构建 Operations Linux 安装包，创建全新的 MariaDB 和 Operations 环境。
3. 创建三台全新的 Customer Linux 节点并安装 Control、Runner、Admin、Member 和 Model API。
4. 核对三台节点自动装载的免绑定 v2 免费测试证书，分别使用申请 JSON、QR PNG、终端二维码截图进入 Operations 的付费履约流程；每条路径均创建独立固定套餐版本与订单、记录测试到账、批准并签发，回到 Customer 上传。校验下载摘要、导入 HTTP 结果和实际生效的证书身份、安装绑定、功能及额度，不能仅凭原本就存在的“有效”状态判断成功。
5. Mock 模式分别签发 `member + runner`、`gateway`、`gateway + runner` 最小组合。前两台在免费测试授权下预先接入上游；仅 gateway 通过真实页面启停账号与模型、删除账号，同时拒绝同步与刷新 API；仅 gateway + runner 通过真实 Runner 完成 OAuth、同步与人工刷新 API 且拒绝成员管理。现有页面没有独立凭据刷新入口，测试不为此新增功能。三台均从系统升级页面核对候选包版本、真实已安装版本和初始空任务，当前版本不能删除，未选包不能升级。主节点在不具备 gateway 管理能力的付费测试授权下，创建成员、首次登录、申请和审批额度并创建 API Key，继续消费已有上游。live 模式三台使用完整能力并由人工完成主节点上游登录。
6. 通过真实 Model API 和 Runner 验证 OpenAI Models、Responses、Chat Completions、Images，以及 Anthropic Models/Messages；覆盖模型速度/推理后缀、各协议原生执行参数、冲突参数前置拒绝、JSON、消息数组、流式、PNG/JPEG/WebP、多图片编辑和 `n > 1` 聚合结算。
7. 在 Member 和 Admin 页面确认真实用量记录；Mock 模式使用真实 Runner 结算后的数据，通过 Member 用量页选择 Key、30 天和不同文本/图像模型，检查请求筛选条件、357 Token / 19 次成功汇总和图表浮层实际值，切回全部模型核对日汇总。测试结束后删除容器、网络、数据库卷和临时私钥。

默认的 `fake` 上游是协议级确定性服务，OAuth 登录也由 Playwright 自动完成，因此整条链路可无人值守反复执行：

```bash
npm run test:system-e2e
```

严格 Mock 使用每次运行随机控制凭证和显式 case ID，校验 OAuth state、PKCE S256、Token 表单、鉴权 Header、Codex 请求字段和 SSE。失败矩阵包含 400、401、403、404、409、429、5xx、非法 JSON/SSE、即时断连与延迟断连；保留的 `timeout` case 名称实际表示 1500 ms 后断开连接，不代表请求截止时间验收。每种失败均断言精确状态与错误编号、单次上游调用、请求标识和没有隐式 Token 刷新。case 不使用全局“下一次响应”，不会被并发请求串走。

确定性计费预期为 19 条成功、11 条失败，原始及计费 tokens 均为 357；失败记录不扣量，并按请求标识核对 5 条执行结果未知的审计。候选使用 `upstream-provider-e2e.v3` 测试矩阵，Runner 协议要求从 schema 读取，辅助测试核对其与 Rust 协议常量一致。以上套餐、价格、有效期、密钥和额度均为隔离测试数据，不定义正式产品规则；Linux lab 构建的测试包不能作为正式交付包。

同一候选制品可并行启动两套完整环境，验证 Compose project、网络、volume、端口、身份、运行目录和浏览器产物互不影响：

```bash
npm run test:system-e2e:parallel
```

调试时可显示浏览器，或保留失败现场：

```bash
npm run test:system-e2e:headed
node ./tests/system-e2e/run.mjs --headed --keep
```

真实上游模式只有“在弹出的上游页面完成登录”需要人工操作；浏览器检测到 OAuth 回调后，会自动继续成员、额度和 API 协议矩阵：

```bash
npm run test:system-e2e:live
```

live 模式禁止 `--keep`，成功或失败都会销毁浏览器状态、数据库、volume、Aster Key 和测试凭据。它在本地生成绑定候选清单 SHA-256 的 `live-result.json` 与人工边界检查单；人工只负责登录、MFA 和上游风控。账号登录或必需能力无法满足时记为 `BLOCKED`，网络或外部环境无法得出可靠结论时记为 `INCONCLUSIVE`，产品或契约断言失败时记为 `FAILED`；三者均以非零状态退出。

对已经由发布流程构建的候选执行测试时，必须同时提供清单和匹配候选许可证信任根的临时私钥。运行器只验证并安装清单中的制品，不会重新构建：

```bash
ASTER_E2E_V2_SIGNERS_FILE=/secure/path/v2-signers.json \
  node ./tests/system-e2e/run.mjs --candidate-manifest=/secure/path/candidate-manifest.json
```

`test-build` 与正式 `release-candidate` 通过清单的 `trust` 字段明确区分，临时构建不能冒充正式候选。摘要不匹配、制品缺失或构建后被覆盖时会在创建容器前失败。

v2 signer 文件必须包含恰好一个与候选信任范围匹配的隔离付费 signer（`key_id`、`private_key_pkcs8` 和 `policy`）。候选还须包含本流程所需的免费 v2 初始授权。不得提供生产私钥。外部 `--candidate-manifest` 不支持 `--keep`，因此也不能通过 `--reuse-run` 复用；运行器不删除用户提供的外部输入文件。

## 环境要求

- Windows：Docker Desktop（Linux containers）、Git Bash、Node.js 22.19、npm 和 Playwright Chromium。
- Linux/CI：Docker Engine、Bash、Node.js 22.19、npm 和 Playwright Chromium。
- 第一次运行浏览器测试可执行 `npm run setup:delivery-tests`。

运行记录写入 `target/system-e2e/runs/<时间戳>-<随机值>`。失败日志写入该运行的 `diagnostics/` 白名单目录，在写盘时脱敏并递归扫描 canary、Authorization、Cookie、Token、私钥和完整 Aster Key；CI 只上传这个目录，不上传含本地运行凭据的整个 `target/system-e2e`。默认会删除临时签名私钥、TLS 私钥、运行时密码和环境文件。`--keep` 只允许 Mock 本机诊断，完成后应再次执行不带 `--keep` 的测试或手动运行该运行专属 Compose project 的 `down --volumes`，并删除对应运行目录里的临时密钥与密码。

调试保留现场后，可用 `--reuse-run=<时间戳>-<8位随机值>` 复用该次已经构建的安装包、临时签名材料和测试证书，但数据库卷、Customer 安装和全部业务状态仍会从零重建。这个参数只用于缩短 Mock 测试脚本迭代时间；默认命令和 CI 始终重新构建安装包。

测试密钥每次运行临时生成，不能替代、覆盖或读取生产 Release/License 签名材料。三种“JSON、QR PNG、终端截图”是许可证申请的传输方式；Operations 签发给 Customer 的最终许可证文件仍统一为 JSON。
