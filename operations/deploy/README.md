# Aster Team Operations 本地安装包

Operations 是 Aster Team 内部使用的 Go API、Vue Console 和备份工具，不属于客户交付包，也不需要公网部署。

## 配置

1. 创建独立 MariaDB/MySQL 数据库和专用账号。
2. 复制 `operations.env.example` 到安装包外部的 root-owned `0600` 文件。
3. 填写数据库、初始操作员、许可证 Ed25519 私钥、客户引用密钥和 ArtifactStore 路径；Operations 没有“交付签名私钥”。
4. 推荐从发布中心触发 GitHub 验证构建并由 Operations 独立复核后自动入库；受控 inbox 手工导入只用于已有离线制品。

若启用“发布中心”远端构建，在 GitHub 创建只安装到目标仓库的 Build App，授予 Actions 读写、Contents 只读、Environments 只读权限。Operations 还保存 Release **公钥环**，用于独立复核 GitHub 产物；这里不能保存 Release seed。若启用正式发布，再创建第二个只拥有 Contents read/write 的 Publish App。两个 App 的 ID、Installation ID 和 RSA 私钥 PEM Base64 写入 root 所有、`0600` 的 `operations.env`，短期安装 Token 只存在进程内存。

`ASTER_OPERATIONS_GITHUB_REQUEST_TIMEOUT` 默认 `20s`，用于 GitHub 元数据 API；`ASTER_OPERATIONS_GITHUB_ARTIFACT_DOWNLOAD_TIMEOUT` 默认 `5m`，单独用于 Actions ZIP 产物下载。慢速网络应增加后者，避免任务停留在“待复核”。

必须长期加密备份且不得泄露：License Ed25519 私钥、Release Ed25519 seed、Build App RSA 私钥、Publish App RSA 私钥、Operations 数据库凭据和客户引用 HMAC Secret。Release/License 公钥环与指纹可以公开，但仍应备份以确认密钥配对。任何私钥都不得提交 Git、写入 MariaDB、进入前端或日志。

```bash
tar -xzf aster-operations-<version>-linux-amd64.tar.gz
cd aster-operations-<version>-linux-amd64
sudo ./install.sh --install-root /opt/aster-operations --env-file /root/operations.env
```

程序、配置、Artifact、备份和运行数据都位于 `--install-root` 下；默认是 `/opt/aster-operations`。安装器只会在该目录外注册 systemd unit，其目录可通过 `--service-registration-root` 指定。`ASTER_OPERATIONS_ARTIFACT_ROOT` 可以填写相对路径，安装时会解析到根目录的 `data/` 下并写入最终配置。

默认仅监听 `127.0.0.1:12080` 和 `127.0.0.1:12090`。需要从内网访问时，应使用带身份认证的 TLS 反向代理，并同步设置 `ASTER_OPERATIONS_TRUSTED_ORIGINS`。

## 许可证操作

订单付款并履约后会生成许可证策略。Customer 使用 `sudo aster-team-cli license request [--output <文件>]` 生成或导出机器申请；Operations 只接收当前签名协议的申请文件并签发机器绑定许可证。

运营人员在“许可证”页面选择策略、导入申请、输入当前 Operations 密码并签发。下载的许可证文件只能在申请对应的安装环境使用。首次签发不计换机；不同机器的后续签发受 `transfer_limit` 控制。

## 备份与恢复

先停止 Operations API。操作员密码只从标准输入读取：

```bash
read -rsp 'Operations operator password: ' AT_OPERATIONS_PASSWORD; printf '\n'
sudo systemctl stop aster-operations-api.service
printf '%s\n' "$AT_OPERATIONS_PASSWORD" | sudo -u aster-operations \
  /opt/aster-operations/current/bin/aster-operations-backup \
  --env-file /opt/aster-operations/config/operations-api.env \
  --operator-email admin@example.com --password-stdin --maintenance-confirmed \
  --output /opt/aster-operations/backups/aster-operations.sql
sudo systemctl start aster-operations-api.service
unset AT_OPERATIONS_PASSWORD
```

校验：

```bash
printf '%s\n' "$AT_OPERATIONS_PASSWORD" | sudo -u aster-operations \
  /opt/aster-operations/current/bin/aster-operations-backup \
  --env-file /opt/aster-operations/config/operations-api.env \
  --operator-email admin@example.com --password-stdin \
  --verify /opt/aster-operations/backups/aster-operations.sql
```

恢复是破坏性维护操作，必须停服、重新验证备份并显式确认数据库名：

```bash
printf '%s\n' "$AT_OPERATIONS_PASSWORD" | sudo -u aster-operations \
  /opt/aster-operations/current/bin/aster-operations-backup \
  --env-file /opt/aster-operations/config/operations-api.env \
  --operator-email admin@example.com --password-stdin --maintenance-confirmed \
  --restore /opt/aster-operations/backups/aster-operations.sql \
  --confirm-database aster_operations
```

数据库备份与 Operations 许可证签名私钥备份必须分别受控；只有数据库而没有原签名私钥时，Customer 已签发许可证仍可验签，但无法继续以同一信任根签发新许可证。


目标环境升级还需在受保护的环境文件中配置 `ASTER_OPERATIONS_UPGRADE_CREDENTIAL_KEY_BASE64`（独立的 32 字节标准 Base64 密钥）。它用于加密保存目标管理授权和探测 Key，不能与密文一起放入数据库；密钥必须随部署配置单独备份。安装／升级时应用新的 Operations 增量迁移，以建立环境任务表并扩展权限约束。操作与旧环境首次接入条件见 [Operations 指引](../../docs/operations-guide.md#目标环境维护升级)。
