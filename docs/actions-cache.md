# GitHub Actions 依赖缓存

缓存是可重建的加速数据，不是发布制品，也不替代安装、编译或验证。

- npm：所有安装任务使用同一个本地 composite action。按系统、架构、Node 主版本和 lockfile 区分下载缓存；每次仍执行 `npm ci`，成功后立即保存，不等待后续测试。安全预检不再写入只有 audit 内容的空 npm 缓存。
- Rust：沿用 rust-cache 的编译器、锁文件、工作目录和 job 隔离。Windows 静态 CRT 与原生运行时版本额外进入 key；只在 main 保存。后续安装测试失败时仍保存已完成的依赖编译；不缓存本项目的 release 二进制来跳过重新编译。
- 下载：Windows Perl 运行时准备成功后立即保存；Caddy 下载在基础包构建成功后保存。恢复后继续执行已有的完整性校验。
- Go：真正编译 Go 的任务保留缓存；只需要 gofmt 的 Linux 客户包构建关闭 Go 缓存。

## 容量维护

`Maintain Actions dependency caches` 在 main 上的客户发布、域验证、系统测试结束后独立执行，不影响发布门禁。脚本仅管理列出的已知缓存命名空间：

1. 清除已确认关闭的 PR 缓存和旧的空 npm 缓存。
2. 新 npm 缓存存在后移除旧 npm 命名空间。
3. main 每类保留最新缓存和一个历史回退；超过 8 GB 目标时按创建时间移除历史回退。
4. 保护每类最新 main 缓存，不删除未知缓存、未确认状态的 PR 缓存或其他分支缓存。无法满足预算时明确报告，不强制删光。

8 GB 是清理目标，不是存储配额设置；为默认约 10 GB 配额留出增长余量。并发任务可能在清理后继续写入，因此并非硬上限。大型单份缓存仍可能需要后续依据实际尺寸再细分。

本地先登录 GitHub CLI，在仓库根目录执行同一脚本：

```sh
npm run test:ci-cache
npm run ci:cache:prune
```

第二条默认只预览，显示精确缓存 ID、原因和预计空间。确认后执行：

```sh
npm run ci:cache:prune -- --apply
```

需要仓库 Actions 缓存写权限。删除只影响可重新下载或编译的数据，不删除安装包、Release 或源码。完整本地 `npm run verify` 包含缓存契约测试；GitHub 实际恢复、上传耗时及 Linux 系统安装测试仍需在对应 runner 验证。首次使用新 key 会冷启动，第二次相同依赖的运行才适合比较。
