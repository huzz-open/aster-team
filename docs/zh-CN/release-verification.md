# 校验发行包

[English](../release-verification.md) · [中文文档](README.md)

Aster Team Release 应包含各平台安装包和 `SHA256SUMS`。预期 SHA-256 应通过与安装包相互独立的可信渠道取得。与安装包放在一起的校验文件可以发现意外损坏，但不能单独建立首次信任。

## Linux

校验清单中的所有文件：

```bash
sha256sum --check --strict SHA256SUMS
```

也可以使用独立取得的摘要校验单个安装包：

```bash
archive=aster-team-<version>-linux-amd64.tar.gz
printf '%s  %s\n' '<可信的SHA-256>' "$archive" | sha256sum --check --strict -
```

## Windows PowerShell

```powershell
$archive = 'aster-team-<version>-windows-amd64.tar.gz'
(Get-FileHash -LiteralPath $archive -Algorithm SHA256).Hash.ToLowerInvariant()
```

输出必须与该文件名公开的 64 位小写摘要完全一致。文件名、大小或摘要不同，均不要运行安装程序。

Linux 与 Windows 压缩包不能混用。可以在 Windows 上下载 Linux 包，但应按实际 Linux 文件名校验，并且在传输时不得修改文件。

解压后，Linux 的 `init.sh` 或 Windows 的 `init.ps1` 还会验证包内签名发行树。外层摘要保护下载压缩包，包内签名验证发行内容，两项校验都应保留。

发现不一致时，通过[安装问题](https://github.com/huzz-open/aster-team/issues/new?template=installation.yml)提供下载来源、完整文件名、预期摘要和实际摘要。不要上传 License、Key、数据库或其他敏感文件。
