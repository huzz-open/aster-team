---
title: 身份认证
description: 使用 Aster API Key 访问模型接口。
---

# 身份认证

在成员页面创建 API Key，并将它保存在服务端环境变量中。不要把 Key 写入浏览器代码、移动端安装包或公开仓库。

## OpenAI 兼容接口

```http
Authorization: Bearer $ASTER_API_KEY
```

适用于 Responses、Chat Completions、图片生成和图片编辑接口。

## Anthropic Messages

```http
x-api-key: $ASTER_API_KEY
anthropic-version: 2023-06-01
```

Key 被禁用、过期或无权访问目标模型时，请求会在发送到模型提供商之前失败。
