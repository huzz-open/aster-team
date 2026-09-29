---
title: SDK examples
description: Connect OpenAI and Anthropic SDKs to Aster.
---

# SDK examples

## OpenAI SDK

```js
import OpenAI from 'openai'

const client = new OpenAI({ apiKey: process.env.ASTER_API_KEY, baseURL: process.env.ASTER_BASE_URL + '/v1' })
const response = await client.responses.create({ model: 'deepseek-v4-pro', input: 'Hello' })
```

## Anthropic SDK

```js
import Anthropic from '@anthropic-ai/sdk'

const client = new Anthropic({ apiKey: process.env.ASTER_API_KEY, baseURL: process.env.ASTER_BASE_URL })
```

Copy an available public model ID from the member model page. Do not use a provider’s internal model name.
