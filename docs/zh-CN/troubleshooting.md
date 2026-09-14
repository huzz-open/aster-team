# 故障排查

[English](../troubleshooting.md) · [中文文档](README.md)

首先执行：

```bash
sudo aster-team-cli status
sudo aster-team-cli doctor --verbose
```

Windows 使用已安装 `aster-team-cli.exe` 的绝对路径，并去掉 `sudo`。

## 服务运行但页面打不开

确认 `status` 显示的是真实服务器地址而不是 `127.0.0.1`，再检查监听端口、主机防火墙、云安全组、路由和 DNS：

```bash
sudo aster-team-cli status
ip -4 route get 1.1.1.1
sudo ss -lntp | grep -E ':(11080|11081|11082)\b'
```

域名部署还需要正确的 DNS 或客户端 hosts 映射，以及有效 TLS 配置。

## `RUNNER_NOT_READY` / `33003`

路由时没有同时满足在线、启用、协议兼容、心跳新鲜和容量要求的 Runner。在“Admin → Runner 节点”点击“测试连通性”，再查看两端日志：

```bash
sudo aster-team-cli logs runner
sudo aster-team-cli logs control
```

同时检查 Runner 的 DNS、TLS、代理、防火墙以及对已配置上游域名的出站访问。返回 `33003` 的请求不会扣费。

## 提示缺少 `GLIBC_2.xx`

当前包是旧的动态链接构建。现行 Linux 发行包应为静态构建。请从 Releases 重新下载最新签名包并再次校验，不要继续使用旧解压目录。

## 提示 License 文件不存在

`license install` 只导入已经签发的许可证，不会创建许可证。先检查路径：

```bash
pwd
ls -l ./license.json
sudo aster-team-cli license install --source ./license.json
```

尚未取得签名 License 时，可使用包内免费 License，或按照 CLI 与 Admin UI 显示的离线申请流程操作。

## Codex 仍在使用其他 Provider

完全退出 Codex，然后运行：

```powershell
asterctl status codex
asterctl doctor codex
```

确认 Base URL 以 `/v1` 结尾、`ASTER_API_KEY` 存在于同一操作系统用户下，而且 API Key 仍然有效。详见 [Codex 接入](codex.md)。

## Claude Code 请求路径错误

Base URL 必须是不带 `/v1` 的服务根地址。重新使用 `https://aster.example.com` 执行 setup，不要填写 `https://aster.example.com/v1`。详见 [Claude Code 接入](claude-code.md)。

## Windows Git Bash 发送中文 JSON 失败

Member 页面中的 curl 示例按标准 curl 行为生成。如果英文正常、中文 JSON 返回 `INVALID_REQUEST`，先检查 Git Bash 实际调用的程序：

```bash
type -a curl
curl --version
```

保持 JSON 和请求头不变，改用 Windows 自带 curl 重试：

```bash
/c/Windows/System32/curl.exe --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H "Authorization: Bearer ${ASTER_API_KEY}" \
  -H 'Content-Type: application/json' \
  --data-binary '{"model":"<已开放模型ID>","input":"你好","stream":false}'
```

## 提交有效的故障信息

模型请求失败时，复制 `X-Aster-Request-ID` 或错误正文中的 request ID，再在 Control 主机执行：

```bash
sudo aster-team-cli trace
```

提供准确时间与时区、操作入口或命令、模型、五位错误编号与字符串码、脱敏后的 `status` 和 `doctor` 输出，以及是否使用 HTTP、HTTPS、域名、代理或内部 CA。

不要提交密码、Cookie、API Key、Access Token、Refresh Token、License 文件、私钥、数据库、完整对话或未脱敏诊断包。脱敏后可提交[安装问题](https://github.com/huzz-open/aster-team/issues/new?template=installation.yml)或[软件问题](https://github.com/huzz-open/aster-team/issues/new?template=bug.yml)。
