# 如何部署独立 Aster Team Runner

[English](../../guides/deploy-dedicated-runner.md) · [部署指南](README.md) · [架构](../architecture.md)

独立 Aster Runner 在客户控制的主机上执行获准的模型请求。它主动通过 WSS 连接 Control，并通过 HTTPS 连接配置的 AI 服务，因此 Runner 主机不需要开放入站监听端口。

## 什么时候使用独立 Runner

当团队需要不同网络路径、主机安全边界、故障域或更多请求容量时，可以部署独立 Runner。小型部署仍可使用同机 Runner。

Runner 主机能够接触它所处理的 Prompt 与响应。请只向授权人员授予操作系统管理权限，并把主机放在符合数据与网络要求的边界内。

## 安装 Runner

使用与 Runner 平台及 Control Release 兼容的签名安装包。Linux：

```bash
sudo ./init.sh
sudo aster-team-cli runner install
```

Windows 上以管理员身份运行 `init.ps1`，然后执行脚本打印的根内 CLI 命令并追加 `runner install`。

## 注册 Runner

1. 在“管理端 → Runner 节点”选择目标平台并创建一次性注册 Token。
2. 把 Token 保存到仅操作系统管理员可读的文件。Linux 使用 `0600`；Windows 仅允许 Administrators 和 SYSTEM 访问。
3. 执行管理端生成的注册命令。命令通过 `--token-file` 读取 Token，不会把它写进命令历史或进程参数。
4. 确认 Runner 已上线，并执行“测试连通性”。

注册 Token 只能使用一次且具有有效期。失败或已经完成注册后，应新建 Token，而不是重复使用旧值。

## 配置 Control 信任

- 使用公共 CA 签发的 HTTPS 地址时，Runner 使用内置 WebPKI 信任库。
- 使用内部 CA 时，通过页面生成的 `--control-ca-certificate` 传入公开 CA 证书。
- 使用可信内网 HTTP 地址时，加入管理端提示的 `--allow-insecure-http`。不要把该入口暴露到不可信网络。

自定义 Control CA 只影响 Runner 到 Control 的注册与 WSS，不会修改 Runner 访问上游 HTTPS 时的 CA 信任。

## 理解动态路由

Runner 不会永久绑定到某个订阅或账号。所有已启用、在线、协议兼容、心跳正常且有容量的 Runner 都可成为路由候选。Aster 会优先使用有价值的近期亲和性，并在上游请求开始前选择其他健康节点。

流式响应开始后，Aster 不会静默把当前请求迁移到另一 Runner，避免把含义不确定的重放伪装成原请求。

## 排查 `RUNNER_NOT_READY`

```bash
sudo aster-team-cli runner status
sudo aster-team-cli logs runner
sudo aster-team-cli logs control
```

检查启用状态、协议兼容性、心跳、容量、DNS、TLS、防火墙、代理和上游连通性。`RUNNER_NOT_READY`（`33003`）表示没有候选节点同时满足全部路由条件，该次拒绝不会扣除额度。
