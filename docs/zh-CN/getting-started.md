# 安装与首次使用

[English](../getting-started.md) · [中文文档](README.md)

本指南帮助管理员从校验安装包开始，完成首个模型请求。

## 1. 安装前准备

准备以下内容：

- 与目标平台和架构匹配的安装包；
- 通过可信渠道取得的安装包 SHA-256；
- Linux 上的 `sudo` 权限，或 Windows 管理员 PowerShell；
- 能访问 Model API、Member UI 和 Admin UI 的网络；
- 接入 ChatGPT 订阅账号时，服务器能够通过 HTTPS 访问 `auth.openai.com` 和 `chatgpt.com`。

安装包名称为 `aster-team-<版本>-<平台>-<架构>.tar.gz`。实际支持的平台以每个 [Release](https://github.com/huzz-open/aster-team/releases) 列出的文件为准，不要根据第三方文件推断支持范围。

## 2. 校验下载文件

运行压缩包内的任何程序前，先完成[发行文件校验](release-verification.md)。初始化时 Aster 还会校验包内签名发行树；外层摘要和包内签名用途不同，两项都应保留。

## 3. 在 Linux 上安装

把占位值替换成实际文件名和从独立可信渠道取得的摘要：

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<可信的SHA-256>' "$archive" | sha256sum --check --strict -
tar -xzf "$archive"
cd "${archive%.tar.gz}"
sudo ./init.sh
sudo aster-team-cli install
```

默认安装根目录是 `/opt/aster-team`。其他根目录只能在首次初始化时指定：

```bash
sudo ./init.sh --install-root /data/aster-team
sudo aster-team-cli install
```

## 4. 在 Windows 上安装

在管理员 PowerShell 中运行：

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
$actual = (Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actual -ne '<可信的SHA-256>') { throw 'SHA-256 不匹配' }
tar.exe -xzf $archive
Set-Location 'aster-team-<version>-windows-amd64'
.\init.ps1
```

继续执行 `init.ps1` 打印的 Control 绝对路径命令。默认根目录为 `C:\ProgramData\Aster Team`；其他根目录只能在首次初始化时指定：

```powershell
.\init.ps1 --install-root 'D:\Aster Team'
```

初始化后不要继续调用下载解压目录中的 CLI，应使用脚本打印的已安装绝对路径。

## 5. 打开各入口

默认 IP + HTTP 入口为：

| 接口 | 默认地址 |
| --- | --- |
| Model API | `http://服务器IP:11080` |
| Member UI | `http://服务器IP:11081` |
| Admin UI | `http://服务器IP:11082` |

明文 HTTP 只适合可信局域网。在公网或不可信网络开放任何入口前，必须配置 HTTPS。

安装程序会打印初始 Owner 凭据，并在 `<安装根>/config/control/initial-owner-credentials` 保存一份。首次登录后立即修改密码，安全保存新密码并删除初始凭据文件。

## 6. 许可证、Runner 和账号

1. 在“产品授权”确认包内免费 License 或导入的签名 License 有效；当前权益以页面显示为准。
2. 在“Runner 节点”确认至少一个 Runner 在线；交互安装时可选择同机 Runner。
3. 在“订阅/账号”发起 ChatGPT OAuth 并保存授权账号。
4. 同步模型，并开放允许成员使用的模型。
5. 创建成员、发放额度，再由成员在 Member UI 创建 API Key。

## 7. 发起首个请求

使用 `/v1/models` 返回的模型 ID：

```bash
curl --fail-with-body -sS 'https://aster.example.com/v1/responses' \
  -H 'Authorization: Bearer <成员API-Key>' \
  -H 'Content-Type: application/json' \
  -H 'X-Request-ID: quickstart-001' \
  --data-binary '{"model":"<已开放模型ID>","input":"你好","stream":false}'
```

接下来可阅读 [API 参考](api-reference.md)、[Codex 接入](codex.md)或 [Claude Code 接入](claude-code.md)。
