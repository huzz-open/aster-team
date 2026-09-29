# Aster Team Linux amd64 安装 / Installation

## 中文

要求 Linux x86-64、systemd、常见 GNU 工具和 `sudo` 权限。先用独立可信渠道取得的 SHA-256 校验完整压缩包：

```bash
archive=aster-team-<版本>-linux-amd64.tar.gz
printf '%s  %s\n' '<SHA-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
sudo ./init.sh
sudo aster-team-cli install
```

Linux 默认安装根目录为 `/opt/aster-team`。需要其他目录时，只在首次初始化增加：

```bash
sudo ./init.sh --install-root /data/aster-team
```

程序、版本、配置、密钥、SQLCipher 数据、状态、升级暂存、备份和日志都位于该根目录。系统目录只保留 systemd 注册项和 `/usr/local/bin/aster-team-cli` 命令链接。

Linux amd64 的外部数据库新安装配置链路已加入源码（MariaDB 11.8.6、TLS 证书与主机名验证、安装身份绑定），尚待签名包实机验收；此阶段同样只开放维护升级。具体参数及备份边界以仓库用户手册的“Linux 外部数据库安装”为准。SQLite／SQLCipher 不支持不停服升级。

初始管理员密码会打印一次，并保存到 `<安装根>/config/control/initial-owner-credentials`。首次登录改密后删除该文件。安装结束会生成离线授权申请；取得 `license.json` 后在 Admin 页面上传，或执行：

```bash
sudo aster-team-cli license install --source ./license.json
```

独立 Runner 使用同一个包，但应指定独立根目录：

```bash
sudo ./init.sh --install-root /opt/aster-team-runner
sudo aster-team-cli runner install
```

Control 的内置 SQLite／SQLCipher 部署只支持维护升级，不支持不停服。Admin“系统升级”页面按后端能力接受签名 `.tar.gz`；先停止旧业务服务，再启动候选并执行内嵌迁移与健康检查，之后恢复访问入口。管理端、用户端、API Key 调用及正在进行的 stream 可能中断。失败时先停止候选，再尝试恢复原版本；恢复失败会明确报错，数据库不会自动回退。

旧版本首次过渡必须用新签名包更新 CLI 后执行维护升级，不能假设旧执行器已采用这一顺序。保持原安装根，CLI 入口为：

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

默认备份写入 `<安装根>/backups`：

```bash
sudo aster-team-cli backup create
sudo aster-team-cli backup restore --source <备份文件> --confirm
```

完整目录、访问方式、升级和恢复说明见 `docs/user-manual.md`。

## English

Requires Linux x86-64, systemd, common GNU tools, and sudo access. Verify the complete archive with a SHA-256 obtained through an independent trusted channel:

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<SHA-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
sudo ./init.sh
sudo aster-team-cli install
```

The Linux installation root defaults to `/opt/aster-team`. Select another root only during first initialization:

```bash
sudo ./init.sh --install-root /data/aster-team
```

Programs, releases, configuration, keys, SQLCipher data, state, staging, backups, and logs all stay below this root. System locations contain only systemd registrations and the `/usr/local/bin/aster-team-cli` command link.

The temporary owner password is printed once and stored at `<install-root>/config/control/initial-owner-credentials`. Delete it after the first password change. Import the signed license in Admin or run:

```bash
sudo aster-team-cli license install --source ./license.json
```

A dedicated Runner uses the same package with a separate root:

```bash
sudo ./init.sh --install-root /opt/aster-team-runner
sudo aster-team-cli runner install
```

For Control upgrades, upload the signed `.tar.gz` in Admin's System upgrade page. The candidate runs embedded migrations and health checks before traffic switches; a failed candidate leaves the old release online. The CLI alternative is:

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

Backups default to `<install-root>/backups`:

```bash
sudo aster-team-cli backup create
sudo aster-team-cli backup restore --source <archive> --confirm
```

See `docs/user-manual.md` for the complete directory, access, upgrade, and restore contract.
