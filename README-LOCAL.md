# 本地开发

## 准备与启动

```bash
npm run setup
npm run setup:check
npm run dev:manager
```

| 常用参数 | 作用 |
| --- | --- |
| `npm run setup -- --yes` | 使用默认值；Windows 要求已经完成 GitHub 认证 |
| `npm run setup -- --install-root "D:\AsterDev"` | Windows 指定统一工具根目录 |
| 环境变量 `ASTER_TOOLS_ROOT` | 三平台覆盖统一工具根目录 |
| 环境变量 `ASTER_GO_ROOT` | 三平台只覆盖 Go 目录 |

macOS/Linux 需先安装 Node.js 22.19+、Rust、Python 3 + tkinter 和 MariaDB/MySQL。Windows 请在管理员 PowerShell 中运行，setup 会准备所需系统组件。

## 开发命令

| 用途 | 命令 |
| --- | --- |
| 启动管理器 | `npm run dev:manager` |
| 停止已识别的服务 | `npm run dev:stop` |
| 重新初始化本地业务数据 | `npm run setup:local` |
| Customer 检查 | `npm run verify:customer` |
| Operations 检查 | `npm run verify:operations` |
| 契约检查 | `npm run verify:contracts` |
| 授权交付测试 | `npm run test:license-delivery` |
| 系统测试 | `npm run test:system-e2e` |

## 发布命令

```bash
npm run release:local -- --platform=windows
npm run release:local -- --platform=linux
npm run release:local -- --platform=all
```

发布要求和密钥配置见 [Operations 指南](docs/operations-guide.md)。

---

### AI / 自动化备注

#### 本文件用途

本文件只列开发机准备、启动、测试和发布入口。用户安装与排错写入 `docs/user-manual.md`；架构说明写入 `docs/architecture.md`。

#### setup 影响范围

- `npm run setup:check` 只读检查。
- Windows setup 可安装 Git、gh、Python、Rust、MSVC、MariaDB、Go 和 Node.js 依赖，并修改当前用户的工具环境。
- macOS/Linux setup 不安装系统包，只准备 Node.js 依赖、统一 Go 工具链和 Go 模块。
- `ASTER_TOOLS_ROOT` 是跨平台统一覆盖；`ASTER_GO_ROOT` 是兼容性的精确覆盖，不属于任何单一平台。
- 默认工具根：macOS 为 `~/Library/Application Support/AsterDev`；Linux 为 `${XDG_DATA_HOME:-~/.local/share}/aster-dev`；Windows 使用已保存的选择。

Windows 选择保存在被忽略的 `.aster-tools/windows-setup.json`。开发管理器从主 worktree 共享该目录给集成 worktree；仅当目标仍使用旧 Go 解析器时注入兼容变量。

#### 本地数据与进程

`setup:local` 会重建 Customer/Operations 本地数据库并生成仅供开发使用的密钥和安装身份。敏感文件、账号、日志和进程状态位于被忽略的 `data/`。关闭管理器窗口不会停止服务。

本地 `dev:api` 启动时会自动准备 OpenAI、DeepSeek、GLM 三个上游适配插件：首次生成仅供开发使用的 `data/local/plugin-signing.seed`，为三个插件身份配置公钥，把 `customer/plugins/shared/` 的通用 Lua 与各厂商元数据分别签包。当前开发安装根为仓库 `data/`，因此三个入口分别是 `data/data/plugins/incoming/openai.asterlua`、`deepseek.asterlua`、`glm.asterlua`。后续重启沿用同一密钥并重新打包当前源码；本地开发不需要先制作 Customer 安装包。若自行设置了 `ASTER_PLUGIN_TRUSTED_KEYS_JSON`，则需要提供三个厂商的签名公钥与对应插件包，不执行自动准备。

固定 PR 集成目录是 `../aster-team_worktrees/integration-test`。开发管理器为它使用独立数据库和端口：Operations API/Console 为 `22090`/`22080`，Customer Control/Member/Admin 为 `21080`/`21081`/`21082`，Website/Backend 为 `24080`/`18788`；主环境继续使用原端口，两套环境可以同时运行。停止服务只依据当前管理器启动时解析出的端口配置和已记录的进程身份。PR 的准确 Head SHA 和验证要求遵循 `AGENTS.md`。
