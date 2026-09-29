<script setup lang="ts">
import { computed, ref } from 'vue'
import { copyText } from './copyText'
import rules from '../../../../customer/plugins/shared/public-rules.json'

type Protocol = 'responses' | 'chat_completions' | 'anthropic_messages'
type FieldStatus = 'supported' | 'mapped' | 'unsupported'
type ModelRule = {
  fields: Record<string, FieldStatus>
  protocol_fields?: Record<string, Record<string, FieldStatus>>
  field_notes?: Record<string, { zh: string; en: string }>
}

const props = defineProps<{ language: 'zh' | 'en'; model: string }>()
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
const protocol = ref<Protocol>('responses')
const copied = ref('')
const modelRules = rules.models as Record<string, ModelRule>
const selected = computed(() => modelRules[props.model])
const protocols: Array<{ id: Protocol; label: string; endpoint: string }> = [
  { id: 'responses', label: 'Responses', endpoint: '/v1/responses' },
  { id: 'chat_completions', label: 'Chat Completions', endpoint: '/v1/chat/completions' },
  { id: 'anthropic_messages', label: 'Anthropic Messages', endpoint: '/v1/messages' },
]
const selectedProtocol = computed(() => protocols.find(item => item.id === protocol.value) ?? protocols[0]!)
const fieldsByProtocol: Record<Protocol, Array<{ canonical: string; name: string; required?: boolean; description: [string, string] }>> = {
  responses: [
    { canonical: 'input', name: 'input', required: true, description: ['输入文本、消息或工具结果', 'Text, messages, or tool results'] },
    { canonical: 'max_output_tokens', name: 'max_output_tokens', description: ['限制本次最大输出', 'Limits output for this request'] },
    { canonical: 'reasoning_effort', name: 'reasoning.effort', description: ['设置推理强度', 'Sets reasoning effort'] },
    { canonical: 'tools', name: 'tools', description: ['声明可调用的函数工具', 'Declares callable function tools'] },
    { canonical: 'parallel_tool_calls_false', name: 'parallel_tool_calls', description: ['控制是否允许并行工具调用', 'Controls parallel tool calls'] },
    { canonical: 'json_object', name: 'text.format', description: ['请求结构化 JSON 输出', 'Requests structured JSON output'] },
    { canonical: 'temperature', name: 'temperature', description: ['控制采样随机性', 'Controls sampling randomness'] },
    { canonical: 'top_p', name: 'top_p', description: ['控制核采样范围', 'Controls nucleus sampling'] },
  ],
  chat_completions: [
    { canonical: 'input', name: 'messages', required: true, description: ['对话消息列表', 'Conversation messages'] },
    { canonical: 'max_output_tokens', name: 'max_completion_tokens', description: ['限制本次最大输出', 'Limits output for this request'] },
    { canonical: 'reasoning_effort', name: 'reasoning_effort', description: ['设置推理强度', 'Sets reasoning effort'] },
    { canonical: 'tools', name: 'tools', description: ['声明可调用的函数工具', 'Declares callable function tools'] },
    { canonical: 'parallel_tool_calls_false', name: 'parallel_tool_calls', description: ['控制是否允许并行工具调用', 'Controls parallel tool calls'] },
    { canonical: 'json_object', name: 'response_format', description: ['请求结构化 JSON 输出', 'Requests structured JSON output'] },
    { canonical: 'temperature', name: 'temperature', description: ['控制采样随机性', 'Controls sampling randomness'] },
    { canonical: 'top_p', name: 'top_p', description: ['控制核采样范围', 'Controls nucleus sampling'] },
  ],
  anthropic_messages: [
    { canonical: 'input', name: 'messages', required: true, description: ['对话消息列表', 'Conversation messages'] },
    { canonical: 'max_output_tokens', name: 'max_tokens', required: true, description: ['限制本次最大输出', 'Limits output for this request'] },
    { canonical: 'reasoning_effort', name: 'thinking', description: ['配置扩展思考', 'Configures extended thinking'] },
    { canonical: 'tools', name: 'tools', description: ['声明可调用的函数工具', 'Declares callable function tools'] },
    { canonical: 'temperature', name: 'temperature', description: ['控制采样随机性', 'Controls sampling randomness'] },
    { canonical: 'top_p', name: 'top_p', description: ['控制核采样范围', 'Controls nucleus sampling'] },
  ],
}
const effectiveStatus = (field: string): FieldStatus => selected.value?.protocol_fields?.[protocol.value]?.[field] ?? selected.value?.fields[field] ?? 'unsupported'
const parameterRows = computed(() => fieldsByProtocol[protocol.value].map(item => ({ ...item, status: effectiveStatus(item.canonical) })))
const statusLabel = (status: FieldStatus) => status === 'supported' ? tx('支持', 'Supported') : status === 'mapped' ? tx('自动转换', 'Converted') : tx('不支持', 'Unsupported')
const restrictions = computed(() => Object.entries(selected.value?.field_notes ?? {}).map(([field, note]) => ({ field, text: props.language === 'zh' ? note.zh : note.en })))
const rawExample = computed(() => {
  if (protocol.value === 'anthropic_messages') return `curl "$ASTER_BASE_URL${selectedProtocol.value.endpoint}" \\\n+  -H "x-api-key: $ASTER_API_KEY" \\\n+  -H "anthropic-version: 2023-06-01" \\\n+  -H "content-type: application/json" \\\n+  -d '{"model":"${props.model}","max_tokens":1024,"messages":[{"role":"user","content":"Hello"}]}'`
  if (protocol.value === 'chat_completions') return `curl "$ASTER_BASE_URL${selectedProtocol.value.endpoint}" \\\n+  -H "Authorization: Bearer $ASTER_API_KEY" \\\n+  -H "Content-Type: application/json" \\\n+  -d '{"model":"${props.model}","messages":[{"role":"user","content":"Hello"}]}'`
  return `curl "$ASTER_BASE_URL${selectedProtocol.value.endpoint}" \\\n+  -H "Authorization: Bearer $ASTER_API_KEY" \\\n+  -H "Content-Type: application/json" \\\n+  -d '{"model":"${props.model}","input":"Hello"}'`
})
const example = computed(() => rawExample.value.replace(/\n\+/g, '\n'))
async function copy(value: string, key: string) {
  await copyText(value)
  copied.value = key
  window.setTimeout(() => { if (copied.value === key) copied.value = '' }, 1500)
}
</script>

<template>
  <div v-if="selected" class="model-call-guide">
    <div class="model-guide-hero">
      <div><span class="model-guide-eyebrow">{{ tx('模型参考', 'Model reference') }}</span><h1>{{ model }}</h1><p>{{ tx('在三个公开协议中使用同一个模型 ID；切换协议可查看对应端点、字段和示例。','Use the same model ID across all three public protocols. Switch protocols to inspect the endpoint, fields, and example.') }}</p></div>
      <span class="model-guide-badge">{{ tx('官方适配规则', 'Official adapter rules') }}</span>
    </div>

    <section class="model-guide-card model-guide-id"><div><span>{{ tx('公开模型 ID', 'Public model ID') }}</span><code>{{ model }}</code></div><button type="button" @click="copy(model,'model')">{{ copied==='model'?tx('已复制','Copied'):tx('复制','Copy') }}</button></section>

    <section class="model-guide-protocol">
      <div class="model-guide-tabs" role="tablist" :aria-label="tx('公开协议','Public protocol')"><button v-for="item in protocols" :key="item.id" type="button" role="tab" :aria-selected="protocol===item.id" :class="{active:protocol===item.id}" @click="protocol=item.id">{{ item.label }}</button></div>
      <div class="model-guide-card model-guide-endpoint"><div><span>{{ tx('请求地址', 'Endpoint') }}</span><code>POST {{ selectedProtocol.endpoint }}</code></div><button type="button" @click="copy(selectedProtocol.endpoint,'endpoint')">{{ copied==='endpoint'?tx('已复制','Copied'):tx('复制','Copy') }}</button></div>
    </section>

    <section>
      <h2>{{ tx('最小调用示例', 'Minimal example') }}</h2>
      <div class="model-guide-code"><button type="button" @click="copy(example,'example')">{{ copied==='example'?tx('已复制','Copied'):tx('复制','Copy') }}</button><pre><code>{{ example }}</code></pre></div>
    </section>

    <section>
      <h2>{{ tx('参数支持', 'Parameter support') }}</h2>
      <div class="model-guide-table"><div class="model-guide-table-head"><span>{{ tx('参数', 'Parameter') }}</span><span>{{ tx('必填', 'Required') }}</span><span>{{ tx('用途', 'Purpose') }}</span><span>{{ tx('状态', 'Status') }}</span></div><div v-for="row in parameterRows" :key="row.name" class="model-guide-table-row"><code>{{ row.name }}</code><span class="model-guide-required">{{ row.required ? tx('是','Yes') : tx('否','No') }}</span><span>{{ tx(row.description[0],row.description[1]) }}</span><b :class="`status-${row.status}`">{{ statusLabel(row.status) }}</b></div></div>
    </section>

    <section v-if="restrictions.length">
      <h2>{{ tx('使用限制', 'Restrictions') }}</h2>
      <div class="model-guide-notes"><p v-for="item in restrictions" :key="item.field"><code>{{ item.field }}</code><span>{{ item.text }}</span></p></div>
    </section>

    <p class="model-guide-footnote">{{ tx('这里说明公开接口的字段适配。你在某个部署中是否可以调用此模型，仍以该部署分配给成员的模型权限为准。','This page describes public API field adaptation. Whether you can call the model in a deployment still depends on that deployment’s member model grants.') }}</p>
  </div>
</template>
