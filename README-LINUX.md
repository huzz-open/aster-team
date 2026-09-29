# Linux 构建

面向开发者的 Linux amd64 Customer 构建与验证命令。用户安装说明见 [用户手册](docs/user-manual.md)。

## 常用命令

| 用途 | 命令 |
| --- | --- |
| Windows 准备 Docker/WSL lab | `npm run setup:linux-lab` |
| 检查 lab | `npm run setup:linux-lab:check` |
| 环境诊断 | `npm run test:linux:doctor` |
| Ubuntu 20.04 主验证 | `npm run test:linux:primary` |
| 完整发行矩阵 | `npm run test:linux:full` |
| 正式本地构建 | `npm run release:local -- --platform=linux` |

Linux 主机只需直接准备 Docker；Windows 使用管理员 PowerShell。正式构建还需要仓库外的签名材料、干净且同步的 `main`。

---

### AI / 自动化备注

#### 本文件用途与影响范围

本文件只提供 Linux 构建入口。生产包包含 Rust CLI、Control、Runner、Caddy 和静态前端，不包含 Node.js、Go、Rust 工具链或数据库服务端。当前正式目标是 Linux x86-64 + systemd。

`release:local -- --platform=linux` 执行源码门禁、签名、SBOM、构建和安装 smoke，通过后才写入 `dist/linux/`。不要用底层 bundle 脚本绕过发布入口。锁定工具链见 `tools/linux-build-runtime.json`，实现位于 `scripts/ci/linux-lab.sh`、`scripts/build-linux-amd64.sh` 和 `customer/deploy/`。

自动测试失败时，只能在隔离测试主机使用测试签名包检查关键入口：

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli backup restore --source <备份文件> --confirm
```

不要在客户生产环境运行测试包或破坏性 smoke。完整安装、许可证、Runner、升级、恢复和卸载操作以 `docs/user-manual.md` 为准。
