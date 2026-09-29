# 文档组织与发布

## 内容与顺序

公开源文件统一位于 `website/docs`，使用 VitePress；中文和英文路径一一对应。左侧导航按受众范围与使用频率排列：开始使用 → 部署与运维 → API 参考 → 模型与参数 → 开发与排查 → 客户端工具 → 管理员命令参考。部署指南和命令组沿用现有可折叠侧栏，当前页面所在分组由主题展开。主题、三栏布局、搜索与 API/命令页地址保持原有结构。

`asterctl` 面向成员电脑，`aster-team-cli` 面向部署主机。每个公开叶子命令有独立页面，至少覆盖用途、语法、参数条件与默认值、示例、配置影响、结果和失败处理。组命令与 `--help` 在索引说明。新增或修改 Clap 命令时同步维护两种语言及 `.vitepress/cli-navigation.ts`。

`docs/user-manual.md` 仍是规范的安装与排查手册。`website/docs/zh-cn/administration/` 的中文任务指南使用 VitePress 原生 `@include` 按命名 region 引用手册正文，不复制另一份中文教程；公开构建只提取显式区域，不发布整棵内部文档目录。网站命令参考补充逐命令契约，不改变手册的地位。安装根、平台交付状态、外部数据库限制等共同事实须保持一致。内部隐藏命令不导入公开站。

## 访问与收录

- 通用文档无需会话，管理员命令说明也公开。管理 API、实例状态、日志、凭据和下载的授权策略保持各自边界。
- 官网开发使用 `npm --workspace @aster/website run dev` 或 `dev:cf`，启动前通过共享脚本以 `--target=website` 构建至忽略的 `website/public/docs`，复用开发路由并保留原有 public 资源。文档修改后运行 `npm --workspace @aster/website run docs:prepare` 刷新，也可使用 `docs:dev` 独立热更新。官网使用 `npm --workspace @aster/website run docs:build` 构建至 `website/dist/docs`；为中英文页面生成官网 canonical、互相对应的 hreflang、x-default 和 sitemap。
- 管理端与成员端共用 `scripts/build-documentation.mjs` 构建 `website/docs`，分别随各端 `dist/docs` 交付；共用开发路由，缓存按输出目录隔离。脚本设置 `ASTER_DOCS_TARGET=customer`，所有本地文档添加 `noindex, follow`，不生成 sitemap，也不指向可能内容不同的官网最新版本 canonical。
- 本地文档与产品构建一起交付，不依赖外网即可查阅。其匿名可访问不等于允许索引；noindex 不是访问控制。
- 当前没有单独发布历史文档站。将来发布版本归档时必须使用显式版本路径并保留对应内容，不能把旧版命令页统一 canonical 到内容不同的新版本。
- 404、无语言的跳转入口不索引；新增语言页面时必须同时提供对应翻译，避免 hreflang 指向不存在的页面。

## 本地更新

从集成工作区的 `customer/admin` 或 `customer/member` 运行，更新对应端的 `.docs-public/docs`：

```text
node ../../scripts/build-documentation.mjs --target=customer --out-dir=.docs-public/docs
```

更新成员开发服务实际使用的文件，并生成静态托管所需的无扩展名路由副本。只构建 `website/dist/docs` 不会更新成员开发服务。

## 更新任务指南

1. 在 `docs/user-manual.md` 的对应命名 region 内修改中文操作正文，保留唯一的开始/结束标记；区域内不添加相对仓库路径的链接或嵌套 include。
2. 核对 `website/docs/en/administration/` 的对应英文摘要，保留条件、风险、关键命令和恢复步骤。区域外的内部技术参考不进入公开页面。
3. 英文核对完成后，更新该页 frontmatter 的 `manualSourceHash`：对中文 region 正文按 LF 换行、去掉首尾空白后计算 SHA-256。摘要用于阻止中文更新后英文无声漂移，不代表机器已验证翻译语义；禁止未核对正文就机械刷新摘要。
4. 新增主题时更新 `website/build/manual-guides.mjs` 的清单及中英文页面。缺失/重复/未闭合区域、错误 include、缺失英文或过期摘要会让 VitePress 构建失败，避免原生 include 静默保留占位符或意外展开整份手册。

在固定集成工作区可用 `node --input-type=module -e "import { readFileSync } from 'node:fs'; import { manualRegion, manualSourceHash } from './website/build/manual-guides.mjs'; console.log(manualSourceHash(manualRegion(readFileSync('docs/user-manual.md', 'utf8'), 'installation')));"` 查看已审阅区域的摘要；按实际主题替换 `installation`。

聚焦契约检查为 `node --test website/build/manual-guides.test.mjs`，网站和成员端均在原有 VitePress 构建入口执行区域/翻译检查。PR 验证继续使用 `npm run verify:changed`，没有新增框架、独立发布流程或客户运行时服务。
