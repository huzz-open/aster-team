# 通过 Aster 使用 Claude Code

[English](../claude-code.md) · [中文文档](README.md)

Aster 为 Claude Code 提供 Anthropic Messages 兼容接口。Base URL 必须使用不带 `/v1` 的服务根地址；Claude Code 会自行追加 `/v1/messages`。

Claude Code 与上游模型服务仍是第三方产品。Aster 只提供兼容网关链路，不包含服务商账号、订阅、客户端授权、网络接入或上游权益，详见[服务边界](service-boundaries.md)。

## 推荐配置方式

从“Member UI → API 文档 → asterctl 工具”安装 `asterctl`，然后运行：

```powershell
asterctl setup claude --base-url "https://aster.example.com" --project "project-demo" --set-key --launch
```

把 `project-demo` 替换成绝对路径或相对于当前终端的路径。省略 `--project` 时使用当前目录；省略 `--launch` 时只配置，不启动 Claude Code。

`asterctl` 会检查已安装的 Claude Code 版本，验证 Aster 地址与成员 API Key，取得管理员已开放的模型映射，并在所选项目中写入 `.claude/settings.local.json`。覆盖已有配置前会显示目标文件，并在确认后创建备份。

## 检查或撤销配置

```powershell
asterctl status claude --project "project-demo"
asterctl doctor claude --project "project-demo"
asterctl remove claude --project "project-demo"
```

项目级配置包含环境专属值时，不要提交到版本库。任何情况下都不要提交成员 API Key。

## 模型映射

Aster 根据管理员当前开放的模型生成 Claude Code model overrides，成员无需单独维护映射。开放模型发生变化后，应重新运行 setup。

如果 setup 拒绝当前 Claude Code 版本，先升级 Claude Code 再重试。如果请求路径错误，确认 `--base-url` 使用不带 `/v1` 的服务根地址。
