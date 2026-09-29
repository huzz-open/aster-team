# GHCR 编译与运行测试镜像

本地与 GitHub Actions 共用 `huzz-open` 命名空间下可匿名拉取的公开 GHCR 镜像，固定到 digest，不使用 `latest`，
不维护国内副本。镜像不是客户安装包，源码、产品数据库、密钥和登录凭据都不写进镜像。

## 两类环境不能混用

- `linux-builder`：Node 22.19.0、Go 1.25.14、Rust 1.95.0，以及 C/C++、musl、Perl、
  pkg-config、Python、binutils 等编译工具。项目依赖仍按锁文件安装和缓存。
- 六个运行镜像：Ubuntu 20.04/22.04/24.04、Debian 12/13、Rocky Linux 9。
  只包含声明的 systemd 安装测试基线，不包含编译工具链。Actions 原有宿主机测试继续保留；
  本地完整测试将上述发行版分别放入独立运行容器。

运行基线保留产品安装器明确要求的 `curl`、用户/服务管理、解压和 shell 命令；
`openssl` 是现有安装烟测生成密码与证书夹具的辅助工具，不代表产品可以动态依赖其库。
它们的传递依赖由发行版决定，完整软件包版本清单写入 `/usr/share/aster-ci/packages.tsv`。
镜像不额外安装 OpenSSL/SQLite 开发包或其他用于补救产品链接错误的库。

测试前后均核对系统包清单，并拒绝编译器进入运行容器。不得为让测试通过而运行 apt/dnf
补包、挂载宿主机库或设置库搜索路径。最终 tar.gz 中的每个 `bin/` 文件在启动测试前做
ELF 检查：动态解释器、外部共享库依赖、错误架构和畸形文件都失败。发布检查继续保留
readelf 检查。缺依赖必须修复构建/打包；新增客户前置条件需要单独对齐，不能暗改测试基线。
这些检查不能替代完整的功能测试，例如运行过程中按需加载资源仍需通过实际场景验证。

## 本地维护命令

要求 Node.js 22+、Linux Docker Engine（Windows 可用 Docker Desktop 的 Linux 模式）。
systemd 验证使用 disposable privileged 容器，只在隔离开发 Docker 主机或 VM 上运行。

首次在自己的终端执行 `docker login ghcr.io`，使用个人 GitHub 用户名及 PAT classic
的 `write:packages` 权限。个人用户需要具有组织发布权限。不把令牌写入源码、命令参数或聊天。
GitHub CLI 登录和 Docker 登录独立，Actions 使用自己的 `GITHUB_TOKEN`，不保存个人 PAT。

```sh
npm run ci:images:plan
npm run ci:images:update
```

也可以分开执行，或仅维护一个目标：

```sh
npm run ci:images:build
npm run ci:images:build -- --only=ubuntu-20.04
npm run ci:images:publish
```

每个镜像通过验证后独立保存 `target/ci-base-images/<target>.json`。网络失败后：

```sh
npm run ci:images:resume
npm run ci:images:publish
```

在源码任务工作树运行镜像维护时，可设置 `ASTER_CI_IMAGES_RECEIPT_DIR` 为固定集成工作树下的
`target/ci-base-images` 绝对路径，让验证记录和可复用缓存留在集成工作树；锁文件仍写入当前源码工作树。

`resume` 复用配方、验证代码与本地 image ID 均未变化的成功记录，仅继续未完成的目标。
配方或验证代码变化时，会从匹配的 Docker 构建层重新构建并重新验证，不能直接复用成功记录。
未完成验证的目标可以复用 Docker 已完成的构建层，但仍必须通过全部镜像验证；
`resume` 用于恢复工作，不代表刷新 apt/dnf 安全更新，主动更新应使用 `build` 或 `update`。
`publish` 不重复编译和 systemd 验证。它先确认全套本地记录，再逐个上传并按 digest 拉回核对。
七个镜像全部成功后，才原子更新 `tools/ci-base-images.lock.json`，不部分切换矩阵。
仅上传失败时直接重跑 `publish`；已上传的内容由 Docker 内容寻址去重复。

`build` 显式刷新源镜像并禁用层缓存，避免维护更新时仍使用旧 apt/dnf 层。
网络较慢时，可在维护命令的进程环境中设置 `ASTER_CI_DEBIAN_MIRROR`、
`ASTER_CI_DEBIAN_SECURITY_MIRROR`，或对应的 `ASTER_CI_UBUNTU_MIRROR`、
`ASTER_CI_UBUNTU_SECURITY_MIRROR`；这些值只影响构建时下载源，不写入锁文件。
编译镜像的 Node/Go/Rust 源镜像 digest 在 Dockerfile 固定，升级工具链必须同步仓库版本契约。
构建上下文限定在 `scripts/ci/systemd`，编译 Dockerfile 也不接收产品源码上下文。
脚本不自动提交、不推送 Git、不触发 Actions、不删除历史镜像。

## 消费与交付

`ci:images:check` 检查全部目标、镜像种类、命名空间、digest 和配方是否一致。
测试入口只拉取并使用锁文件中的镜像，找不到或无法拉取就失败，绝不回退现场构建。
本地 `linux-lab.sh` 用编译镜像构建，再把成品交给全新运行容器；E2E 的 Customer 服务只使用
运行镜像。模拟上游是单独的测试应用，可以构建自己的测试代码，不给 Customer 安装依赖。

Actions 的 Linux Customer 编译使用 `npm run ci:builder -- 命令`，运行测试不在编译容器内。
本地 `release:local` 的 Linux Docker 构建入口也只消费同一个预构建镜像，不再现场构建基础镜像。
Windows 仍使用 Windows runner；浏览器、Operations 等独立测试依赖不属于此 Linux 镜像方案。
GHCR 新包首次发布默认私有；用于公开构建的七个固定摘要镜像必须逐一设为 Public，并在未登录的环境中验证可拉取。镜像只含工具链或系统测试基线，不含产品源码、密钥、凭据和客户数据。上传仍需维护者的包写权限，公开读取不授予写权限。

首次上线前必须完成真实镜像验证、匿名拉取、工作流语法检查与仓库规定验证，再经 PR 交付。
更新或回退均提交真实锁文件和相应配方；保留仍被历史提交引用的镜像。
