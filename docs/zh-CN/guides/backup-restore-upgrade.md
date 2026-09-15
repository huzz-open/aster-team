# 如何备份、恢复和升级 Aster Team

[English](../../guides/backup-restore-upgrade.md) · [部署指南](README.md) · [运维参考](../operations.md)

本运行手册适用于使用内置本地数据库的 Aster Team，介绍日常健康检查、创建备份、使用签名包升级和故障恢复。

## 维护前检查

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

升级前先处理安装身份、数据库、License、入口或服务错误。操作系统服务正在运行，并不代表完整模型请求链路一定健康。

## 创建并保护备份

```bash
sudo aster-team-cli backup create
```

未指定输出路径时，归档保存在 `<install-root>/backups`。备份包含恢复安装所需的数据库、身份与密钥材料、产品 License、访问配置、代理状态和 Runner 身份。

备份属于敏感文件。应限制访问、复制到批准的备份位置，记录创建时间与来源安装，并在使用独立根目录和端口的隔离环境中验证恢复流程。

## 从管理端升级

推荐使用“管理端 → 系统升级”：

1. 下载与当前平台、架构一致的签名安装包。
2. 校验 SHA-256；安装前 Aster 还会验证已签名的 Release 文件树。
3. 创建最新备份。
4. 安排维护窗口；本地数据库升级可能中断页面、API 和进行中的 Stream。
5. 上传安装包。浏览器暂时断开后继续观察原任务，不要重复提交升级。

Aster 会校验安装包、准备候选版本、按需停止服务、执行内嵌正向迁移、检查候选健康状态，并在成功后恢复公开入口。候选启动失败时，维护流程会尝试恢复原程序状态，并明确报告服务是否恢复健康。

## 从主机 CLI 升级

校验并解压新包后运行：

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

使用自定义安装根时，向 `init.sh` 传入原 `--install-root`，随后使用该根目录中的稳定 CLI。成功升级会保留安装身份、本地数据库、License、订阅/账号、设置和 Runner 身份。

数据库迁移只向前执行。迁移成功后，不要手动让旧版程序连接已经升级的数据库。

## 恢复备份

恢复前先查看当前 CLI 的命令约定：

```bash
sudo aster-team-cli backup restore --help
```

然后按照已安装 CLI 显示的参数指定来源并确认。确保归档属于同一安装身份、平台、安装根和实例配置。恢复用于故障恢复，不是通用的跨机器迁移方式。

恢复后运行 `status` 和 `doctor --verbose`，再测试成员端、管理端、`/v1/models` 和一次低成本模型请求。事件关闭前应保留失败维护任务的诊断信息。

独立节点应使用已安装 CLI 提供的对应 `runner backup` 与 `runner upgrade` 命令。
