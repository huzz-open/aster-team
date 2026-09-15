# 运行、备份与升级

[English](../operations.md) · [中文文档](README.md)

以下示例使用 Linux 命令入口。Windows 应使用 `init.ps1` 打印的已安装 `aster-team-cli.exe` 绝对路径，去掉 `sudo`，其余子命令保持一致。

按任务执行维护见[备份、恢复和升级 Aster Team](guides/backup-restore-upgrade.md)；独立执行主机见[部署独立 Runner](guides/deploy-dedicated-runner.md)。

## 日常检查

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor
sudo aster-team-cli doctor --verbose
sudo aster-team-cli logs control
sudo aster-team-cli logs runner
```

`status` 汇总服务状态和公开入口；`doctor` 检查安装身份、数据库访问、许可证、公开入口和服务预检。操作系统服务正在运行，不等于上游请求一定成功。

复现问题时可以实时查看日志：

```bash
sudo aster-team-cli logs control --follow
sudo aster-team-cli logs runner --follow
```

Control 使用 Blue/Green 槽位，CLI 会自动解析当前活动槽位；运维人员不应直接操作指定槽位的服务。

## 安装独立 Runner

使用与 Control 预期版本、平台和架构一致的签名包。Linux：

```bash
sudo ./init.sh
sudo aster-team-cli runner install
```

Windows 在管理员 PowerShell 运行 `init.ps1`，再对脚本打印的已安装 CLI 命令追加 `runner install`。

在“Admin → Runner 节点”创建一次性注册 Token，并执行页面生成的命令。Token 文件必须只允许管理员读取。HTTP 局域网入口需要使用页面显示的明确允许不安全 HTTP 参数；使用内部 CA 的 HTTPS 入口还需要提供公开 CA 证书。

## 备份

升级和重要配置变更前创建备份：

```bash
sudo aster-team-cli backup create
```

未指定输出路径时，备份位于 `<安装根>/backups`。备份可能包含加密数据库、身份材料和运行配置，应按敏感数据保护。

恢复前先查看当前命令约定，并确认备份属于同一安装身份：

```bash
sudo aster-team-cli backup restore --help
```

## 升级

推荐使用“Admin → 系统升级”，上传与当前平台和架构匹配的签名包。Aster 会校验包、准备非活动槽位、执行内嵌迁移和健康检查，只在候选版本健康后切换流量；失败时当前版本继续服务。

也可以在主机上执行：

```bash
sudo ./init.sh
sudo aster-team-cli upgrade
```

升级会保留安装身份、本地数据库、许可证、订阅/账号、设置和 Runner 身份。数据库只向前迁移；升级成功后不要手动切回旧程序。

## Windows 多实例

同一台 Windows 主机需要隔离测试实例时，应在首次 `install` 或 `runner install` 前选择独立安装根、任务命名空间和端口段：

```powershell
$env:ASTER_SERVICE_PREFIX = 'lab'
$env:ASTER_PORT_OFFSET = '10000'
```

本例将公开端口 `11080`–`11082` 移到 `21080`–`21082`，并隔离计划任务名称。每个实例都必须使用不同根目录、前缀和未占用端口段。后续服务、升级、备份、恢复和卸载均以已保存的安装元数据为准。
