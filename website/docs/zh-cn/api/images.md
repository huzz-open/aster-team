---
title: 图片生成与编辑 API
description: 区分 Aster 图片生成和编辑，了解图片张数额度、输入格式、结果交付与模型差异。
---

# 图片生成与编辑

图片是独立操作。生图使用 `POST /v1/images/generations`，编辑使用 `POST /v1/images/edits`。选择已授权的公开**图片模型**；普通文本模型不自动具有生图能力。

## 生图

将 `ASTER_BASE_URL` 设为[快速开始](/zh-cn/start/quickstart)中显示的当前实例 API 地址，并将 `YOUR_IMAGE_MODEL` 换成成员模型列表中的图片模型 ID。

<InstanceCurlExample language="zh" kind="generate" />

通用模板：

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/images/generations" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -H "Content-Type: application/json" \
  -d '{"model":"YOUR_IMAGE_MODEL","prompt":"一只站在雨夜街头的猫","n":1}'
```

```json
{"model":"YOUR_IMAGE_MODEL","prompt":"一只站在雨夜街头的猫","n":1}
```

`n` 是请求张数，具体上限依模型而定。`size`、`quality`、`output_format` 与透明背景按模型及组合验证。某厂商只返回短期 URL 时，Aster 需取回并校验图片后，才能向客户端返回统一的 `b64_json`；下载失败不能重新提交生图。

## 编辑

编辑使用 `multipart/form-data` 上传本地原图，不要手动设置 `Content-Type`；cURL 会生成包含 boundary 的请求头。仅选择支持图片编辑的模型。

<InstanceCurlExample language="zh" kind="edit" />

通用模板：

```bash
curl --fail-with-body "$ASTER_BASE_URL/v1/images/edits" \
  -H "Authorization: Bearer $ASTER_API_KEY" \
  -F "model=YOUR_EDIT_MODEL" \
  -F "prompt=把背景改成夜晚" \
  -F "image=@source.png"
```

编辑需要原图；可选 mask 受目标模型、尺寸和格式约束。GLM 通用生图接口不代表已实现图片编辑。调用前请查看当前实例对该模型开放的操作。

图片额度以张数独立管理。上游没有 token usage 时，不以估算 token 代替张数结算。

## 图片模型矩阵

选择图片模型和操作，查看字段是否可用以及取值映射。这里展示的是签名插件规则；当前实例是否开放模型，还取决于管理员配置和成员权限。

<ImageFieldMatrix language="zh" />
