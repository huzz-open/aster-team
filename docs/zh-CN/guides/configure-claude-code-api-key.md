# 如何使用 Aster API Key 配置 Claude Code

[English](../../guides/configure-claude-code-api-key.md) · [开发者指南](README.md) · [Claude Code 参考文档](../claude-code.md)

Aster 为 Claude Code 提供 Anthropic Messages 兼容端点。每位开发者可使用独立的 Aster 成员 Key，管理员则统一管理模型访问、额度、路由与用量记录。

## 前置条件

- 安装 Claude Code。
- 在 Aster 成员端创建个人 API Key。
- 获取 Aster 公网服务地址。
- 确认已有管理员启用的模型和健康的 Runner。

Claude Code 使用的 Aster Base URL 必须是服务根地址，不能以 `/v1` 结尾；Claude Code 会自行追加 `/v1/messages`。

## 推荐配置方式

从 **成员端 → API 文档 → asterctl 工具** 下载 `asterctl`，然后运行：

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

将 `project-demo` 替换为绝对路径或相对当前终端的路径。省略 `--project` 可配置当前目录；省略 `--launch` 则不会启动 Claude Code。

`asterctl` 会验证地址与隐藏输入的 Key，获取管理员启用的模型映射，并把项目级配置写入 `.claude/settings.local.json`。覆盖前会展示既有设置，并在确认后创建备份。

查看、诊断或移除配置：

```powershell
asterctl status claude --project "project-demo"
asterctl doctor claude --project "project-demo"
asterctl remove claude --project "project-demo"
```

## 不要把本地配置提交到 Git

将环境相关的 Claude Code 设置与 `asterctl` 状态加入 `.gitignore`：

```gitignore
**/.claude/settings.local.json
**/.claude/.asterctl-state.json
**/.claude/settings.local.json.asterctl-*.bak
```

不要提交成员 Key。需要独立额度和审计归属时，应为每位人员、每台机器或每个 CI 边界使用不同的成员或自动化身份。

## 常见问题

- **请求路径错误：** 从 `--base-url` 中移除 `/v1`。
- **客户端版本被拒绝：** 更新 Claude Code 后重新配置。
- **缺少模型：** 管理员调整启用模型后重新执行配置。
- **认证失败：** 创建有效的成员 Key，并运行 `asterctl doctor claude`。
- **配置到了错误项目：** 使用绝对 `--project` 路径，并检查该目录的 `.claude/settings.local.json`。

完整行为和移除语义见[通过 Aster 使用 Claude Code](../claude-code.md)。
