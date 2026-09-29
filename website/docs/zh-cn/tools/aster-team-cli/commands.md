---
title: "aster-team-cli 命令参考"
description: "aster-team-cli 的完整公开子命令、参数和使用场景索引。"
---

# aster-team-cli 命令参考

按任务选择命令；点击命令名称查看参数、示例和运行影响。

## 安装与升级

- [`install`](/zh-cn/tools/aster-team-cli/install)：从已初始化并选中的发布包安装 Control 主机。
- [`upgrade`](/zh-cn/tools/aster-team-cli/upgrade)：使用新发布包选中的版本升级已有 Control 安装。

## 运行与排查

- [`info`](/zh-cn/tools/aster-team-cli/info)：查看主机角色、版本、访问入口和重要路径。
- [`status`](/zh-cn/tools/aster-team-cli/status)：查看服务、许可证与数据库运行状态。
- [`doctor`](/zh-cn/tools/aster-team-cli/doctor)：诊断主机、安装身份、数据库、许可证、TLS 与服务。
- [`logs`](/zh-cn/tools/aster-team-cli/logs)：读取 Control 或 Runner 的服务日志。
- [`trace`](/zh-cn/tools/aster-team-cli/trace)：按 Aster request ID 查找活动 Control 服务中的请求日志。
- [`version`](/zh-cn/tools/aster-team-cli/version)：显示 CLI 构建信息和已安装产品版本。

## 服务管理

- [`service start`](/zh-cn/tools/aster-team-cli/service-start)：启动指定的 Aster 服务。
- [`service stop`](/zh-cn/tools/aster-team-cli/service-stop)：停止指定的 Aster 服务。
- [`service restart`](/zh-cn/tools/aster-team-cli/service-restart)：重启指定的 Aster 服务。

## 许可证与管理员

- [`license request`](/zh-cn/tools/aster-team-cli/license-request)：生成当前 Control 主机的授权申请与二维码。
- [`license install`](/zh-cn/tools/aster-team-cli/license-install)：导入已经签发的 Control 许可证。
- [`license status`](/zh-cn/tools/aster-team-cli/license-status)：查看当前 Control 许可证状态。
- [`password reset-admin`](/zh-cn/tools/aster-team-cli/password-reset-admin)：在 Control 主机重置管理员密码并撤销其会话。

## 备份与恢复

- [`backup create`](/zh-cn/tools/aster-team-cli/backup-create)：创建Control 本地数据库安装备份。
- [`backup restore`](/zh-cn/tools/aster-team-cli/backup-restore)：恢复Control 本地数据库安装备份。

## 独立 Runner

- [`runner install`](/zh-cn/tools/aster-team-cli/runner-install)：安装独立 Runner 主机。
- [`runner enroll`](/zh-cn/tools/aster-team-cli/runner-enroll)：使用注册令牌把独立 Runner 加入 Control。
- [`runner status`](/zh-cn/tools/aster-team-cli/runner-status)：检查本机 Runner 服务及身份文件是否存在。
- [`runner upgrade`](/zh-cn/tools/aster-team-cli/runner-upgrade)：升级独立 Runner 主机的发布版本。
- [`runner backup create`](/zh-cn/tools/aster-team-cli/runner-backup-create)：创建独立 Runner备份。
- [`runner backup restore`](/zh-cn/tools/aster-team-cli/runner-backup-restore)：恢复独立 Runner备份。

## 卸载

- [`uninstall`](/zh-cn/tools/aster-team-cli/uninstall)：卸载服务与程序，默认保留业务数据。

## 帮助与退出码

```text
aster-team-cli --help
aster-team-cli install --help
aster-team-cli --version
```

帮助和版本查询正常结束时返回 0，命令运行失败返回 1，参数解析失败通常返回 2。业务状态仍需阅读输出，例如 trace 没有匹配记录也会成功退出。
