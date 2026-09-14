# 发行文件校验

每个 Aster Team Release 应同时提供安装包和 `SHA256SUMS`。首次信任摘要时，应从独立可信渠道核对发行页公布的 SHA-256，不能只信任与安装包来自同一下载位置的校验文件。

Linux 示例：

```bash
sha256sum --check --strict SHA256SUMS
```

Windows PowerShell 校验 Windows 实验包的示例：

```powershell
(Get-FileHash -Algorithm SHA256 .\aster-team-<版本>-windows-amd64.tar.gz).Hash.ToLowerInvariant()
```

输出必须与 Release 中对应文件的 64 位小写十六进制摘要完全一致。文件名、大小或摘要不一致时不要运行安装程序，并通过 Issue 说明下载来源、文件名和实际摘要；不要上传 License、凭据或其他敏感文件。

如果在 Windows 上为 Linux 服务器下载文件，请将上述文件名替换为实际的 `linux-amd64.tar.gz` 文件名；两种平台的压缩包不能混用。

解压后，Linux 的 `init.sh` 或 Windows 的 `init.ps1` 还会验证包内签名发行树。外层文件摘要和包内签名分别解决下载完整性与发行内容验证，两项都应保留。
