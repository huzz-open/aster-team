---
title: SDK 示例
description: 使用 OpenAI 与 Anthropic SDK 连接 Aster。
---

# SDK 示例

## OpenAI SDK

```js
import OpenAI from 'openai'

const client = new OpenAI({
  apiKey: process.env.ASTER_API_KEY,
  baseURL: process.env.ASTER_BASE_URL + '/v1',
})

const response = await client.responses.create({
  model: 'deepseek-v4-pro',
  input: '你好',
})
```

## Anthropic SDK

```js
import Anthropic from '@anthropic-ai/sdk'

const client = new Anthropic({
  apiKey: process.env.ASTER_API_KEY,
  baseURL: process.env.ASTER_BASE_URL,
})
```

从成员模型页面复制实际可用的公开模型 ID，不要填写上游厂商的内部型号。
