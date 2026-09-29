---
title: "asterctl 故障排查"
description: "排查 asterctl 找不到命令、客户端版本、认证、模型目录及配置冲突问题。"
---

# asterctl 故障排查

## 命令找不到

在工具目录使用 `.\asterctl.exe version`，或按 [安装说明](/zh-cn/guides/asterctl) 加入用户 PATH 后重新打开终端。不要在服务端运行成员工具来配置另一台电脑。

## 认证或连接失败

先确认使用的是 Aster 成员 Key。Codex 地址带 `/v1`，Claude 根地址不带 `/v1`。确认客户端能访问 API 入口，并检查证书信任、Key 状态和成员模型权限。`status` 不联网，检查通过不代表远端认证成功；使用对应 `doctor`。

## Codex 运行中或模型目录校验失败

配置和移除前完全退出 Codex。核对安装的客户端是否提供工具所需的 `debug models` 能力；升级客户端后重新执行 [setup codex](/zh-cn/tools/asterctl/setup-codex)。切换 CODEX_HOME 前应先处理旧目录的托管状态。

## Claude 版本或平台映射变化

先运行 `claude update`，再对同一项目执行 [setup claude](/zh-cn/tools/asterctl/setup-claude)。平台映射变更后，doctor 会指出保存的设置与当前规则不一致；它不会自动重写文件。

## 移除失败或发现手动修改

Codex 会保留冲突字段，但可能已撤销其他托管字段；Claude 在文件摘要不一致时拒绝覆盖。先保存并检查自己的修改以及原备份，不要直接删除状态文件来绕过冲突。具体行为见对应 remove 命令。

## 配置成功但没有启动客户端

检查是否传入 `--launch`。启动阶段失败时先执行 status，确认设置已完成，再手动启动客户端。不要假设非零退出码代表配置完全未改变。

## 提交问题信息

提供 `asterctl version`、具体子命令、客户端版本、失败时间及脱敏后的错误。不要提交真实 Key、完整配置或含凭据的备份。

[完整命令参考](/zh-cn/tools/asterctl/commands)
