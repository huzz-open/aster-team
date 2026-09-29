# Aster Team 用户手册

本手册是 Aster Team 面向最终用户的统一安装、使用和故障排查入口。后续新增的用户安装说明与常见问题都继续维护在本文中；开发者的 Linux 构建与验收入口见 [Linux 构建与验证](../README-LINUX.md)。

逐命令的参数、默认值、配置影响和失败处理见 [asterctl 命令参考](../website/docs/zh-cn/tools/asterctl/commands.md)与 [aster-team-cli 命令参考](../website/docs/zh-cn/tools/aster-team-cli/commands.md)。网站文档提供中英文公开版本，安装与恢复流程仍以本手册为统一维护入口。网站中文任务指南直接引用本文的命名章节；更新正文时同步核对对应英文指南。

## 1. 安装前准备

<!-- #region preparation -->

Customer 按操作系统和 CPU 架构分别打包，安装包名称固定为 `aster-team-<版本>-<平台>-<架构>.tar.gz`。推荐使用 Linux amd64；Linux 自动验收覆盖 Ubuntu 20.04/22.04/24.04、Debian 12/13 和 Rocky Linux 9。Windows amd64 作为实验版本提供，不承诺稳定，缺陷修复可能较慢，建议优先使用 Linux。Windows 签名包仍需执行安装、计划任务、日志、升级、恢复和卸载生命周期验收。macOS amd64/arm64 只有实现级静态检查，当前不生成、不下载也不交付 macOS 安装包。具体可下载版本以发行清单为准。

准备以下内容：

- 与目标主机平台、架构匹配的签名安装包；
- 从独立可信渠道取得的安装包 SHA-256；
- Linux 上具有 `sudo` 权限的账号，或 Windows 管理员 PowerShell；
- 能访问 Admin、Member 和 Model API 端口的内网地址；
- 需要连接 ChatGPT 时，服务器必须能够通过 HTTPS 访问 `auth.openai.com` 和 `chatgpt.com`。

不要只使用压缩包旁边的 `.sha256` 文件建立首次信任。第一次执行包内程序前，应当使用通过另一条可信渠道取得的摘要校验整个压缩包。

正式 Linux 与 Windows 安装包都携带 `licenses/free-license.json`。它是已经由 Aster 运营端签名、免机器绑定且不设到期日的免费许可证，不是客户可修改的配置文件；其功能、数量限制和最低版本都在签名内容中。安装器会先验证整个发布树和许可证，再决定是否用于首次安装。历史包内没有该文件时，安装流程保持传统的机器授权申请方式。

<!-- #endregion preparation -->

## 2. 首次安装 Control

<!-- #region installation -->

### Linux

在 Linux x86-64、运行 systemd 的新服务器上，可从官网用一行命令安装最新正式版（需要 `sudo` 权限）：

```bash
curl -fsSL https://aster.huzz.top/install.sh | bash
```

该脚本直接跟随 GitHub `/releases/latest` 的跳转取得最新正式版 tag，从 `huzz-open/aster-team` 下载对应 Linux 安装包及 `.sha256` 文件，校验 SHA-256 后运行包内的 `init.sh` 和交互式 `aster-team-cli install`。脚本运行时不依赖官网发布接口；GitHub 下载失败或校验失败时会停止。安装过程会询问访问地址、管理员邮箱等配置；已有安装请使用升级流程，不要重新运行首次安装脚本。若需自定义安装根目录，请使用下面的手动步骤。

官网安装区也可选择版本并填写管理员邮箱、HTTP/HTTPS 和访问域名或 IP。页面会生成带参数的命令，例如：

```bash
curl -fsSL https://aster.huzz.top/install.sh | bash -s -- --version v2.1.1 --email owner@example.com --protocol https --host team.example.com
```

`latest` 和发布接口列出的正式 Linux 版本可选。仅指定版本时仍进入交互式安装；填写邮箱、协议或地址后，脚本使用发布包支持的无人值守安装，自动安装本机 Runner，并在成功后显示一次性管理员密码，务必保存。未填写邮箱时脚本会在终端询问。HTTPS 默认使用 Caddy 内部 CA。若需要自行选择证书来源、数据库或 Runner 安装方式，请使用交互式默认命令或下方手动流程。

以下示例以 `2.0.0` 为例；安装其他版本时替换文件名和目录名：

```bash
archive=aster-team-2.0.0-linux-amd64.tar.gz
printf '%s  %s\n' '<独立取得的压缩包SHA-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd aster-team-2.0.0-linux-amd64
sudo ./init.sh
sudo aster-team-cli install
```

Linux 默认把全部内容安装到 `/opt/aster-team`。如果要使用其他根目录，只在第一次执行 `init.sh` 时指定；以后 CLI 会从根目录标记自动发现同一位置：

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
```

`init.sh` 验证签名发布树、建立统一安装根并安装稳定 CLI；实际业务安装由 `aster-team-cli install` 完成。

如果 Linux 包携带有效的免费许可证，首次 Control 安装会在创建管理员和数据库前自动导入。安装结束应显示 `A license was installed during setup.` 和 `license: active`，不会再生成机器授权申请。恢复保留数据、升级或安装独立 Runner 时不会自动覆盖已有许可证。

包内没有免费许可证时，CLI 会在安装成功后生成离线授权申请 JSON 和二维码。两类包使用同一套程序、签名发布树和许可证验证链路，区别只在于是否携带已签名的免费许可证。

### Windows

在管理员 PowerShell 中校验并解压 Windows amd64 包：

```powershell
$archive = 'aster-team-2.0.0-windows-amd64.tar.gz'
if ((Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant() -ne '<独立取得的SHA-256>') { throw '安装包 SHA-256 不匹配' }
tar.exe -xzf $archive
Set-Location 'aster-team-2.0.0-windows-amd64'
.\init.ps1
# 继续执行 init.ps1 最后打印的 Next (Control) 命令
```

默认根目录是 `C:\ProgramData\Aster Team`。需要其他位置时只在第一次执行：

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

Windows 不创建额外的数据目录；任务计划程序仅保存服务注册，所有程序和运行数据都在所选根目录。后续始终使用 `init.ps1` 打印的根内绝对 CLI 路径，不要继续调用解压目录中的 CLI。

同一台 Windows 安装多个实例时，必须同时隔离根目录、任务命名空间和全部监听端口。在**首次**执行 `install`（Runner-only 使用 `runner install`）前，可在当前管理员 PowerShell 设置：

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

随后使用所选安装根内的 CLI 执行安装。示例任务位于 `\Aster Team\lab\`；API、Member、Admin 分别使用 21080、21081、21082，内部 Blue/Green 端口也一起偏移，Caddy 管理端口为 12019。第二个实例必须选择另一个根、前缀及未占用的端口段。前缀限 1–32 位小写字母、数字、连字符，首位必须是字母；未设置时保留原任务名称和原端口。

可用 `ASTER_API_PORT`、`ASTER_MEMBER_PORT`、`ASTER_ADMIN_PORT` 单独覆盖公网端口，用 `ASTER_BLUE_API_PORT`、`ASTER_BLUE_MEMBER_PORT`、`ASTER_BLUE_ADMIN_PORT` 和对应的 `ASTER_GREEN_*_PORT` 覆盖内部端口，用 `ASTER_CADDY_ADMIN_PORT` 覆盖 Caddy 管理端口。域名入口另使用 `ASTER_DOMAIN_HTTP_PORT`、`ASTER_DOMAIN_HTTPS_PORT`（默认 80/443，同样参与统一偏移）。单独设置优先于偏移，最终端口必须互不重复且位于 1–65535。

配置会写入安装根的 `install.json`，后续服务管理、升级、备份恢复与卸载使用已保存配置，不再读取这些环境变量。不要手动修改元数据来迁移或重命名实例。恢复要求安装根及实例配置一致；自定义实例不能切换到不支持该配置的旧包。普通卸载会连同客户数据一起保留配置。相同主机名下的不同端口请使用独立浏览器会话或无痕窗口登录。

### macOS（暂不交付）

当前没有经过真实 macOS 构建和验收的签名安装包，不应使用实现目录自行组装客户交付物。待 macOS amd64/arm64 构建、签名和生命周期矩阵正式启用后，本节再提供安装命令。

### 统一目录规则

当前交付的 Linux 和 Windows 都只在首次初始化时确定安装根。程序、版本、配置、密钥、SQLCipher 数据、运行状态、维护锁、升级/恢复暂存、备份和日志全部从该根派生。`install.json` 记录根目录和平台，CLI、Control、Runner、升级与恢复会自动发现它；代码不再依赖散落的业务绝对路径。操作系统目录只保留无法避免的 systemd、Windows 任务计划或命令入口注册，它们只引用根内文件。

交互安装会依次确认 Owner、访问协议、访问地址以及是否安装同机 Runner。默认使用根目录内的本机 SQLCipher，不要求额外执行迁移命令。Linux amd64 的新安装也可按下面的方式选择外部数据库。

安装结束后保存终端打印的初始管理员凭据。凭据也会写入 root 专属文件：

```text
<安装根>/config/control/initial-owner-credentials
```

首次登录 Admin 后立即修改密码；确认新密码已经安全保存后，再删除初始凭据文件。

### Linux 外部数据库安装

本轮实现新增 Linux amd64＋MariaDB **11.8.6** 的外部数据库配置；尚未发布或完成签名包实机验收。Windows、macOS 和 MySQL 不在此矩阵。外部库当前只开放维护升级，完整不停服能力仍待蓝绿与排空阶段完成。

由数据库管理员预先创建专用空库和账号。账号权限限定在该库的 SELECT、INSERT、UPDATE、DELETE、CREATE、ALTER、INDEX、REFERENCES；不使用 Operations 的业务库。保存不含密码的 JSON，例如 `/root/aster-database.json`：

```json
{
  "driver": "mariadb",
  "host": "db.internal.example",
  "port": 3306,
  "database": "aster_team",
  "username": "aster_team",
  "tls": true,
  "custom_ca": true,
  "max_connections": 10
}
```

数据库密码另存为 root 所有、权限 0600 的 UTF-8 文件（例如 `/root/mariadb.password`），不要写入 JSON、命令参数或环境文件。准备服务器证书对应的受信 CA PEM。完成 `init.sh` 后执行：

```bash
sudo aster-team-cli install --unattended \
  --owner-email owner@example.com --owner-password-file /root/owner.password \
  --database-config /root/aster-database.json \
  --database-password-file /root/mariadb.password \
  --database-ca-certificate /root/database-ca.pem --install-local-runner
```

使用驱动内置的公共 CA 信任库时设置 `custom_ca: false` 并省略 CA 文件参数；TLS 始终校验证书链与主机名。仅明确的回环 IP 允许 `tls: false`，不能用于远程数据库。连接信息保存到 `<安装根>/config/control/database.json`，密码保存到 `config/keys/mariadb.password`，自定义 CA 保存到 `config/control/database-ca.pem`；权限为 root:aster-team、0640。启动、诊断、密码重置和维护执行器统一读取安装配置。安装会绑定数据库与安装身份／密钥；错误的数据库、身份或密钥会拒绝启动，不隐式迁移本地 SQLCipher 数据。

原有 `backup create`／`backup restore` 是本地 SQLCipher 备份链路，不支持外部数据库的完整恢复。外部部署需要数据库原生一致性快照以及同一安装根的配置、身份和密钥，不能仅复制本地目录就认为已备份业务数据。外部库的自动备份恢复工具不属于本轮不停服升级工作项。

### 忘记登录密码

成员无法自行恢复或查看旧密码。成员忘记密码时，由管理员进入 Admin 的“成员与额度”，在目标成员的操作栏点击“重置成员密码”。系统会生成一个只显示一次的临时密码，同时撤销该成员已有的全部登录会话。管理员应通过安全渠道把临时密码交给成员；成员使用它登录后必须立即设置自己的新密码。

管理员忘记密码时，不依赖厂商服务器，也不能由 Member 页面重置。请登录部署 Aster Team 的 Control 主机并执行：

```bash
sudo aster-team-cli password reset-admin
```

CLI 会提示选择管理员邮箱，并以隐藏输入的方式要求输入和确认新密码。重置成功后，该管理员已有的全部会话会失效，新密码可直接用于登录。也可以在自动化场景显式指定邮箱：

```bash
sudo aster-team-cli password reset-admin --email admin@example.com
```

成员重置、管理员本机重置和登录后的主动修改共用同一套密码更新机制：数据库只保存密码哈希，每次更新都会撤销目标账号的现有会话并写入审计记录。

默认 IP + HTTP 模式的入口为：

```text
Member UI: http://服务器IP:11081
Admin UI:  http://服务器IP:11082
Model API: http://服务器IP:11080
```

HTTP 只适合可信局域网。公网或不可信网络必须配置 HTTPS。

<!-- #endregion installation -->

## 3. 安装许可证

<!-- #region licensing -->

如果首次安装已经显示 `license: active`，无需生成申请，可以直接进入 Admin 的“产品授权”页面核对套餐、功能和额度。否则，首次安装结束时会生成离线授权申请 JSON、对应的 `.qr.png`，并打印适合截图或拍照的终端二维码。以后也可以在任意安全目录执行：

```bash
sudo aster-team-cli license request
```

将生成的 `aster-team-license-request-<UTC日期时间>.json` 或对应的 `.qr.png` 标准二维码离线交给运营人员。收到签名后的 `license.json`，可以在 Admin 的“产品授权”页面上传，也可以在文件所在目录执行：

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

许可证只在客户主机本地验证，不与厂商服务器通信。本次首发采用新格式 v2 许可证，明确签入功能及成员席位、Runner、订阅/账号和成员 API Key 等额度。界面显示的固定值、禁止或不限量均来自当前已验证许可证，数据库配置和官网文案不能扩大这些权益。此前未对外发行的内部版本不提供旧证书或旧数据库迁移；从内部测试版开始使用本次首发时按全新安装处理。

免费版不设授权到期时间，开放模型接入、成员协作和 Runner 执行的基础流程。免费权益按每套安装分别计算；后续新增的独立高级功能不自动加入已有免费许可证。

| 免费资源 | 上限 | 占用与释放 |
| --- | --- | --- |
| 成员席位 | 3 | 启用且可以使用模型的账号计数；管理员使用模型也占席位，纯管理账号不占。停用释放，重新启用时重新检查容量 |
| Runner | 1 | 离线或停用仍占用；删除后释放 |
| 订阅/账号 | 1 | 禁用或凭据失效仍占用；删除后释放 |
| 每人 API Key | 1 | 未撤销的 Key 计数；撤销释放，更换期间也不能超过 1 个 |

这些规则由随包免费许可证签名授予，不是本地可修改的默认配置。实际包是否已经包含对应免费许可证，以发行说明和安装后的“产品授权”页面为准；免费授权不提供上游 AI 账号或其订阅额度。

续费或升级许可证可以提前导入。签名的 `not_before` 尚未到达时，Admin 会在“产品授权”中把它显示为“下一份授权”，当前许可证、现有数据和业务权限保持不变；到达生效时间后，Control 会在启动、预检或下一次授权检查时原子切换，并在审计日志中记录系统自动激活。下一份许可证要求更高的软件版本时，页面会提示先升级 Aster。预存文件损坏不会隐藏当前许可证，可以重新导入运营人员提供的原文件。

许可证显示有效，表示当前安装、时间和版本满足授权条件；具体功能仍以许可证授予的权益为准。如果成员端提示“当前授权未包含成员功能”，请由管理员在“产品授权”页面核对功能列表并更新许可证。更新后可点击“重新检查授权”，无需重新安装或清空数据。提示“系统尚未获得有效授权”时，则需要检查许可证、有效期及安装状态。

“模型接入”功能支持查看和管理已有订阅/账号及模型。ChatGPT 订阅授权、上游模型同步和人工刷新凭据还需要“Runner 执行”功能，因为这些操作需要节点连接上游服务。API Key 连接的创建与手工添加模型只需要“模型接入”，不访问上游；自动同步另需“Runner 执行”。缺少所需功能时，订阅/账号页面会显示要求并禁用相应按钮，服务端也会返回功能未授权错误（`51008`）。如果两项功能均已授权但提示没有可用 Runner，请检查节点连接状态。凭据已保存而模型同步失败时，应按具体错误处理后重新同步，或手工添加模型，不必重复新增连接。

管理端也按当前许可证的功能显示菜单锁定状态。打开未授权页面会显示功能提示，不会加载该页面的业务数据；“产品授权”等恢复入口仍可进入。导入新的有效许可证后，刷新授权状态或切换页面即可更新菜单，无需重启服务。许可证有效不代表所有功能都已授权。

兑换券投放的接收人列表只显示启用成员；已停用成员需要先启用才能接收新的指定兑换券。消费日志的成员筛选仍保留停用成员，便于查看其历史消费。两个列表均不显示已删除成员或管理员。

导入时若连接中断或提示存储暂时不可用，许可证可能已保存。保留原始文件，排除磁盘空间、目录权限或文件占用问题后，在“产品授权”页面重新读取状态或重试导入同一文件，也可重试上面的 CLI 命令。页面读取状态失败时仍保留上传和命令入口；不要通过删除授权历史来重新激活。如果提示历史缺失、损坏或回退，应按备份恢复流程处理或联系支持。

导入授权与创建成员、启用成员、注册 Runner、添加订阅/账号或创建 Key 同时发生时，若本地状态文件或锁操作失败，会返回 `BACKEND_LOCAL_STATE_FAILED`（`91008`）。等待正在提交的操作结束后，先刷新授权状态和相关列表，再重试未完成的操作。系统会重新核对当前授权，避免使用切换前的旧额度；不要为解决这类竞争删除授权状态或锁文件。

重试不会延长有效期。已经接受的安装即使在到期后恢复完成，也只表示文件保存成功，当前是否可用仍以页面的授权状态和有效期为准。有效授权更新后，运行中的服务会读取提交后的证书，无需为刷新授权状态专门重启服务。

网页导入会保留当时的管理员身份和操作时间，命令行导入记录为系统操作；提前导入与到期自动启用是两条独立记录。证书保存和待写审计通过本地事务一起恢复，服务重启或请求断开不会把原操作人改成恢复时的用户。数据库暂时不可用时，待写记录保留在本机，Control 启动后及运行期间每 30 秒尝试补写。重复恢复同一次事务不会重复记录；重新提交一次导入请求会记录为新操作，因此不确定是否成功时应先刷新授权状态。

若待写审计长期无法处理并达到队列容量，新的导入会停止，原许可证仍保留。应先修复磁盘、权限或审计数据库问题，再读取授权状态并重试；不要删除待写记录来绕过此检查。

### 订阅期间的功能更新

包含标准功能集合的付费授权，在订阅有效期内升级软件即可使用新增标准功能，无需为每次标准功能更新重新导入许可证。可选扩展模块按需购买，增加模块权益后导入更新的许可证；安装包保持统一。席位、资源额度、安装绑定和授权期限继续按许可证执行。

免费授权只开放证书明确列出的功能，不会因软件升级自动获得新的收费能力。软件授权不包含上游订阅、模型消费、服务器或网络费用。

### 订阅到期后的操作

付费订阅到期后，Admin 和 Member 显示“订阅已到期”。已授权功能下的已有数据仍可查看和取回，账号登录、修改密码及必要的授权恢复入口继续保留。管理员可以停用或删除成员、Runner 和订阅/账号，成员可以撤销已有 Key。

新增成员、Runner、订阅/账号和 Key，以及重新启用成员或资源、发放或兑换额度、新的模型调用与任务暂停。页面会禁用对应操作；客户端直接发送这些请求也不会绕过服务端授权。已有请求沿用原有检查逻辑，不增加强制切断或保证完成的处理。

导入生效的续订 License 后恢复相应能力，已有数据、配置和审计记录保留。停用成员会使原登录会话失效；重新启用后需重新登录。证书损坏、安装不匹配、授权历史损坏或不包含对应功能时，不适用到期保留规则。

到期不会自动切换为免费版。若希望继续使用免费额度，在管理端进入“产品授权”，使用“切换免费版”区域：

1. 点击“重新检查”，核对成员席位、Runner、订阅/账号和每人 Key 的使用量。页面从安装包内的签名免费证书读取额度，不依赖手动填写套餐值。
2. 在对应管理页面手动停用成员、删除多余 Runner 或订阅/账号。停用 Runner 或订阅/账号不释放额度；停用成员会释放席位，但不会撤销其 Key。
3. 对每人 Key 超额的身份，在本区域手动撤销不再使用的 Key；停用成员也可以由管理员整理。仅显示 Key 名称和记录标识，不显示密钥明文。每人 Key 一栏显示单个身份的最高未撤销数量。
4. 所有额度满足后点击“切换免费版”，核对确认窗口并执行。提交时服务端会重新核对额度；页面检查后新增资源导致超额时，切换会被拒绝，应整理后重新检查。

切换立即应用免费权益，保留已有数据、配置、付费授权历史和审计，不自动删除任何资源。已预存的付费续期仍按原计划生效。普通导入、替换授权文件或删除历史都不是免费切换方式；即使免费证书签发时间较新，也必须经过上述容量检查。再次购买付费授权时，使用运营人员新交付的 License，不能重放切换前的旧付费文件。

若没有显示切换区域，当前安装可能没有附带可用的签名免费证书，请联系交付人员核对安装包。授权文件、历史完整性或机器绑定异常时，应先恢复授权状态，不能利用免费切换跳过这些检查。

<!-- #endregion licensing -->

## 4. 接入订阅/账号并同步模型

<!-- #region management -->

1. 确认许可证状态正常。在“订阅/账号”页面按类别查看已接入的订阅账号和 API Key 连接；展开一行可查看详情。
2. 接入 ChatGPT 订阅时，确认至少一个 Runner 在线，点击“新增 ChatGPT 账号”完成 OAuth 授权。保存凭据后同步模型；也可以稍后点击该账号的“同步模型”。
3. 接入官方 API Key 时，点击“新增 API Key 连接”，选择 OpenAI、DeepSeek 或 GLM 通道，填写连接名称与 Key。OpenAI、DeepSeek 可选择创建后自动同步模型；GLM 或需要自定义公开名称时，按行填写公开模型名称、上游模型 ID 和额度类型。按张计费的图片模型选择“按张”。
4. 自动同步失败时连接仍会保存，展开该连接并点击“手动添加模型”补录。新增的公开模型默认停用，在“模型管理”中启用并设置成员可用范围后才能供成员调用。

Runner 只允许访问明确批准的 HTTPS 上游域名。白名单是出站网络安全边界，不检查或过滤提示词、回答和 Token 内容。当前 ChatGPT 模型同步与对话必须允许 `chatgpt.com`，OAuth 刷新必须允许 `auth.openai.com`。

### 为成员开放使用

1. 在 Admin 创建或启用成员，确认可消费成员没有超过产品授权的席位上限。
2. 在“模型管理”启用公开模型，并设置允许使用它的成员范围；模型存在不代表已向所有成员开放。
3. 配置成员额度，让成员在 Member 登录并创建自己的 API Key。成员 Key 与管理员保存的上游 Key 用途不同，不要相互替代。
4. 成员从可用模型列表复制公开模型 ID，通过接入说明配置客户端或发起测试请求，再在用量记录中确认结果。

产品许可证、成员模型权限、成员额度和上游可用性分别检查。遇到拒绝时根据具体错误排查，不要仅凭许可证显示有效就重复提交模型请求。

<!-- #endregion management -->

### 使用成员 API Key 调用模型

成员在 Member 页面创建 API Key 后，可以使用 OpenAI Responses、Chat Completions、Anthropic Messages、图片生成和图片编辑接口。OpenAI 客户端的 Base URL 为 `http://服务器IP:11080/v1`，Anthropic 客户端的 Base URL 为 `http://服务器IP:11080`。例如 Responses 同时支持字符串输入和标准消息数组：

```bash
curl --fail-with-body -sS 'http://服务器IP:11080/v1/responses' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -H 'Content-Type: application/json' \
  -H "X-Request-ID: diag-$(date +%s)" \
  --data-binary '{"model":"<已启用模型ID>","input":"你好","stream":false}'
```

#### 选择处理速度和推理强度

Aster 的 `/v1/models` 只返回管理端开放的基础模型，避免把执行选项重复展示成大量模型。调用方既可以使用各协议的原生字段，也可以在不支持这些字段的客户端中手动填写模型变体名：

- `<基础模型>`：速度和推理强度均使用模型原生默认，不向上游注入覆盖值；例如 `gpt-5.6-terra` 的原生推理强度默认是 `medium`。
- `<基础模型>-fast` 或 `<基础模型>-standard`：只指定处理速度。
- `<基础模型>-none`、`<基础模型>-low`、`<基础模型>-medium`、`<基础模型>-high`、`<基础模型>-xhigh` 或 `<基础模型>-max`：只指定推理强度。
- `<基础模型>-fast-high`：按“基础模型-速度-推理强度”的固定顺序同时指定。模型名采用精确匹配，不接受调换顺序、大小写变化或模糊后缀。

例如 `gpt-5.6-terra-fast-high` 会路由到基础模型 `gpt-5.6-terra`，使用快速处理和 `high` 推理强度；响应仍返回调用方填写的模型变体名，路由和消费统计则归入基础模型。模型变体不会出现在模型列表中，需要在客户端手动添加。若模型变体与显式字段同时出现，两者必须一致，否则返回 `400 / 32001`。

使用协议原生字段时，对应关系如下：

| 协议 | 快速处理 | 推理强度 |
| --- | --- | --- |
| OpenAI Responses | `"service_tier":"fast"` | `"reasoning":{"effort":"high"}` |
| OpenAI Chat Completions | `"service_tier":"fast"` | `"reasoning_effort":"high"` |
| Anthropic Messages | `"speed":"fast"` | `"output_config":{"effort":"high"}` |

“接入文档”的协议示例提供“处理速度”和“推理强度”选择器，会按当前协议生成正确字段。选择“模型原生默认”时，相应字段会被省略。`fast` 只改变处理速度，不会自动降低推理强度；推理强度主要控制模型投入的推理工作量，不是严格的输出字数限制。具体模型不支持某个速度或推理等级时，上游仍可能拒绝该组合。

### 在 Codex 客户端中使用成员 API Key

Codex 支持使用自定义 Responses Provider。成员只需安装官方 Codex Desktop，不需要 Aster 启动器、Python、Node.js 或额外图像脚本。每个成员应使用自己在 Member 页面创建的 API Key；Aster 会按成员和 Key 独立统计额度，Codex 则在当前 Windows 用户的默认目录中保存对话历史。

#### 安装 Windows 客户端（跳过 ChatGPT Installer）

当前 Windows 客户端通过 Microsoft Store 分发，商店中的产品名称是 OpenAI 发布的 `ChatGPT`，Codex 是该客户端内的编码代理，不是另一个独立的 MSI 产品。安装后看到应用名称为 ChatGPT 不表示装错。

如果官网提供的 ChatGPT Installer 无法顺利调用 Microsoft Store 下载，可以跳过该 Installer，打开 PowerShell 后直接运行：

```powershell
winget install --id 9PLM9XGG6VKS -s msstore
```

`9PLM9XGG6VKS` 是该客户端的 Microsoft Store 产品 ID。上述命令绕过的是官网的 ChatGPT Installer 引导程序，实际安装包仍由 Microsoft Store 服务下载、校验并安装，因此目标电脑仍需能够访问 Microsoft Store 的软件源。安装前也可以先确认产品信息：

```powershell
winget show --id 9PLM9XGG6VKS -s msstore
```

如果首次使用 `msstore` 源时要求接受软件源或软件包协议，可以运行：

```powershell
winget install --id 9PLM9XGG6VKS -s msstore --accept-source-agreements --accept-package-agreements
```

不要从其他电脑的 `C:\Program Files\WindowsApps` 目录复制已安装文件，也不要把名称以 `ChatGPT` 开头的 MSIX 文件描述为独立的 Codex MSI 安装包；这些做法容易混淆客户端身份、签名、依赖和更新机制。批量部署时应继续使用同一个 Store 产品 ID，并交由组织的软件分发工具管理。

Member“接入文档”的“asterctl 工具”页面会列出当前 Customer 交付包实际携带的客户端制品，并优先选择与浏览器所在设备匹配的平台和架构。下载后先运行页面生成的一行安装命令：Windows 默认移动到 `%USERPROFILE%\.aster\bin`，Linux 和 macOS 默认移动到 `~/.local/bin`；命令同时持久化用户级 PATH，并立即更新当前终端。

完成安装后可在任意目录直接运行 `asterctl`，不再需要使用 `./asterctl` 或 `.\asterctl.exe`。接入 Codex 的步骤为：

1. 在“asterctl 工具”页下载并安装与当前设备匹配的 asterctl；
2. 完全退出 Codex；
3. 在任意目录打开终端，复制并运行页面根据当前平台地址生成的一行初始化命令，例如：

```powershell
asterctl setup codex --base-url "https://当前平台地址/v1" --set-key --launch
```

4. 按终端的隐藏提示输入成员 API Key；初始化成功后，asterctl 会自动启动 Codex。

`asterctl` 会验证平台地址和 Key、设置当前 Windows 用户的 `ASTER_API_KEY`、合并 `%USERPROFILE%\.codex\config.toml`，并通过 Codex 自带 CLI 确认实际 Provider。检测到 Codex 仍在运行时不会修改配置。Key 不会出现在命令行历史、日志或状态文件中。

初始化还会在 Codex 配置目录生成 `aster-models.json`，并设置 `model_catalog_json`。该文件合并原有模型与 GPT-6 Astra、GPT-5.6 Sol、Terra、Luna 的 Fast 条目；新增条目的显示名称以 `Fast Aster` 结尾，请求使用 `-fast` 模型 ID。原有模型的名称、指令、可见性和 API 支持限制保持不变，也不会修改默认模型。所有模型仍通过 Aster Provider 请求，实际可用性取决于平台开放的模型与上游能力。

模型目录优先读取本机有效的 `models_cache.json`，缓存缺失、损坏或包含 Fast 覆盖条目时回退到已安装 Codex 的内置目录；已有自定义目录中的条目也会合并保留。设置了 `CODEX_HOME` 时使用该目录。Codex 升级或原有模型目录更新后，重新运行 `setup codex --base-url "https://当前平台地址/v1"` 刷新合并文件即可，不必重新输入 Key。此功能要求已安装的 Codex 支持 `debug models`；目录加载检查失败时，setup 不会安装新配置。

`--launch` 用于在初始化成功后启动 Codex；省略该参数时只完成配置，不启动客户端。

需要检查或撤销时，可在任意目录使用：

```powershell
asterctl status codex
asterctl doctor codex
asterctl remove codex
```

`remove codex` 只恢复仍与 asterctl 写入值相同的字段，保留 Codex 后续增加的 `notify`、用户修改过的字段以及 `ASTER_API_KEY`。

`status codex` 会显示模型目录路径和条目数，`doctor codex` 还会调用 Codex 检查完整目录能否加载。移除时恢复初始化前的 `model_catalog_json`（原先没有则删除该设置），并删除未被手动修改的生成目录。原始自定义目录不会被改写或删除。如果生成目录已被手动修改，setup 会停止覆盖，remove 会保留该文件和目录引用并报告冲突；请先保存这些修改，再处理冲突。

#### 高级：手动配置或恢复

仅在 `asterctl` 不可用或需要排障时手动设置。先把成员 API Key 保存为当前 Windows 用户的环境变量：

```powershell
[Environment]::SetEnvironmentVariable('ASTER_API_KEY', '<成员API-Key>', 'User')
```

然后打开 `%USERPROFILE%\.codex\config.toml`，把下面整段放在文件最顶部，不要放在 `[desktop]` 或其他 TOML 表之后；不需要配置固定 `model`：

```toml
model_provider = "aster"

[model_providers]

[model_providers.aster]
base_url = "https://当前平台地址/v1"
env_key = "ASTER_API_KEY"
name = "Aster Team"
wire_api = "responses"

[model_providers.aster.http_headers]
x-openai-actor-authorization = "aster-proxy"
```

其中 `x-openai-actor-authorization` 是 Codex 对代理 Provider 开放内置工具的非密钥标记；真实鉴权始终使用 `ASTER_API_KEY`。配置完成后重新打开 Codex 并新建任务；文本对话、内置 `image_gen` 生图和图片编辑都会通过 Aster Provider 请求。`OPENAI_API_KEY` 和 `OPENAI_BASE_URL` 不是该方案的依赖。

### 在 Claude CLI 中使用成员 API Key

Claude CLI 可通过 Aster 的 Anthropic Messages 兼容接口工作。`ANTHROPIC_BASE_URL` 必须是服务根地址，不能带 `/v1`；Claude CLI 会自行追加 `/v1/messages`。Aster 使用 Claude Code 的 `modelOverrides`，把 Fable、Opus、Sonnet 和 Haiku 的完整模型 ID 自动映射到管理端当前开放的模型。管理员和成员都不需要另行维护映射；开放模型发生变化后，下一次初始化会直接使用新的自动映射结果。

#### 使用 asterctl 自动初始化

安装 `asterctl` 后可在任意目录直接运行初始化命令。`project-demo` 是项目目录示例，复制后将它替换为绝对路径或相对于当前终端目录的相对路径：

```powershell
asterctl setup claude --base-url "http://服务器IP:11080" --project "project-demo" --set-key --launch
```

也可以省略 `--project`，此时使用当前终端目录：

```powershell
asterctl setup claude --base-url "http://服务器IP:11080" --set-key --launch
```

`asterctl` 会自动执行以下操作：

1. 运行 `claude --version` 并解析版本。最低支持 Claude Code `2.1.255`；版本过低时提示运行 `claude update`，不会创建项目目录、询问覆盖或写入文件。
2. 使用隐藏输入的成员 API Key 完成鉴权，由 Aster 根据管理端当前开放的模型实时生成映射。
3. 调用与 Member 手动配置页面相同的 Rust 公共模块生成完整配置。
4. 解析项目路径；目标目录不存在时创建，已存在时直接复用，然后在其中写入 `.claude/settings.local.json`。
5. 指定 `--launch` 时进入目标项目目录并自动启动 Claude；省略该参数时只完成配置。

如果目标配置已经存在，`asterctl` 会显示文件路径并询问是否覆盖。选择否会立即退出，不修改任何文件，并提示改用手动配置；选择是会先在同一目录创建备份再覆盖。可使用以下命令检查、诊断或撤销自动配置：

```powershell
asterctl status claude --project "project-demo"
asterctl doctor claude --project "project-demo"
asterctl remove claude --project "project-demo"
```

这些命令的 `--project` 同样可省略，省略时检查或处理当前终端目录。

`remove claude` 只处理仍与 `asterctl` 写入内容一致的配置。初始化前已有文件时会恢复备份；配置后来被用户修改时会拒绝覆盖这些修改。

#### 高级：手动配置

在 Member“接入文档”的 Claude CLI 页面粘贴 `claude --version` 完整输出。页面会根据 Claude Code 版本和管理端当前开放的模型自动生成 `.claude/settings.local.json`，不提供人工映射选项：

```json
{
  "env": {
    "ANTHROPIC_BASE_URL": "http://服务器IP:11080",
    "ANTHROPIC_API_KEY": "<成员API-Key>",
    "ANTHROPIC_DEFAULT_FABLE_MODEL": "claude-fable-5-1",
    "ANTHROPIC_DEFAULT_OPUS_MODEL": "claude-opus-5",
    "ANTHROPIC_DEFAULT_SONNET_MODEL": "claude-sonnet-5",
    "ANTHROPIC_DEFAULT_HAIKU_MODEL": "claude-haiku-4-5-20251001"
  },
  "model": "opus",
  "modelOverrides": {
    "claude-fable-5-1": "gpt-5.6-sol",
    "claude-opus-5": "gpt-5.6-terra",
    "claude-sonnet-5": "gpt-5.4-mini",
    "claude-haiku-4-5-20251001": "gpt-5.6-luna"
  }
}
```

低于 `2.1.255` 的 Claude Code 不生成配置，请先运行 `claude update`。兼容信息以 [Claude Code 官方模型配置文档](https://code.claude.com/docs/en/model-config) 和官方发布记录为依据，在 Aster 发版时更新；运行时不会抓取外部网页，因此配置生成不依赖 Anthropic 官网可用性。`asterctl` 自动初始化和 Member 手动配置页面都调用 `aster-claude-config` 公共模块生成配置，不会各自维护一套映射或 JSON 模板。

从项目根目录启动新会话：

```bash
claude
```

`model` 默认使用 `opus` 别名。`ANTHROPIC_DEFAULT_*` 把 Claude 的家族别名固定到已验证的完整 Claude 模型 ID，`modelOverrides` 再把这些 ID 映射到 Aster 当前开放的模型；因此 `/model` 切换家族时仍会经过同一套映射。

`.claude/settings.local.json` 含成员 API Key，只能保存在本机，不得提交到版本库。项目的 Git 忽略规则应包含：

```gitignore
**/.claude/settings.local.json
**/.claude/.asterctl-state.json
**/.claude/settings.local.json.asterctl-*.bak
```

图片接口固定公开 `gpt-image-2.5-flare`（默认）、`gpt-image-2.5-sunburst`、`gpt-image-2` 和 `gpt-image-1`，不要求模型同步结果中出现同名模型。Flare 适合快速、高质量的日常生成，Sunburst 适合质量优先和精确编辑：

```bash
curl --fail-with-body -sS 'http://服务器IP:11080/v1/images/generations' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"gpt-image-2.5-flare","prompt":"一只坐在窗边的猫","size":"1024x1024","response_format":"b64_json"}' \
  > aster-image-response.json

python - <<'PY'
import base64, json, pathlib, sys
sys.stdout.reconfigure(encoding='utf-8')
result = json.loads(pathlib.Path('aster-image-response.json').read_text(encoding='utf-8'))
output = pathlib.Path('aster-image.png').resolve()
output.write_bytes(base64.b64decode(result['data'][0]['b64_json']))
print(f'图片已保存: {output}')
PY

curl --fail-with-body -sS 'http://服务器IP:11080/v1/images/edits' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -F 'model=gpt-image-2.5-sunburst' \
  -F 'image[]=@aster-image.png' \
  -F 'prompt=把天空改成晚霞' \
  > aster-edit-image-response.json

python - <<'PY'
import base64, json, pathlib, sys
sys.stdout.reconfigure(encoding='utf-8')
result = json.loads(pathlib.Path('aster-edit-image-response.json').read_text(encoding='utf-8'))
output = pathlib.Path('aster-edited.png').resolve()
output.write_bytes(base64.b64decode(result['data'][0]['b64_json']))
print(f'图片已保存: {output}')
PY
```

`400` 表示公开协议字段或参数无效；`502 / 35002` 表示请求已经进入网关，但上游或 Runner 执行失败。排查 `502` 时保留自定义 `X-Request-ID`，并在 Control 日志中按该值查找脱敏后的公开模型、内部路由模型、Runner 和上游 HTTP 状态。不要把 API Key 或上游响应正文写入工单。

## 5. 安装独立 Runner

<!-- #region runners -->

在 Admin 的“Runner 节点”页面创建节点，选择 Runner 所在平台，复制页面生成的完整安装命令并在目标主机运行。安装器会询问一次安装根目录，默认使用执行命令时的当前目录：Windows 从 `D:\software` 执行时默认安装到 `D:\software\Aster Team`，Linux 从 `/data` 执行时默认安装到 `/data/aster-team`。输入 `Y` 使用该默认目录，也可以直接输入另一个已存在的绝对根目录。

Control 从当前已签名 Release 中提供同版本的精简 Runner 内容。目标主机只安装 Runner、CLI 和服务启动文件，不安装 Control、Caddy、Admin/Member 前端或数据库。Linux 将一次性 Token 临时保存为 root 所有、权限 `0600` 的文件；Windows 只允许 Administrators 和 SYSTEM 读取，注册完成后立即删除。

安装过程写入 `<安装根>/logs/install-runner.log`；Runner 启动后的 Windows 运行日志位于 `<安装根>/logs/runner.log`，Linux 使用 `journalctl -u aster-runner.service`。安装中断后可重新执行同一命令，安装器会识别并继续自己留下的未完成 Runner 目录；已完成安装或包含其他文件的目录不会被覆盖。

HTTP 内网入口必须由页面命令显式允许不安全传输；生产环境应使用 HTTPS。服务端只能提供已经预置的目标平台 Runner 内容，不会在安装时转到 GitHub 或其他公网地址下载。

Runner 与订阅/账号不绑定。所有在线、启用、协议兼容、心跳新鲜且仍有容量的 Runner 都是动态路由候选；默认优先使用该账号上次成功的 Runner，掉线后再选择近期负载更低的节点。

<!-- #endregion runners -->

## 6. 日常检查和日志

<!-- #region daily-checks -->

以下展示 Linux 的命令入口；Windows 使用 `init.ps1` 打印的 `<安装根>\bin\aster-team-cli.exe` 绝对路径并去掉 `sudo`，子命令相同。

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
sudo aster-team-cli logs control
sudo aster-team-cli logs runner
```

`status` 适合快速查看服务状态和访问地址；`doctor` 检查安装身份、数据库、许可证、公开入口和服务预检。服务为 `active` 只说明 systemd 进程仍在运行，不等同于真实上游请求一定成功。

需要实时观察日志时：

```bash
sudo aster-team-cli logs control --follow
sudo aster-team-cli logs runner --follow
```

Control 使用 Blue/Green 槽位，CLI 会自动读取当前活动槽位；不需要用户判断 `aster-control@blue` 或 `aster-control@green`。

<!-- #endregion daily-checks -->

## 7. 备份和升级

<!-- #region backup-upgrade -->

当前源码中的 Runner 协议已升级为 v3（尚未据此完成实机发布验证）。首次从 v2 过渡需安排维护窗口，协调更新 Control 与所有承担请求的本地／远程 Runner；旧 Runner 不具备新协议的执行期限与取消能力，新 Control 不会把它当作兼容节点。此变更不表示蓝绿升级已经开放。

省略输出路径时，备份写入 `<安装根>/backups`：

```bash
sudo aster-team-cli backup create
```

内置 SQLite／SQLCipher 部署**不支持不停服升级**。在 Admin 的“系统升级”页面查看环境能力并上传对应平台和架构的签名 `.tar.gz`，执行维护升级。系统先校验和准备发布包，再停止同机 Runner 和两个 Control 槽位，确认停止后启动候选并执行内嵌迁移；候选健康后恢复 Caddy 入口、稳定 CLI 和原先运行的同机 Runner。期间用户端、管理端、API Key 调用可能不可用，已有 stream 可能中断，请安排维护窗口。

页面暂时断线时会继续查询后台任务；不要重复上传，重新连接后以最终任务结果为准。候选失败时会先停止候选，再尝试恢复原版本并检查健康；恢复失败会明确提示需检查服务状态，不保证旧版本始终在线。数据库不会自动回退或恢复旧数据快照。服务恢复未完成时，任务会保留并由执行器下次运行重试，期间不能再提交升级。若只是安装包校验或解包时中断，恢复流程不会重启业务服务。

外部数据库的不停服升级尚未正式交付；管理端按实际后端能力开放升级，不会因为配置了外部数据库就宣称支持不停服。连续性探测和开发测试结果由内部 Operations 工作项提供，不放入客户管理端。

旧版没有升级能力信息时，首次过渡使用新签名包的 CLI 维护路径：核验并解压新版本，在新包目录执行下面的命令；`init.sh` 会先更新稳定 CLI，随后由新版执行器升级。不要用旧管理端上传来假设旧执行器也会采用新维护顺序。已有自定义安装根时，必须给 `init.sh` 传入原 `--install-root`，随后执行该根 `bin` 下的 CLI：

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

旧版 Windows／macOS 的维护任务可能仍从 `current` 启动 CLI。首次过渡须核对实际启动器；旧执行器尚未切换时，不承诺中断后自动恢复。恢复应由已验证的新包 CLI 执行，不能让旧执行器接管未完成的新任务。

迁移执行结果记录在数据库 `schema_migrations`，相同版本、名称和 SHA-256 只成功执行一次。SQL 不单独交付，也不要求用户手动运行 migrate。数据库只向前升级，不做反向迁移；发布迁移必须保持旧程序可读。升级成功后不能手动切回旧程序，确认稳定后可在页面删除非当前版本和对应快照。

升级保留安装身份、SQLCipher 数据库、许可证、账号、设置和 Runner 身份。恢复备份前应先阅读命令帮助并确认备份属于当前机器：

```bash
sudo aster-team-cli backup restore --help
```

外部数据库需要使用数据库原生工具取得一致性快照，并同时备份同一安装的配置、身份和密钥；仅运行本地 `backup create` 不足以恢复外部数据库。升级后运行 `version`、`status` 和 `doctor`，再验证登录和实际模型调用。

<!-- #endregion backup-upgrade -->

详见 [Runner 通信与任务票据](runner-trust-model.md#通信与任务票据)。

## 8. 常见问题

<!-- #region troubleshooting -->

### 页面打不开，但服务显示 active

先确认安装器打印的是真实服务器 IP，而不是 `127.0.0.1`：

```bash
sudo aster-team-cli status
ip -4 route get 1.1.1.1
sudo ss -lntp | grep -E ':(11080|11081|11082)\b'
```

然后检查客户端到服务器的防火墙、安全组和路由。直接 IP + HTTP 模式应监听配置的内网地址；域名模式还需要正确的 DNS 或客户端 hosts 记录。

### Runner 页面在线，但同步模型返回 `33003`

`33003 / RUNNER_NOT_READY` 表示路由时没有同时满足在线、启用、协议、心跳和容量条件的 Runner。先在 Admin 点击该 Runner 的“测试连通性”，再查看两端日志：

```bash
sudo aster-team-cli logs runner
sudo aster-team-cli logs control
```

再检查 Runner 的 DNS 和 HTTPS 连通性：

```bash
sudo systemctl cat aster-runner.service | grep '^ExecStart='
getent ahosts chatgpt.com
sudo -u aster-runner curl -sS -o /dev/null -w 'HTTP %{http_code}\n' https://chatgpt.com/backend-api/codex/models
```

能够返回 HTTP 状态码说明 DNS、TLS 和基础网络已经连通；连接超时、解析失败或证书错误需要继续检查代理、防火墙和 CA。

### 执行程序提示缺少 `GLIBC_2.xx`

这表示使用了较早的 glibc 动态链接安装包。当前正式 Customer 包使用 musl 全静态编译，不依赖宿主机 glibc。重新下载并校验最新签名包，不要从旧解压目录继续安装。

### 安装许可证提示文件不存在

先运行 `sudo aster-team-cli license status`。如果显示 `license: active`，说明随包或显式许可证已经安装，不需要再次执行 `license install`。如果显示 `license: missing`，`license install` 只负责导入已经签发的许可证，不负责创建许可证；执行 `license request`，把申请交给运营人员签发，收到文件后进入它所在的目录并使用相对路径：

```bash
pwd
ls -l ./license.json
sudo aster-team-cli license install --source ./license.json
```

### 如何提交有效的故障信息

模型请求失败时，先复制错误正文或 `X-Aster-Request-ID` 响应头中的 request ID，然后在 Control 主机执行：

```bash
sudo aster-team-cli trace
```

命令会提示粘贴 request ID，并自动查询当前活动的 Blue/Green Control 服务最近 24 小时日志。也可以直接传入 request ID；如果问题发生得更早，可用 `--hours` 扩大到最多 720 小时。无需查找 systemd 单元名，也不要修改示例中的占位参数。

每次模型 HTTP 请求都会生成独立的 Aster request ID，用于额度预留、结算和日志追踪。客户端提供的有效 `x-client-request-id` 会继续传给 OpenAI；缺失或无效时依次使用有效 `x-request-id`、Aster request ID。客户端编号与内部执行编号会关联记录，但不作为 Aster 的额度幂等键；同一会话的后续请求或重试可以复用客户端编号，每次实际执行仍分别计费。排查时优先使用错误正文或 `X-Aster-Request-ID` 返回的内部标识。

请同时提供：

- 问题发生的准确时间和时区；
- 操作页面或 CLI 命令；
- 固定五位错误码和字符串错误码；
- `aster-team-cli status`、`doctor` 的输出；
- Control 与 Runner 同一时间段的日志；
- 是否使用 HTTP、HTTPS、域名、代理或内部 CA。

不要提交密码、Cookie、API Key、Access Token、Refresh Token、许可证私钥或完整对话内容。`request_id` 是单次请求追踪号，不是错误码。

### Windows Git Bash 中的中文 JSON 请求返回 `400`

接入页面的“cURL”示例面向 macOS 和 Linux；Windows 应优先选择同一页面的“PowerShell”示例。如果需要在 Git Bash 中直接运行 cURL 示例，先确认实际调用的是哪个程序：

```bash
type -a curl
curl --version
```

Git Bash 通常优先使用自身携带的 `/mingw64/bin/curl`。如果英文请求正常、中文请求返回 `400 / INVALID_REQUEST`，使用 Windows 自带的 cURL 重新执行原示例，JSON、请求头和其他参数都不需要修改，只替换命令名称：

```bash
/c/Windows/System32/curl.exe --fail-with-body -sS 'http://SERVER_IP:11080/v1/responses' \
  -H "Authorization: Bearer ${ASTER_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"gpt-5.6-luna","input":"你好，请介绍一下自己。","stream":false}'
```

也可以在当前 Git Bash 会话中固定使用它，然后直接粘贴接入页面生成的 cURL 示例：

```bash
alias curl='/c/Windows/System32/curl.exe'
```

这不是服务端或模型不支持中文。示例使用 `--fail-with-body`，HTTP 失败时会同时保留非零退出码和 JSON 错误正文；若仍然失败，可以增加 `-i` 一并查看响应头。不要把真实 API Key 粘贴到聊天、工单或日志中。


### 请求中断后额度仍显示被预占

请求开始时会预占部分额度，正常结束后按实际结果结算。Control 意外退出等情况可能留下尚未结算的预占。Control 恢复运行后会先尝试恢复已经持久记录的实际结果，按该次请求冻结的用量和倍率结算。没有持久结果的预占，在请求创建 30 分钟后定期核对并分批回收，无需用户再次发送模型请求。仍在执行的请求不会被这项到期回收抢先处理。批次扫描和数据库恢复需要时间，因此到期后可能不会立即显示释放。

无法确认实际用量的到期项目会记录为“预占到期回收，未计费且上游执行结果不确定”。Key 已撤销或产品授权已到期，仍可处理这些既有预占；新的模型请求继续受当前授权限制。数据完整性检查失败时，该项目会保留供排查。请保留安装目录中的 `data/settlements`，不要通过删除恢复记录释放额度；尚未成功写入磁盘的用量不能保证恢复。若额度持续未恢复，请按“如何提交有效的故障信息”提供问题时间、request ID 和相关 Control 日志。

### Windows 实验包的免费初始授权

推荐使用 Linux amd64。Windows amd64 作为实验版提供，不承诺稳定，缺陷修复可能较慢。下载时核对平台为 Windows amd64，并按对应发行页的摘要校验压缩包。

带有 `licenses/free-license.json` 的统一安装包会在首次全新 Control 安装时自动验签并导入免费证书，无需联网领取授权。签名或有效性检查失败会停止安装，不会跳过授权校验。付费后使用同一安装，在管理端或 CLI 导入私下交付的付费 License；升级、恢复和回滚保留已有授权。独立 Runner 不导入这份免费证书，其可用权限由所属 Control 决定。

<!-- #endregion troubleshooting -->
