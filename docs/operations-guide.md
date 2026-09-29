# Aster Team 内部运营指南

Operations 面向内部销售、商务、财务、交付和运维人员，部署在受控内网，不属于客户安装包，也不与 Customer 建立运行时连接。

## 组件与数据

| 组件 | 默认地址 | 作用 |
|---|---|---|
| Operations API | `127.0.0.1:12090` | Go 业务 API、离线许可证签发和 MariaDB 事务 |
| Operations Console | `127.0.0.1:12080` | Vue 运营界面 |
| Operations Backup | CLI | SQL 备份、哈希、inventory 验证和恢复 |

本次首次对外交付以新授权协议和全新安装为基线。此前版本仅内部使用，不要求迁移旧数据库或兼容旧 License；内部测试环境可按对应实例的重建流程重新部署。新套餐必须显式保存四类额度，签发后由 Customer 在实体写入事务中执行。首发后同一新基线内的续费、免费/付费切换、升级和备份恢复仍须支持。

## 新套餐快照接口与升级权限

当前已确认的付费数量与年费如下。席位按启用且可使用模型的身份计数，纯管理身份不占用；三类非席位资源均须在套餐定义中显式设为 `unlimited`，不能省略字段。

| 年度套餐 | 年费 | 成员席位上限 | Runner / 订阅/账号 / 每人 Key |
| --- | --- | --- | --- |
| 20 席位以内 | 5999 元 | 20 | 不设商业数量上限 |
| 50 席位以内 | 9999 元 | 50 | 不设商业数量上限 |
| 更多席位 | 联系报价 | 按成交数量确定 | 不设商业数量上限 |

年度金额在系统中以分保存，分别为 `599900` 和 `999900`。不限商业数量不影响业务身份校验、数据范围、并发和资源保护。正式上架仍须固定完整套餐版本并批准公开目录；税费和支持条款等未决项不得直接沿用测试数据。更多席位联系报价最终也须保存明确的成交席位数，不能据此签发无限席位。

授权从订单约定的开始时刻起算，购买年数按套餐显式时区的日历年规则计算，目标年月没有同一日期时取该月最后一天。准确起止时间固定在成交快照中，再原样用于 License；不自动改为付款日、签发日、首次安装日或重装日。创建订单前应与客户明确开始时间，后续重试、下载及安装不延长期限。

`/api/operations/v1/commercial/` 内部 API 及“套餐与权益”“订单与收款”页面保存完整套餐版本和订单快照；免费来源签发、全额到账确认及付费 v2 初次批准、签发和下载均已接入运营页面，见[初次付费履约](#初次付费履约)。首发使用 v2 授权链路。旧套餐、订单、许可证、试用和交付页面及对应 HTTP 接口均已移除；安装包管理统一到“安装包”，付费交付统一到“订单与收款”；续费和扩容只接受现有 v2 付费授权作为前驱。免费用户购买后使用首次付费交付，不引用旧内部试用记录。构包和本地开发配置采用受限 v2 签发策略，具体输入见下文。HTTP API 路径中的 `v1` 与 License 格式版本不是同一概念。内部保存不代表已经公开上架，支持的技术模式及测试数据也不代表全部正式商业条款已确认。

- 套餐分别检查 `commercial.plan.read`、`commercial.plan.write`。
- 订单分别检查 `commercial.order.read`、`commercial.order.write`。
- 检查位于业务服务入口；有会话或发布中心权限不代表拥有上述权限。
- 创建订单需明确选择套餐版本、年数和合同开始时间，金额与权益由服务端从该版本计算并固定，客户端不能覆盖；初始状态为待付款。
- 并发修订需提交预期版本，冲突后重新读取；重试保留同一个操作 ID 和相同输入。

运营界面新增“套餐与权益”和“订单与收款”。套餐页面可保存完整配置、查看历史版本；发生 `67703` 冲突时，服务端已确认本操作未保存，比较当前已保存版本与你的修改，明确选择保留配置后才生成下一版本。`67702` 不能用作原操作未保存的证明。不会自动覆盖他人已保存版本或改变既有订单。

订单页面选择固定套餐版本、支持的年数及明确 UTC 开始时间。免费与联系报价不进入年度付费订单下拉框；订单金额和结束时间按服务端保存结果显示。已有订单列表与详情直接使用订单快照，不要求同时取得套餐读取权限。创建订单需能读取候选客户与套餐；当前列表展示最近 100 条记录。

发送前保存按当前账号与标签页隔离的重试记录。遇到回包丢失或未知错误后，后续 403 等错误不会清除原请求；刷新或切换页面后使用对应“继续”入口核对原操作。重试记录保存在 `sessionStorage`，不含密码或会话令牌，不能替代服务端验权；关闭标签页不保证保留该重试记录。已成功保存的套餐草稿另外保存在服务端，不依赖标签页继续存在。公开目录批准、本地导出、官网报价订单、线下到账确认及付费 v2 初次履约见下文；线上自动扣款尚未接入。所有订单使用当前商业履约流程，旧内部业务入口不再提供。

### 到期与免费切换的已确认交付规则

2026-09-08 已确认：付费到期保留登录、现有数据查看与取回、授权导入等必要管理入口，停止新的模型调用、任务和受限资源创建；续费授权生效后恢复，保留原数据和配置。在途请求沿用原有执行和检查逻辑，检查到到期则按原逻辑处理，不额外实现强制中断、保活或保证完成。

付费到期不自动降为免费。用户主动切换前须自行整理资源至免费上限，系统不自动删除数据；切换应保留商业授权历史，不能清空历史来接受旧免费证书或重新使用旧付费授权。

到期后的保留访问、资源整理和主动免费切换已进入当前本地实现，具体操作见用户手册，验收范围见内部实施记录。切换读取当前安装包的 `licenses/free-license.json`，要求已验签、免绑定且不设到期；管理员先整理四类额度，再显式确认。服务端在同一授权变更租约内检查额度并提交，保留付费签发历史和已预存续期；较新的公共免费证书也不得通过普通导入或启动文件替换跳过容量检查。正式发行仍需完成最终验证与本地 Review。

### 已确认售后范围

免费版提供用户手册、FAQ 和公开 Issue，不承诺逐一解决。付费订阅期内包含安装指导、配置咨询和产品缺陷处理，通过客户专属支持群沟通；两个工作日内首次回复，缺陷修复时间在排查后告知，不承诺固定修复期限。首次回复承诺只适用于付费支持，不代表两个工作日内保证修复。

定制开发、代部署和长期代运维另行报价。正式交付时将同一支持条款版本固定到套餐与订单，公开说明采用相同边界；真实联系账号及群工具需落实后再公开，不使用测试账号或占位群链接。

付费订阅期间包含标准功能更新与升级，可选扩展模块按需购买。当前全部现有能力均属于标准功能，暂不拆出收费扩展模块。套餐编辑器可明确勾选“标准功能”集合；该集合随套餐、订单及 License 一起冻结并签名，客户升级软件后在有效订阅期内自动获得新增标准功能。扩展模块必须另行授予，不通过名称通配符或数据库标记自动开放。免费套餐只允许逐项功能，“填入免费默认功能”仅协助创建当前版本的明确列表，不扩大已签证书。续订按续订时的价格建立新订单，原合同期内成交金额与起止时间保持不变。已有标准功能不能通过更改目录分类收回已有授权。

开票处理暂缓。尚未确认的税费与开票信息保持“另行确认”，不对外承诺具体发票类型；以后通过新版本公开配置和条款更新，不改写已成交订单。

Linux amd64 为推荐平台，Windows amd64 按当前实验交付边界说明稳定性和支持范围。Windows 构包与生命周期检查的具体启用状态见下文发行流程，构包成功不能冒充已完成安装验收。PR、tag、公开发行和官网部署按当前任务授权及仓库流程执行，不继承历史本地 Review 的临时状态。

### 保存草稿与生成固定版本

在“套餐与权益”打开“套餐草稿”，选择从新套餐或已有固定版本开始。填写完整有效配置后保存；每次保存产生一个不可变草稿修订，列表显示每个草稿的最新修订。草稿不进入套餐版本列表，不能直接创建成交订单或批准签发，也不会公开到官网。历史修订可由草稿读取接口的 `revision` 参数取得。

两人同时编辑时，较旧修订收到 `67708`，服务端已确认该操作没有保存。点击“比较最新修订”，核对双方配置，再明确选择“保留我的配置”并保存新的修订。不会静默覆盖新内容。已有套餐版本发生变化时，在草稿编辑中使用“核对套餐当前版本”，比较后保存新的草稿基准；代码与目标套餐保持不变。

“生成版本”先显示服务器保存的草稿名称、代码、修订和实际权益；确认后，服务端从该固定修订取值，在同一事务中核对草稿摘要、草稿当前修订和套餐当前版本，并保存固定套餐与来源审计。失败完整回滚，接口不接受客户端补传权益。生成后仍需另行公开批准或签发批准。

生成成功但响应丢失时，用“继续生成版本”重试原操作。即使随后保存了新草稿，恢复仍核对原历史修订并返回原固定版本，不生成第二份。`67708` 或 `67703` 明确冲突后可返回编辑草稿；普通 `67702`、网络失败或尚未确认的错误不表示原操作没有提交，不要为了绕过错误修改操作 ID。成功生成不会删除草稿；继续修订同一套餐时需核对并保存新的套餐基准。

该阶段的独立浏览器验证见[商业运营测试说明](../tests/commercial-operations/README.md)，使用实际 API、独立 MariaDB 库和运营前端生产构建，不调用 CF 部署。

新安装的初始管理员取得上述套餐和订单权限，以及免费分发、签发、公开目录和发布核对权限。升级已有数据库只扩展权限目录，不自动给已有账号提权。由持有受控 Operations 数据库配置的本地管理员，核对具体账号 ID 后执行一次授权：

```sh
./bin/aster-operations-api --env-file /secure/operations.env --migrate-only --grant-commercial-admin OPERATOR_ID --confirm-database aster_operations
```

`OPERATOR_ID` 必须替换为目标 active 账号的准确 ID，`--confirm-database` 必须等于该配置中的实际数据库名。命令不启动 HTTP 服务；授权与审计在同一事务中完成，重复授权不重复记录成功事件。审计的操作者类型标记为本地数据库管理员，不冒充目标账号登录操作。不要把该命令开放为官网或客户可调用入口。

### 公开套餐目录

从“套餐与权益”进入“公开目录”。新建时明确选择本地验证或生产环境，填写仅内部保存的批准说明，添加需要展示的固定套餐版本并调整顺序。预览显示实际公开的名称、说明、权益、价格、期限及支持条款引用；年度总价由服务端使用同一套餐计算规则生成。未选择任何套餐时可以批准空目录，后续官网消费端应显示联系入口，不补造默认套餐或价格。

预览后输入当前密码确认批准。批准绑定完整选中版本及预览摘要，后续修改套餐不会改变已批准目录；调整选择需要重新预览。请求成功但响应丢失时，刷新后点击“继续批准”读取并核对原记录，已经保存则直接恢复。原记录尚不存在时重试原请求；`67710` 表示服务端已确认该批准未保存且预览不匹配，需重新预览。其他未知错误或后续 403 不能证明原操作未提交。密码不进入 `sessionStorage`，每次发送后清空。

| 权限 | 用途 |
| --- | --- |
| `commercial.catalog.read` | 预览、读取批准记录及下载已导出的公开文件 |
| `commercial.catalog.approve` | 批准固定公开目录，另需当前密码 |
| `commercial.catalog.export` | 核对并导出到运营主机，另需当前密码 |

新建页面选择套餐还需要 `commercial.plan.read`，已经批准的目录详情直接从固定记录读取。上述本地商业管理员授权命令包含三项目录权限；迁移不会自动授予旧账号。

在 Operations 的受控环境文件中设置 `ASTER_OPERATIONS_PUBLIC_CATALOG_ROOT` 为专用绝对目录后重启 API，允许进程创建和写入该目录。留空时批准和预览可用，但导出与下载返回 `67711`。该配置是运营主机本地路径，不能设为网站公开根目录；页面不能传入任意文件路径。

打开目录详情，输入“导出验证密码”并点击“导出到运营主机”。实际文件位于 `<ASTER_OPERATIONS_PUBLIC_CATALOG_ROOT>/<environment>/<catalog_revision>/plans.json`；每个 revision 固定一份完整公开 JSON，没有可变的 latest 指针。文件只含白名单投影，不含内部批准说明、操作者、来源快照摘要、换机次数或签发材料。“下载 plans.json”会再次核对实际文件、版本及 SHA-256，下载的字节与本地导出一致。

| 状态或故障 | 处理方式 |
| --- | --- |
| 已批准 | 目录快照已入库，尚不能下载；导出前返回 `67712` |
| 文件写入成功但数据库或回包失败 | 重新输入密码，重试同一目录；核对原文件后补齐状态，不生成新目录或新套餐 |
| 已导出但文件或整个 revision 目录丢失 | 点击“核对并重新导出”，从已批准内容恢复；数据库状态不能代替实际文件存在性检查 |
| 文件内容不同、文件类型异常或符号链接 | 拒绝下载与覆盖；保留异常文件供核对，排查路径和写入来源后再处理 |

批准和本地导出不执行官网构建或部署，不激活生产目录，也不赋予订单受理或许可证签发资格。`production` 仅标记目标环境，不表示已经上线。官网构建输入见[官网说明](../website/README.md)，发布核对及报价建单见下文；收款与付费履约见下文，不自动调用 Cloudflare。Windows 上不声称目录 fsync 或掉电持久性保证；每次下载或重导出均重新核对实际文件，适用 Linux 发布检查另行执行。

### 按官网报价建单

运营服务器的 `ASTER_OPERATIONS_QUOTATION_ENVIRONMENT` 明确选择 `local` 或 `production`；留空不允许新建官网报价订单，原订单仍可按原请求恢复。无效值阻止启动，不推断默认环境。`local` 只用于隔离验证，不能作为生产批准证据；这项配置不会启动发布、部署或签发。

在“订单与收款”选择“按官网报价建单”，选定客户，粘贴客户咨询中的完整目录编号并点击“读取报价来源”。也可填写确切发布记录编号。目录查询只在服务器配置的环境内选择仍在受理期限内的已批准记录，不受最近 100 条列表限制；同目录有多个记录时选择最近获准且仍有效的一条。页面显示选定的具体发布编号和截止时间，保存时固定这些引用。

选择该目录公开的年度套餐版本、年限及明确的 UTC 合同开始时间。金额、折扣、期限与完整权益由服务端从原发布中的固定套餐重新计算；客户端不能传入价格、权益、环境或批准证据。免费、联系报价、未公开版本或年限、未批准记录与已过受理截止的来源不能创建此类付费订单。读取报价来源和建单都使用 `commercial.order.write`，无需发布管理权限；读取订单详情仍使用 `commercial.order.read`。

保存时订单与来源、审计在同一数据库事务提交；受理有效区间为 `accepted_at ≤ ordered_at < accept_until`，其中 `ordered_at` 是数据库等待之后取得的服务端时间，合同开始时间不替代报价受理时间。超时或回包丢失后重试原请求，刷新页面后使用“继续报价订单”，不可另建请求掩盖不确定结果。恢复返回原订单，即使原报价已经过期、客户已停用或当前受理环境配置变化；它不延长日期或新增权益。

官网报价订单使用 `aster.order-snapshot.v2` 并强制包含发布来源，来源和全部成交内容一起纳入订单摘要。旧手工固定套餐订单继续保持 v1 表示、字节和摘要；同一操作 ID 不能跨入口改成另一种订单。后续付款与付费签发必须核对真实原发布记录、固定套餐和完整订单，不能把 compact 来源的格式或自身摘要当作批准凭据，也不能把无发布来源的手工订单当作已经验证的官网报价。当前建单尚不表示收款或批准签发。

### 核对订单到账

在“订单与收款”选择“核对订单到账”，粘贴完整订单编号并读取。页面显示原订单客户、币种、应收全额、合同日期、来源与摘要；该窄查询只需 `commercial.payment.confirm`，不要求套餐、发布或客户列表管理权限。迁移不会自动给旧账号授权，可用本文已有本地商业管理员授权命令明确授予。

核对实际到账后填写可追溯的收款凭据、实际 UTC 到账时刻和备注，勾选全额到账确认并输入当前密码。服务端以原订单金额和币种记录全额收款，不接受重新定价或权益覆盖，也不调用支付平台扣款。部分到账请先人工核对，不要标成全额；到账记录不等于已批准安装或已签发付费授权。可以补录合同结束后的真实收款，但不会延长合同或赋予新签发资格。

- 确认使用 `commercial.payment.confirm` 及当前密码；只持有 `commercial.order.read` 可以读取回执，不能确认到账。
- 确认前固定原操作 ID、订单摘要、收款凭据、到账时间、备注与操作者。回包丢失或刷新后使用“继续确认到账”，重新输入密码并重试原请求；不修改原输入重新确认同一笔收款。
- 当前标签页的恢复记录不保存密码。临时拒绝不清除已经结果未知的请求；首次明确拒绝后保留输入，重新读取订单核对当前状态，避免另一位操作者已经处理后仍按旧页面继续。
- 同一订单只保存一份初次全额到账记录；付款、订单进入待履约状态与审计在同一事务中保存。界面读回原凭据及确认时间，后续订单状态变化不重写原到账事实。
- 读取与恢复核对真实原订单和固定来源，不仅检查付款记录自己的摘要。存储损坏报告服务端错误，不解释成输入无效或引导用户再造一笔记录。

内部接口为 POST/GET `/api/operations/v1/commercial/orders/{orderID}/payment`，辅助投影为 GET `/api/operations/v1/commercial/orders/{orderID}/payment-context`。付款回执采用 `aster.payment-confirmation.v1`，固定完整订单与原请求，密码只用于当前请求的二次认证。

### 官网核对与受理

从“公开目录”进入“官网核对与受理”。这项操作只读取受控站点并记录核对结果，不上传页面或执行部署。配置目标或生成核对结果不表示已经执行官网部署；部署仍走官网的独立发布流程。

| 权限 | 用途 |
| --- | --- |
| `commercial.publication.read` | 读取发布候选、当前环境引用及失败记录 |
| `commercial.publication.prepare` | 保存固定核对请求，另需当前密码 |
| `commercial.publication.accept` | 核对官网并批准受理，另需当前密码 |

新建时读取目录还需要 `commercial.catalog.read`。既有本地商业管理员授权命令包含这三项新权限，迁移不会自动授予旧账号。

运营主机的 `ASTER_OPERATIONS_PUBLICATION_LOCAL_ORIGIN` 配置本地核对源，例如隔离测试使用 `http://127.0.0.1:26394`；它必须是本机地址且不带路径、查询参数或凭据。未来使用的 `ASTER_OPERATIONS_PUBLICATION_PRODUCTION_ORIGIN` 仅接受公网 HTTPS 源，本目标保持未配置。核对器检查 DNS 和实际连接地址，拒绝重定向，不使用环境代理。地址由后端固定配置，浏览器不能传入或替换目标。

1. 导出已批准的固定公开目录，按官网说明构建，并启动相应本地生产构建预览。
2. 点击“新建核对”，输入目录版本并“读取目录”，核对环境、套餐和当前发布基准。
3. 选择该次构建的 `website-release.json`，页面计算其实际字节摘要。核对同时覆盖首页、脚本、样式和公开目录；复制的媒体和安装包不属于这份运行时清单的完整性保证。
4. 填写未来的“受理截止时间”和核对说明，确认目录公开年度套餐的价格和已公布年限在截止前可受理，包括之后切换页面时的历史报价。时间按浏览器时区填写并固定为对应时刻，不设隐含的正式报价期限。
5. 输入当前密码保存候选，再输入“核对验证密码”执行“核对官网并批准受理”。服务端实际读取目标字节，验证成功后将证据、受理状态、环境引用及审计一起保存。

| 状态或故障 | 处理方式 |
| --- | --- |
| 待核对 | 已保存候选，尚未批准受理；排查构建或配置后可重试同一记录 |
| 最近已核对 / 历史已核对 | 反映已保存的核对结果，不承诺站点此后持续在线；恢复历史回执不重新激活旧引用 |
| 保存回包丢失 | 继续原请求，每次重新输入密码；原操作已保存时恢复原记录，不新建重复候选 |
| `67716` 原请求未保存 | 保留目录、说明和期限；重新读取目录、选择构建清单并确认后，才以新操作保存 |
| 已保存候选的基准被其他发布更新 | 读取原记录确认仍待核对，再使用“以此记录准备新核对”；展示新基准并重新确认，旧候选及失败记录不变 |
| 批准回包丢失或提交超时 | 先重新读取原记录；结果未知时只允许原 ID 恢复或重试，不自动建立新候选。已批准的原记录保持原截止和原证据 |
| `67714` / `67715` | 对应目标未配置或内容尚未核对通过；保留候选，在详情查看未完成尝试后排查 |
| `67717` | 本次故障记录未能保存，不能把临时提示当成已留档；读取原发布记录确认是否已批准，再排查存储 |

失败记录按尝试追加，包含发布及摘要、阶段、时间、操作者和固定错误类别，不保存原始网络响应或凭据。详情显示最近 100 次，成功后仍保留历史。请求取消后也会尝试有界保存；存储不可用时明确报告未记录，不伪造审计成功。核对批准只是商业来源的一环，报价订单、到账确认与付费批准签发仍各自执行来源核验，不能凭目录或发布批准直接获得 License。

## 工作台与业务导出

工作台直接统计当前商业套餐、待处理订单、免费分发和付费履约；“已签发付费授权”按付费履约记录计数，不将重下原文件或换机当作新成交。安装包计数来自已入库发布物。

“导出运营数据”需要当前密码，导出 `aster.operations-export.v2` 业务报告。客户、联系人和开票资料与 `commercial_records` 中的套餐草稿/固定版本、订单、到账、免费分发、付费履约、补发、换机、续费/升级来源、公开目录、发布核对和失败记录在同一数据库读取事务中生成；各商业记录包含移除嵌套完整 License 后的快照投影和当前流程状态；`source_snapshot_sha256` 是数据库原始快照摘要，不是脱敏投影的摘要。另含安装包索引、审计和备份记录，不导出完整 License 文件、密码、会话或签名密钥。

该报告用于业务核对，不是数据库恢复文件，也不能作为授权导入。完整恢复继续使用 Operations Backup；客户申请、联系人和业务快照属于内部资料，不应上传到公开支持仓库。旧内部业务表不再属于本次首发的导出格式。

## 许可证链路

```text
免费：固定免费套餐 → 批准免费来源 → 离线预签 v2 → 统一安装包 → 首次安装
付费：固定付费套餐 → 订单 → 全额到账 → 原始机器申请 → 批准与签发 v2 → 客户导入
```

同一个 `request_id` 和 `operation_id` 重试返回同一签发结果。签发文档包含 `key_id`，Customer 从编译进二进制的 License 公钥环选择对应公钥。业务错误使用固定五位码；请求和操作 ID 可以随机，但不能被展示成错误码。

## 发布与首次安装信任

Operations Console 的“发布中心”提供发布任务和 Task → Run → Job → Step/Artifact 详情。创建验证构建时，Operations API 只触发服务端固定的 `customer-release.yml`，并先检查 SemVer、来源 commit 属于 main、仓库版本一致、Workflow 已启用，以及发布环境所需 Secret/Variable 名称存在。同版本、同 commit 的未结束任务由数据库唯一键拒绝并发重复触发。

发布中心分别展示 Linux amd64 和 Windows amd64 安装包的复验状态。“下载”下拉选择平台，只有该包复验通过才可下载；一个包失败不影响另一个已通过的包。“重新复验”也按平台选择，只处理选中的包。下载文件名由后端提供，传输期间同一包不会重复下载。历史 Windows 包不会在升级或重启时自动下载或补验，需要操作员主动发起。数据库通过版本化 SQL 迁移保留既有 Linux 复验记录与下载关联。正式发布审批当前仍只针对 Linux。

从本地 Operations 开发环境触发构建前，必须先在同一来源 commit 的仓库根目录运行 `npm run release:preflight`。该命令会准备仓库锁定的 `cargo-audit` 版本，并执行与远端发布工作流相同的 Rust 和生产 Node.js 依赖审计。远端工作流仍将该检查作为首个强制 Job；预检失败时不会启动后续 Windows 或 Linux 构建。

发布历史保存在 Operations MariaDB；数据库只保存状态、校验摘要、脱敏失败摘要和外部日志/diagnostics 引用，不保存 GitHub 凭据、Release 私钥或完整大日志。GitHub App JWT 和一小时安装令牌只在进程内存中存在。Operations API 启动后会立即扫描未结束任务，随后按固定周期轮询，所以页面关闭或 API 重启不会丢失运行状态。

发布中心 GitHub App 只安装到目标仓库，授予 Actions 读写、Contents 只读和 Environments 只读权限。以下值放入 root 所有、`0600` 的 Operations 环境文件，不提交仓库：

- `ASTER_OPERATIONS_GITHUB_ENABLED=true`；
- `ASTER_OPERATIONS_GITHUB_APP_ID`、`ASTER_OPERATIONS_GITHUB_INSTALLATION_ID`；
- `ASTER_OPERATIONS_GITHUB_APP_PRIVATE_KEY_PEM_BASE64`：GitHub App RSA 私钥 PEM 的 Base64；
- `ASTER_OPERATIONS_GITHUB_REPOSITORY=owner/repository`；
- 固定 Workflow、Environment 和 main ref 配置。

本地 Python 开发控制台可以在首次或重新初始化时选择仓库外的“发布安全配置”目录。目录至少包含 `license-v2.signers.json`、`license-v2.public-keyring.json` 和 `release-v1.public-keyring.json`；选择后，根目录 `.env` 只保存该目录的绝对路径，初始化器会重新校验密钥配对并把所需值写入权限受限的 `data/local/operations.env` 和 `customer.env`。`release-v1.seed` 不会被读取、复制或写入 Operations。

要启用运营端远端验证构建，把 [`operations-release-center.example.json`](../operations/deploy/operations-release-center.example.json) 复制为安全目录中的 `operations-release-center.json`，填写 Build App ID、Installation ID，并把 GitHub App 下载的至少 2048 位 RSA 私钥保存为清单引用的 PEM 文件。清单只允许引用同一目录中的文件名，拒绝绝对路径和目录穿越。运营端原有发布操作可配置 `publish_repository` 和 `publish_app`，但版本自动发布使用下述公开仓库 Tag 流程，不依赖运营端的发布申请。重新初始化会从外部目录重新导入运营端配置。

Operations 还必须配置 `ASTER_OPERATIONS_RELEASE_TRUSTED_KEYS_JSON`。它只包含 Release Ed25519 **公钥**，用于在 GitHub Workflow 成功后独立复核下载结果；不得把 Release seed/private key 放进 Operations。复核会分别校验 GitHub ZIP digest、ZIP 内 `.tar.gz.sha256`、Customer tar.gz SHA-256、安全 tar 文件树、`RELEASE.json` Ed25519 签名、完整文件哈希/权限、CycloneDX 1.6，以及三个 linux/amd64 ELF 都是 `musl-static` 且不含动态解释器或共享库依赖。只有全部通过，任务才从 `verifying` 变成 `completed` 并进入不可变制品库。

GitHub 元数据 API 和构建产物下载使用独立超时：`ASTER_OPERATIONS_GITHUB_REQUEST_TIMEOUT` 默认 `20s`，只约束短 API 请求；`ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT` 默认 `5m`，覆盖 ZIP 下载及响应体读取，允许范围为 `30s` 到 `30m`。网络较慢时只调整下载超时，不要用一个超长 API 超时掩盖 GitHub 身份、权限或接口故障。

版本发布从公开 huzz-open/aster-team 的 v<version> Tag 启动，不需要在运营平台创建发布任务。合并版本文件的 Release PR 后，在同步、干净的 main 上运行 npm run release:tag -- --version <版本> --tag-only。命令读取仓库外安全目录中的已签发免费授权文件，将其 SHA-256 和分发记录编号写入不可移动的签名 Tag。公开仓库 Actions 完成签名构建及 Linux/Windows 验收后，使用该仓库受限的 GITHUB_TOKEN 创建草稿 Release，上传四个包与校验文件，并核对远端摘要后公开。Tag 直接指向这次构建的公开源码提交。成功发布后删除这次运行中两份临时包制品，失败构建的诊断制品按原保留期留存。

在公开仓库的 customer-release Environment 设置 ASTER_RELEASE_SIGNING_SEED_BASE64 Secret，以及 ASTER_LICENSE_TRUSTED_KEYS_JSON、ASTER_RELEASE_TRUSTED_KEYS_JSON、ASTER_RELEASE_SIGNING_KEY_ID、ASTER_CUSTOMER_FREE_LICENSE_BASE64、ASTER_CUSTOMER_FREE_LICENSE_SHA256 变量。Environment 只允许 main 分支运行恢复工作流及受保护的 v* Tag 部署；签名 Seed 不进入源码、包、日志或 Operations。发布上传失败时可重跑失败的 Tag Workflow；已有草稿会继续上传，已公开的 Release 只允许摘要完全一致时幂等结束。若 Tag Workflow 已验收成功、但发布 Job 因条件跳过，不能移动 Tag 或手动调用底层上传脚本；在公开仓库 main 手动运行 Resume verified Customer release publication，输入原始 Tag 和成功的 Tag Workflow Run ID。该工作流复核 Tag、原始 Run、Release Gate、Linux 安装矩阵和两份未过期包后才发布，成功后清除原 Run 中两份临时包。运营平台继续负责签发安装包使用的免费授权文件。

Windows 安装、授权、备份、升级与独立 Runner 测试暂时默认关闭，避免阻塞当前仍无 Windows 客户的版本发布；Windows 客户端工具和签名安装包仍必须构建、校验并上传。需要恢复 Windows 测试时，在公开仓库的 Repository Variables 中把 `ASTER_ENABLE_WINDOWS_TESTS` 设为精确字符串 `true`，并从新 Workflow 运行开始验收；本地 `npm run verify` 与 `npm run release:local` 则需设置同名环境变量。未设置或任何其他值均为关闭。关闭状态下 Release Gate 要求 Runner 测试 job 为 `skipped`，开启时必须为 `success`，Windows 包构建及 Linux 各项验收始终必须成功。正式支持 Windows 客户前应开启测试并留存成功运行记录。

GitHub App 私钥泄露会允许攻击者以该 App 的仓库权限换取短期安装令牌，必须单独加密备份和轮换；它不是 Customer Release seed，也不能与 License/Release 签名密钥复用。

Customer Release 在干净的 Linux amd64 环境中构建一次，不为客户注入环境数据。包内只有 Rust `aster-team-cli`、Control/Runner 和静态前端，不含 Node.js、CJS、Go Guard 或签名私钥。

发布构建需要：

- License 公钥环文件；
- Release 公钥环文件；
- 仓库外、32 字节的 Release Ed25519 seed；
- 与 Release 公钥环匹配的 `key_id`。

构建器先从锁定的 Cargo 解析图与 npm lockfile 生成确定性 CycloneDX 1.6 `SBOM.cdx.json`，拒绝开发依赖混入、许可证元数据缺失和未经批准的许可证，再生成签名 `RELEASE.json`，列出包括 SBOM 在内的完整文件树、大小、SHA-256 和可执行位，并由包内 Rust `aster-team-cli` 再次验签。正式构建还会静态检查 Customer Rust 的 tracing 调用，禁止直接记录授权头、Cookie、Token、密码、凭据、完整请求/响应、提示词、正文和本地密钥；CI 同时对 Cargo/npm 锁文件执行漏洞审计。首次安装必须通过独立可信渠道向客户提供压缩包 SHA-256：客户先校验整个 tar.gz，解压后执行 `sudo ./init.sh`，需要自定义根目录时增加 `--install-root <绝对路径>`。脚本调用包内 CLI 验证 Release 签名和完整文件树，把稳定 CLI 安装到统一根目录，并在 Linux 的 `/usr/local/bin` 建立命令链接。业务安装、升级、许可证、服务、备份恢复和 Runner 生命周期随后全部通过该 CLI 执行，包内 `libexec` 脚本不作为客户命令公开。

正式构建：

```bash
bash scripts/build-linux-amd64.sh --version 2.0.0 \
  --license-keyring /secure/license-public-keys.json \
  --release-keyring /secure/release-public-keys.json \
  --release-signing-key /secure/release-v1.seed \
  --release-signing-key-id release-v1 \
  --free-license /secure/free-license.json \
  --asterctl-windows-x64 /secure/build-inputs/asterctl.exe
```

每个新版本先用发布准备命令统一更新 npm 与 Cargo 的版本清单和锁文件：

```bash
npm run release:prepare -- --version=2.0.1-rc.3
```

该命令要求工作区干净、目标版本是高于当前版本的合法 SemVer，并同时更新 `package.json`、`package-lock.json`、`Cargo.toml` 和 `Cargo.lock`。版本变更必须经过检查并提交到 Git；不要用环境变量或只传 `--version` 把旧源码伪装成新版本。Rust 二进制使用 Cargo 编译期版本，固化版本清单可以保证二进制自报版本、Runner 上报、包名、SBOM、签名清单和升级状态一致。

版本提交进入 `main` 并同步到 `origin/main` 后，GitHub Actions 暂不可用时，可以在已经完成 Windows 与 Docker 构建环境准备的 Windows x64 主机上，从根目录 `.env` 指定的仓库外安全目录构建两个平台。发布入口默认从版本清单读取版本，因此不需要重复传入：

```bash
npm run release:local -- --platform=all
```

默认执行完整源码验证。需要快速生成本地签名包时，可显式选择验证级别：

```bash
# 跳过源码测试，仍执行签名、SBOM、包完整性和平台安装 smoke 校验
npm run release:local -- --platform=linux --verify=none

# 根据相对 origin/main 的变更选择验证套件
npm run release:local -- --platform=linux --verify=changed

# 只运行指定检查组；可选 lint,unit,e2e,build,contracts,docs,assets
npm run release:local -- --platform=linux --checks=lint,unit

# 可选：显式断言版本清单必须是指定版本，写错时立即失败
npm run release:local -- --version=2.0.1-rc.3 --platform=linux --verify=none
```

`--verify` 与 `--checks` 不能同时使用。`--verify=none` 仅跳过源码验证，不会关闭发布签名、SBOM、归档完整性、校验和或平台 smoke 校验；这些发布门禁始终执行。正式打标签前仍必须按发布流程完成完整验证。

本地入口要求 npm 与 Cargo 版本清单一致，源码为干净并已同步到 `origin/main` 的 `main`。可选的 `--version` 只用于断言版本清单，不会动态覆盖二进制版本。包含 Windows 目标时，必须从管理员 PowerShell 执行，主机必须事先安装完整 Perl（推荐 Strawberry Perl）并把 `perl.exe` 加入 `PATH`；可用 `perl -MLocale::Maketext::Simple -e 1` 验证。该入口只检查系统 Perl，缺失时直接失败，不下载或安装 Perl。包含 Linux 时，入口自动发现 Docker Desktop 及其凭据助手、验证 Engine，并提前拉取 Digest 锁定的基础镜像；公共镜像不要求登录。首次 Cargo Release 构建会从源码编译 SQLCipher 与 vendored OpenSSL，之后在工具链、锁文件、目标和编译参数未变化时复用 `target/`；不应在每次本地发布前删除该目录。

AI 执行发布指令时采用固定顺序：收到“生成 VERSION 的 Linux 包”后，先读取当前版本；目标版本不同时运行 `npm run release:prepare -- --version=VERSION`，检查并提交四个版本文件，然后按用户指定的 Git 交付方式让该提交进入并同步到 `main`。完成所需源码验证后，运行 `npm run release:local -- --platform=linux`；用户明确要求跳过源码验证或同一提交已经完成所需验证时，可以增加 `--verify=none`。AI 不得直接调用底层 bundle 脚本、不得用构建环境变量伪造版本，也不得关闭签名、SBOM、完整性、校验和或平台 smoke 检查。

它会先按 `--verify` 或 `--checks` 选择运行源码验证，并验证 License 私钥与公钥环、Release seed 与公钥环的配对关系；Windows 在宿主机原生构建并执行安装、备份恢复和升级生命周期冒烟，Linux 在固定 Docker 工具链内调用上述 `scripts/build-linux-amd64.sh`，签名目录只读挂载。两个平台都先写入 `target/release-local/staging/`，全部通过后才移动到 `dist/windows/` 和 `dist/linux/`；任一步失败都会移除 staging 正式产物，并把结果和诊断保留在 `target/release-local/runs/`。同版本输出存在时必须先明确归档旧产物，命令不会静默覆盖。

Build ID 默认使用 HEAD 的 12 位短 SHA，也可通过 `--build-id=$(git rev-parse --short=12 HEAD)` 显式提供；它只标识本次日志和 staging，不替代版本号或完整提交校验。Caddy 下载在逐次校验 SHA-512 后复用，Docker 编译缓存不再绑定仓库绝对路径。最终报告包含完整源码提交、Build ID 和两级 SHA-256，但不会自动上传或创建 Release。GitHub Windows 发布任务使用锁定 portable Perl，并分别缓存该运行时和 Rust 依赖；本地使用用户已安装的完整 Perl。缓存未命中时仍会完成下载、校验和源码编译。

`customer-release.yml` 支持运营端显式 `workflow_dispatch` 验证构建，也在公开仓库收到 `v*` Tag 时启动正式版本构建。验证构建生成 Actions Artifact；Tag 构建通过 Release Gate 后发布同仓库 Release。正式发布 Job 使用受保护的 GitHub Environment `customer-release`，该环境需提供以下值：

- Variables：`ASTER_LICENSE_TRUSTED_KEYS_JSON`、`ASTER_RELEASE_TRUSTED_KEYS_JSON`、`ASTER_RELEASE_SIGNING_KEY_ID`、`ASTER_CUSTOMER_FREE_LICENSE_BASE64`、`ASTER_CUSTOMER_FREE_LICENSE_SHA256`；后两项是 Tag 构建使用的正式免费证书原始字节及其摘要，更新时必须成对替换；
- Secret：`ASTER_RELEASE_SIGNING_SEED_BASE64`，内容是独立 Release seed 的 Base64，不得使用 Operations 的 License 私钥。

生产密钥必须在仓库外生成。`node scripts/generate-production-signing-keys.mjs <输出目录绝对路径> <签发策略文件绝对路径>` 按显式策略生成独立的 License v2 密钥，并另行生成 Release 密钥；拒绝把私钥写入仓库或覆盖非空目录。策略文件是含 `key_id` 与完整 `policy` 的 JSON 数组（1–8 项），不含私钥。免费分发签发者不得同时包含商业或试用来源；免费和付费采用不同 key ID 与公钥材料。生成器不代替套餐批准，不自动签发证书，也不采用测试密钥。必须长期加密备份 `license-v2.signers.json` 和 `release-v1.seed`；前者是 Operations 许可证签发能力，后者是 Customer 安装与升级包签名能力。GitHub Secret 无法读取原值，不能替代离线备份。两个公钥环和公钥指纹可以公开，但也应与私钥备份一起留存，以便恢复时确认配对关系。

持有 Release seed 的构建 Job 只有仓库只读权限，并拒绝签名不属于 `origin/main` 的提交。Tag 构建通过全部验收后，同仓库发布 Job 使用限于 `contents: write` 的 `GITHUB_TOKEN` 上传经校验的归档和 `.sha256`，不会覆盖已有同名资产。正式交付仍需把 Archive SHA-256 和 `RELEASE.json` SHA-256 通过独立渠道留档、交付。

## 签名密钥

License 私钥泄露会允许伪造许可证；Release 私钥泄露会允许伪造程序包，影响更大。两者必须由不同密钥、不同用途和不同权限控制，公钥也不得复用。客户自己的 installation/database key 泄露只影响该客户安装，不会取得厂商签名权限。

正常轮换使用一次桥接 Release 同时信任旧、新公钥，再切换签名私钥，最后移除旧公钥；不会重加密客户数据库，也不会改变安装身份或 Runner 身份。若 Release 私钥已经泄露，则桥接包必须再通过独立可信摘要或人工渠道确认，不能只相信旧 Release 签名。

## 高风险操作与备份

确认线下到账、数据导出、免费分发批准和许可证签发要求当前密码二次认证。密码、Cookie、完整 Token、私钥和客户凭据密文不得进入日志或审计正文。

备份前停止 Operations API，使 SQL dump 与 inventory 位于同一维护窗口。许可证签名私钥和数据库备份分别加密保管，并定期演练恢复。命令见 [Operations 安装说明](../operations/deploy/README.md)。


## 目标环境维护升级

Linux 双驱动包的 SBOM 使用 `sqlcipher,mariadb` 存储标记。运营平台须先部署支持该标记的复验版本，再导入这类制品；仅 Linux amd64 接受双驱动，原 SQLCipher 制品保持有效。此标记代表包包含驱动，不代表目标环境已具备不停服能力。

发布中心的“环境升级”入口用于内部目标环境。添加环境名称、安装身份、实际 Admin／Member／API 入口、探测模型和专用管理账号／模型 API Key 后，选择已有的独立复验制品，检查目标能力，再发起维护升级。目标的安装身份必须与实际接口一致，制品平台、架构和版本必须匹配。首次接入的旧环境须先通过已验证的 CLI／Admin 维护路径过渡到支持 `correlated_upgrades` 的桥接版本；没有该契约时平台拒绝远程升级。

Operations 配置 `ASTER_OPERATIONS_UPGRADE_CREDENTIAL_KEY_BASE64`，内容为独立生成的 32 字节标准 Base64 密钥，可用 `openssl rand -base64 32` 生成。该密钥放在受保护的部署配置中并单独备份，不存入 MariaDB，也不要直接换成新值，否则历史环境凭据无法解密。此配置是凭据存储所需密钥，不是 localhost／开发模式开关。目标凭据采用 AES-256-GCM 并绑定环境 ID；页面仅显示环境元数据和凭据版本。管理密码或模型 Key 变更时使用“更新环境凭据”，正在运行的任务在没有上传／探测在途时按一分钟周期读取新版本。所有非回环入口使用 HTTPS，内部 CA 可单独提供，重定向不会携带凭据继续访问。

新增 `release.environment.write` 与 `release.environment.upgrade` 权限，分别控制配置／凭据更新与目标检查／升级。已有 `release.build` 账号通过增量迁移获得新权限，管理员可按既有权限表收回；已有 `release.read` 用于查看环境和结果。所有修改继续检查 Operations 会话、来源与 CSRF，并记录操作审计。

后台使用数据库租约执行任务，浏览器关闭不停止升级。默认基线 30 秒、后置观察 60 秒；API 入口每秒、页面每两秒、短模型请求和 SSE 各每五秒探测一次，各类别最多一个在途请求。模型请求总计最多 120 次，探测窗口最多 30 分钟，单次探测最多 20 秒；这些是请求数／时长预算，不是上游输出 token 的硬限制。执行器已接受但响应丢失时仅按关联 ID 查询，不自动重传。无法确认接收状态的任务保持“状态待确认”并占用该环境的升级名额；观察预算结束后继续低频查询，不把未知任务标成完成或自动开放第二次升级。诊断时先核对目标对应任务及执行器状态。

结果分开展示升级执行、连续性与恢复。SQLite／SQLCipher 的连续性始终为“不适用：维护升级允许中断”，保留真实失败和采样观察的最长中断时间；恢复成功不抹去中途失败。后台重启或预算耗尽会保留观察缺口。当前 P2 覆盖 API 入口、Admin／Member HTML、短模型请求与 SSE 的有效输出和结束标志；浏览器交互、会话／对话续接、跨切流证据和结算一致性属于方案 P5，不能用当前摘要声称这些已通过。

开发者可运行 `npm run test:environment-upgrades:db`，自动创建并清理隔离 MariaDB 11.8.6 容器，验证迁移、权限扩展、任务互斥、租约接管、凭据轮换和样本保存。此命令需要本地 Docker，不连接已配置的业务数据库。真实签名包、140 和实际付费上游升级验证由操作员后续补充。

## 免费分发与受限 v2 签发

运营端“授权与交付 → 免费分发”支持从固定免费套餐版本批准来源、签发、核对和下载授权文件。保存套餐不会自动批准，批准也不会自动打包或发行。Customer 自动装载和统一包已在隔离测试签名包中验证；正式证书和公开发行仍需按发行流程完成。

免费套餐不设到期，包含 `gateway`、`member`、`runner` 三项基础能力，四类额度均为 `limited`：`member_seats=3`、`runners=1`、`upstream_accounts=1`、`api_keys_per_member=1`。免费签发器限制来源为 `free_distribution`、绑定为 `unbound`、期限类型为 `none`，权益上限与上述免费范围一致。批准来源仍须明确生效时间；不设到期不表示可以缺失签名字段。

席位按启用且可使用模型的身份计数，纯管理账号不占用；Runner 和订阅/账号停用仍占用、删除释放；Key 撤销释放，更换不额外赠送临时容量。面向客户的规则见[用户手册](user-manual.md#3-安装许可证)。这些正式产品决定应录入 Operations 套餐固定版本，再经批准、签发和公开投影交付；不得把额度硬编码到官网或另建免授权免费包。正式密钥、最低支持版本、支持条款及发行制品继续在相应交付步骤确定。

服务器可选配置 `ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON`，值为 1–16 个签发配置组成的 JSON 数组。留空时签发不可用；非法配置会阻止启动，不回退旧密钥。每个配置严格包含以下字段：

| 字段 | 含义 |
| --- | --- |
| `key_id` | 唯一 v2 签发标识，免费与付费分别配置 |
| `private_key_pkcs8` | Ed25519 私钥的 PKCS#8 DER Base64URL，保存在受控运营主机的非公开配置中 |
| `policy.sources` | 允许的来源类型；免费专用密钥仅允许 `free_distribution` |
| `policy.bindings` | 允许的绑定类型；免费专用密钥仅允许 `unbound` |
| `policy.expiries` | 允许的期限类型，按已批准规则明确选择 `fixed` 或 `none` |
| `policy.entitlement_ceiling` | 共享目录定义的能力与全部额度上限，不能超过获准免费范围 |

`policy` 的四项均必需，不能省略额度表示不限量。实际证书权益仍来自已批准的套餐版本，签发策略只作额外上限检查，不替代套餐或授予更多权益。协议与字段见[分发合同](../contracts/schemas/free-distribution.v1.schema.yaml)和 [v2 合同](../contracts/schemas/license.v2.schema.yaml)。免费有效期与额度采用本节已确认规则；正式受信公钥按发行流程确定，不将测试密钥当作正式默认值。

免费与商业/试用来源不得混在同一签发策略中，构建合同、Rust 和 Go 均拒绝；同一 v2 公钥也不能通过多个 ID 重复登记。配置接口只返回公开 SPKI、公钥标识和受限策略，不返回私钥。恢复历史签发记录需要保留其对应可信配置；直接删除或更换同 ID 密钥会导致旧记录无法验签和下载，不能据此重新生成同编号的不同授权。可通过 `ASTER_OPERATIONS_LICENSE_V2_VERIFIERS_JSON` 单独保留历史可信公钥，不再要求持有原签名私钥才能读取 v2 回执。该配置为 1–64 个条目的 JSON 数组，每项严格包含 `key_id`、`public_key_spki`、完整 `policy`，字段与公开签发配置相同。留空时只从当前签发器提取公钥；显式空数组、null、重复条目、同公钥别名或同 ID 的不同公钥/策略均阻止启动。签发和验签配置中完全相同的条目可共存。不得从数据库或待下载证书复制公钥来建立信任；保留公钥也不授予新的签名能力。

Customer 的受信公钥合同见 [license-trust.v1](../contracts/schemas/license-trust.v1.schema.yaml)。该密钥容器的版本独立于 License 协议；构建配置 `ASTER_LICENSE_TRUSTED_KEYS_JSON` 只接受 1–8 个受限 v2 条目，每项完整保留 `key_id`、`public_key_spki` 与 `policy`。缺失或 null 策略、残缺策略、未知或重复字段、重复 ID/公钥及 License/Release 公钥混用均拒绝。运营端公开配置可直接作为受控构建输入，不得复制 `private_key_pkcs8`。历史 v2 验签公钥可以保留；它不提供签发能力。

本地初始化读取 `license-v2.signers.json` 与 `license-v2.public-keyring.json`，逐项核对私钥派生公钥、ID 和完整策略，在任何数据库创建/重置之前拒绝不匹配。公钥环可额外保留历史只读签发者。未选择外部目录时，仅生成新的本地免费/付费测试密钥，写入 v2 签发与验签配置，履约环境固定为 `local`；不会重用公开测试私钥。旧内部环境需要通过已有初始化流程重建，不自动转换旧配置。

本地构包仅需要 License v2 公钥环、Release 公钥环与 `release-v1.seed`，不读取也不要求 License 私钥。开发初始化不读取 Release seed。正式 Customer 信任来自编译配置，运行目录的环境变量、数据库或证书不能改写；运行时覆盖仅限既有 `local-demo` 构建。一键本地授权通过相同的商业套餐固定版本、订单、到账、批准、v2 签发和下载接口完成；保留原始申请与本地操作 ID，丢失响应重试同一单。它只连接回环地址，生成的订单与到账记录明确标识为本地联调，不发生实际收款。


操作顺序：

1. 保存免费套餐的固定版本，在免费分发页选择套餐，核对全部权益，填写明确的 UTC 生效时间和批准说明，再输入当前密码批准。
2. 在分发详情核对批准人、版本和生效时间，选择获准免费范围的 v2 密钥，重新输入当前密码签发。未配置合适密钥或超出签发范围时拒绝。
3. 签发准备阶段冻结声明和密钥，重试保持同一来源、声明与期限。若响应丢失，重新读取服务器记录；已签发的记录直接下载，不另建来源掩盖不确定结果。
4. 下载时服务器验证来源和签名，浏览器核对实际文件 SHA-256 同时匹配记录和响应头；尚未签发或完整性失败的记录不提供文件。后续正式构建还需验证签名、范围和包的对应关系。

读取、批准和签发分别需要 `commercial.distribution.read`、`commercial.distribution.approve` 和 `commercial.license.issue`。迁移登记权限名称但不向旧账号自动提权；新安装的初始管理员按初始化规则获得权限，已有账号按本指南的显式授权流程处理。签发和批准每次请求都重新验证当前密码，密码不进入商业输入、审计或 `sessionStorage` 恢复记录。恢复记录按账号和标签页隔离，只保存原操作与业务字段，刷新后需重新输入密码。


## 初次付费履约

当前已接通受控付费批准、准备签名、持久交付、下载 API 及“订单与收款”页面的交付弹窗。这些能力不触发网站部署、线上收款、合同签署、正式发行或安装包上传，也不表示 Customer v2 运行时已经验收。

流程以已固定的原订单及全额到账回执为依据。`GET /api/operations/v1/commercial/orders/{orderID}/fulfillment-context` 只需要 `commercial.fulfillment.approve`，提供批准所需的原客户、订单/付款摘要、合同金额和日期、协议/目录版本及可信环境，不额外要求套餐、客户、付款或发布管理权限。

1. 使用 `POST /api/operations/v1/commercial/orders/{orderID}/fulfillment` 批准。输入为固定 `operation_id`、预期订单与付款摘要、原始 `license_request_json` 文本、原因和当前密码。保留客户导出文件的原始文本，不先转换为对象再提交，避免抹掉重复键等非法内容。服务端严格要求明确支持 v2 的安装请求，旧 v1 请求不会自动晋升。
2. 服务端锁定真实订单、到账和客户，每单只允许一份初次批准。批准冻结原来源、完整请求、客户引用、环境与操作者。同一请求 ID 只能继续表示同一客户的同一完整机器请求。只有全新批准才调用本机客户引用派生器；已提交原请求恢复不重算引用或日期，也不因配置轮换升级环境。
3. 使用 `POST /api/operations/v1/commercial/paid-fulfillments/{fulfillmentID}/issue`，提交 `key_id` 和当前密码。需要独立 `commercial.license.issue` 权限。先持久冻结完整 claims 和 key，再签名；证书、issued 状态、订单 fulfilled 状态及审计在同一事务提交。签名准备或最终审计失败会回滚对应状态，不能仅凭已生成签名字节判断交付成功。
4. 原订单的 `/fulfillment` GET 和 `/commercial/paid-fulfillments/{fulfillmentID}` GET 可恢复真实记录；该记录的 `/license` GET 返回经过来源与可信签名核验的原始证书字节，响应含 `X-Content-SHA256`，客户端应核对实际摘要。读取允许 `commercial.fulfillment.read`、`commercial.fulfillment.approve` 或 `commercial.license.issue` 任一权限。下载不要求签名私钥或客户引用密钥仍存在，但对应可信公钥必须保留。

批准与签发均须登录、可信 Origin、CSRF 和当前密码。密码不属于商业请求或快照，不进入持久恢复记录。新权限只登记、不自动授予已有运营账号；按本指南显式授权。输入非法不代表整个订单从未被处理；响应丢失时保留原操作与输入，读取原记录，不创建第二份初次批准。

页面操作从“订单与收款”工具栏的“批准与签发”进入，按订单编号读取原记录。尚未批准时会显示原冻结套餐、完整权益、到账摘要和履约环境；选择原始 v2 JSON 文件、填写批准说明并核对后输入当前密码。批准后再按需读取公开签发配置，只显示允许 `commercial_order`、`installation` 和 `fixed` 的密钥。已经签发的记录无需签发配置读取权限也可核对和下载。批准或签发结果未知时，工具栏变为“继续批准交付”或“继续签发交付”；刷新后必须重新输入密码，且只能继续原操作、原请求和原密钥。

服务器 `ASTER_OPERATIONS_FULFILLMENT_ENVIRONMENT` 只能为空、`local` 或 `production`。空值禁止新批准；已批准来源与当前配置不一致时，准备、签名和最终提交均拒绝。只读原回执和恢复已经提交的原批准不升级环境；已 issued 原请求可返回原文档。同环境的 prepared 记录跨到期恢复保持原声明与合同日期，新批准和首次准备在事务锁等待后也不得越过原到期时间。

环境是业务约束，当前证书 claims 不包含环境标签。`local` 必须使用独立测试密钥，生产 Customer 信任集合不得包含该测试公钥，不能依赖环境字符串隔离同一密钥签出的证书。付费密钥策略应按获准范围明确包含 `commercial_order`、`installation` 和 `fixed`，并配置相应权益上限；不借用免费 unbound 专用密钥。

`ASTER_OPERATIONS_CUSTOMER_REF_SECRET` 可以缺失，此时新付费批准报告能力不可用，原批准恢复不重新派生引用。旧 `ASTER_OPERATIONS_LICENSE_SIGNING_KEY_ID` 或 `ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8` 任一非空都会阻止启动；迁移本地配置时移除旧条目，按上述受限 v2 签发配置设置。系统不会自动补密钥，也不会把能力缺失解释成免授权。旧内部试用证书不再作为付费续接来源。

既有客户表采用不区分大小写的排序规则。若原订单客户 ID 是真实 ID 的大小写别名，付费批准会在派生及提交前拒绝，防止形成不可读回执。应保留原订单与到账记录进行人工核对，等待受控纠正处理；当前不能直接用该别名订单签发。不得改写旧快照或摘要，也不得为了绕过冲突重复确认同一笔款项。正常页面使用从数据库读取的原始客户 ID。
