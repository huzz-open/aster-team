# 管理员指南

[English](../admin-guide.md) · [中文文档](README.md)

## 推荐配置顺序

1. **产品授权**：确认包内免费 License，或导入签名 License；免费和付费权益使用同一套产品。
2. **平台设置**：配置对外 Model API 地址，向成员提供前确认 TLS 正常。
3. **Runner 节点**：至少保持一个兼容 Runner 在线；需要时增加容量或故障域。
4. **订阅/账号**：授权 ChatGPT 订阅账号并同步模型。
5. **模型管理**：只开放团队允许使用的模型。
6. **成员与额度**：为每个成员建立独立身份，并按团队策略发放额度。
7. **审计与用量**：持续查看消费、账号健康、Runner 可用性和安全相关变更。

## 许可证

全新安装可以自动导入包内免费 License。也可以在主机上导入单独交付的签名 License：

```bash
sudo aster-team-cli license install --source ./license.json
sudo aster-team-cli license status
```

许可证完全在本地验证。导入另一份有效 License 可以直接变更权益，不必重新安装。当前限制始终以“产品授权”页面显示为准。

## 成员与凭据

为每个人员或自动化身份创建独立成员和 API Key，不要让整个团队共用一个成员 Key；只有独立身份才能实现有效的撤销、额度控制和事件复盘。

重置成员密码时，Aster 会生成只显示一次的临时密码并撤销既有会话，应通过安全渠道交付。管理员无法登录时，在 Control 主机执行：

```bash
sudo aster-team-cli password reset-admin
```

可用 `--email admin@example.com` 指定邮箱；新密码仍通过隐藏输入填写。

## 账号与模型

发起 ChatGPT OAuth 前，确认主机能通过 HTTPS 访问 `auth.openai.com` 与 `chatgpt.com`，并且至少有一个 Runner 在线。授权成功后同步账号模型，再明确开放允许成员使用的模型。

上游账号凭据只由管理员管理。成员通过 Aster API Key 使用已开放模型，不需要接触上游凭据。

## 额度与用量

请求执行前会预留额度，执行后根据可信 usage 结算。没有发送到上游的请求不扣除额度。排查异常消费时，应结合 Admin 的用量与审计页面，而不是只看上游汇总。

## Runner 部署位置

简单部署可使用同机 Runner。需要独立出口、地域、主机隔离或更多容量时，可增加专用 Runner。Runner 主机受信任并能看到它转发的内容，应据此选择部署位置。
