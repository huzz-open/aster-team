# Aster Team 免费 License、付费 License 与离线激活指南

[English](../../guides/aster-team-license-guide.md) · [部署指南](README.md) · [管理员指南](../admin-guide.md)

Aster Team 的免费和付费部署使用同一个安装包。产品能力与资源上限来自本地验证的签名 License；升级为付费 License 不需要重新安装网关。

## 使用随包免费 License

Release 包含有效的随包免费 License 时，新安装的 Control 会自动验证并激活。安装完成后打开“管理端 → 产品授权”，或运行：

```bash
sudo aster-team-cli license status
```

当前随包免费 License 开放基础的账号接入、成员协作和 Runner 执行流程，其签名资源上限为：

| 资源 | 免费上限 | 如何释放容量 |
| --- | ---: | --- |
| 成员席位 | 3 | 停用会使用模型的成员 |
| Runner | 1 | 删除 Runner；离线或停用仍占用容量 |
| 订阅/账号 | 1 | 删除逻辑账号；停用或失效仍占用容量 |
| 每位成员的 Active API Key | 1 | 撤销旧 Key |

实际安装应以“产品授权”页面为准。Aster License 只控制 Aster 功能和上限；上游账号、订阅、积分与服务商费用仍然独立。

## 完成离线激活

未自动安装有效 License 时，在安全目录生成离线申请：

```bash
sudo aster-team-cli license request
```

命令会创建带时间戳的 JSON 申请与二维码图片。按照约定渠道把其中一种交给授权联系人。收到签名 `license.json` 后，在“管理端 → 产品授权”导入，或运行：

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

License 在客户主机本地验证，运行时不需要连接 Aster 授权服务器。

## 从免费版升级为付费版

把签名付费 License 导入现有安装即可。成员、账号、模型、用量记录、设置和 Runner 身份都会保留。刷新管理端授权状态，不需要重装或清空数据。

可以提前导入未来生效的续费 License。管理端会在当前 License 继续有效时把它显示为“下一份授权”，到达签名生效时间且当前 Aster 版本满足最低版本后自动启用。

## 付费 License 到期后

已有授权数据仍可用于查看和恢复操作。新的模型请求以及会增加授权资源用量的操作会暂停，直到有效续费 License 生效。

到期不会自动切换为免费版。如需继续使用免费额度，在“产品授权 → 切换免费版”检查成员、Runner、订阅/账号和每人 Key 数量，把各项降至签名免费上限后确认切换。停用 Runner 或账号不会释放实体上限；不再需要时应删除。

## 常见授权错误

- `51001 LICENSE_MISSING`：安装随包或已签发的签名 License。
- `51002 LICENSE_EXPIRED`：导入续费 License，或整理资源后切换免费版。
- `51005 LICENSE_MACHINE_MISMATCH`：为当前安装身份重新申请 License。
- `51006 LICENSE_VERSION_INVALID`：升级到 License 要求的最低 Aster 版本。
- `51008 FEATURE_NOT_LICENSED`：在“产品授权”检查当前功能列表。

其他安装和 API 错误见[Aster Team 错误码](aster-error-codes.md)。
