# Customer 使用与运维

面向最终用户的安装步骤和常见问题统一维护在 [Aster Team 用户手册](user-manual.md)。本文继续记录 Customer 的运维与安全契约。

## 首次使用

1. 校验并解压与主机平台匹配的签名包，执行平台引导脚本，再运行根内稳定 CLI 的 `install`。Linux 使用 `init.sh`，Windows 使用 `init.ps1`，macOS 使用 `init-macos.sh`；需要其他位置时只在首次初始化增加 `--install-root <绝对路径>`。程序、配置、密钥、数据库、状态、维护暂存、备份和日志均位于该根目录。临时凭据保存在 `<安装根>/config/control/initial-owner-credentials`，首次登录必须改密。
2. 将安装时生成的 JSON 或 `.qr.png` 标准二维码离线交给运营人员。以后也可使用 `sudo aster-team-cli license request` 重新导出；省略 `--output` 时，CLI 在当前安全目录生成带 UTC 日期时间的 JSON 和对应 PNG，如果当前目录位于签名发布包内则自动写到包外。运营端使用 ZXing 在浏览器本地解析 PNG。
3. 在 Admin 的“产品授权”页面上传 Operations 返回的签名 `license.json` 并立即启用；无法使用网页时，将文件放到当前目录并执行 `sudo aster-team-cli license install --source ./license.json`。
4. 独立 Runner 主机使用匹配其平台的同版本签名包，先执行平台引导脚本和 `aster-team-cli runner install`，再在 Admin 创建一次性注册 Token，将其保存为仅管理员可读的文件并通过 `aster-team-cli runner enroll --token-file <文件>` 完成注册。Control 会在注册响应中一并下发公开任务验签钥，Token 不进入命令行参数。
5. 完成一次或多次上游 OAuth 授权，创建成员、额度和 API Key。

当前产品使用 v2 许可证，分别约束成员席位、Runner、逻辑订阅/账号和每位成员的 active API Key，不接受旧内部 v1 授权。一个逻辑账号可有多份独立 OAuth 凭据，所有健康 Runner 都是动态转发候选，不需要人工绑定。禁用或离线 Runner、禁用或失效账号仍占用相应实体额度，删除后释放；API Key 撤销后释放该成员的 Key 额度。

访问默认使用主要内网 IP 的 HTTP 三端口模式；也可组合 HTTPS 与基础域名。输入 `inner-aster.com` 后固定生成 `app.inner-aster.com`、`admin.inner-aster.com`、`api.inner-aster.com`，域名模式不带端口，并由包内 Caddy 路由到只监听回环的 Rust Control。Caddy 内部 CA 的公开根证书位于 `<安装根>/config/tls/caddy-root.crt`；专用 Runner 通过 `--control-ca-certificate` 信任它。可信局域网 HTTP Runner 必须显式使用 `--allow-insecure-http`，不得把明文入口暴露到公网。

成员 API Key 可用于 OpenAI Responses、Chat Completions、Anthropic Messages、图片生成和图片编辑。OpenAI Base URL 为 `<部署地址>/v1`，Anthropic Base URL 为 `<部署地址>`。图片接口分别为 `POST /v1/images/generations` 与 `POST /v1/images/edits`；后者使用 `multipart/form-data`。公开图片模型固定包含 `gpt-image-2.5-flare`（默认）、`gpt-image-2.5-sunburst`、`gpt-image-2` 和 `gpt-image-1`，不会因为上游文本模型同步结果缺少同名模型而消失。

各公开协议由独立适配器校验并映射到统一的 Responses 事件模型，再由 Codex 上游适配器编码。字符串和消息数组都是 Responses 的正式输入形态；Chat、Anthropic 与 Images 的字段规则不会互相渗透。非流式请求由事件归并器重建完整输出，不能只依赖最终事件中可能为空的 `output`。

## 许可证状态

- `unlicensed`：尚未导入许可证；
- `active`：签名、机器、版本和时间有效；
- `invalid`：结构、签名、公钥 ID 或最低版本无效；
- `machine_mismatch`：许可证不属于当前安装；
- `expired`：达到固定到期时间；
- `clock_invalid`：系统时间违反本机单调检查状态。

许可证、安装身份和时间状态在客户主机的版本目录之外，普通升级会保留。换机时在新机器重新生成申请并由 Operations 签发。

## 凭据与刷新

OAuth Access/Refresh Token 由 Control 使用 installation key 派生的信封加密密钥保存，数据库只有密文、nonce、包裹数据密钥和去重 HMAC。Runner 只在任务执行期间取得所需材料，任务完成后清零内存，不长期保存 Refresh Token。

刷新冲突保护仅针对同一个 `credential_instance_id`：短租约避免同一旋转型 Refresh Token 被同时使用，revision CAS 拒绝迟到结果覆盖新 Token。不同 OAuth grant 是不同凭据实例，能够通过不同 Runner 并行刷新和请求。默认沿用上次成功 Runner；节点掉线时自动改用近期负载更低的健康 Runner。

## 成员席位与额度

创建启用成员时事务占用一个席位；停用或删除成员后释放。Owner/管理员身份与成员计费身份分离，许可证中没有管理员席位字段。超出成员席位时返回固定业务错误，不会临时生成长数字。

额度申请、审批、充值、扣减和冲正均写入同一事务账本。请求没有发送到上游时不扣额度；上游返回可信 usage 后按 usage 结算；没有可信 usage 的中断不会猜测 Token 数。

## 备份与升级

备份以统一安装根为边界，包含恢复账号、数据库、installation/database key、许可证、访问配置、Caddy 状态和 Runner 身份所需的全部持久内容。`aster-team-cli backup create` 默认把归档写入 `<安装根>/backups`；备份归档会排除根内既有 `backups` 和维护 `staging`，不会递归嵌套或打包未完成事务。

Control 推荐从 Admin 的“系统升级”页面上传签名安装包。升级器在非活动槽位落盘并启动候选版本；候选启动时自动执行受签名二进制内嵌的向前迁移，健康后才切换 Caddy 流量，并同步稳定 CLI、服务定义和开机活动槽位。失败不切流，旧版本继续服务。迁移通过 `schema_migrations` 的版本、名称和 SHA-256 账本确保只执行一次，不单独交付 SQL，也不执行数据库降级。

CLI 仍可在校验并解压新包、执行平台引导脚本后运行 `aster-team-cli upgrade`；专用 Runner 使用 `aster-team-cli runner upgrade`。备份通过 `aster-team-cli backup restore --source FILE --confirm` 恢复，专用 Runner 使用 `aster-team-cli runner backup restore --source FILE --confirm`。恢复工作区、恢复前安全备份和失败现场都保留在安装根内；恢复时只原子切换根内受管理内容，不移动根目录本身。启动或健康检查失败时自动换回恢复前内容。Control 备份受机器身份和平台约束，不用于跨机器或跨平台迁移。

常用诊断：

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli logs control
sudo aster-team-cli runner status
sudo aster-team-cli logs runner
```

报告故障时提供固定五位错误码、时间、接口和可选 `request_id`。`request_id` 是单次追踪号，不是错误码。
