<script setup lang="ts">
import { computed, ref } from 'vue'
import MatrixStatus from './MatrixStatus.vue'
import rules from '../../../../customer/plugins/shared/public-rules.json'

const props = defineProps<{ language: 'zh' | 'en'; initialProtocol?: 'responses' | 'chat_completions' | 'anthropic_messages' }>()
const model = ref('deepseek-flash')
const protocol = ref(props.initialProtocol ?? 'responses')
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
type FieldStatus = 'supported' | 'mapped' | 'unsupported'
type ModelRule = {
  provider: string
  upstream_protocol: string
  wire_by_public_protocol?: Record<string, string>
  fields: Record<string, FieldStatus>
  protocol_fields?: Record<string, Record<string, FieldStatus>>
  field_notes?: Record<string, { zh: string; en: string }>
  reasoning?: { native: string[]; mapped: Record<string, string> }
}
const modelRules = rules.models as Record<string, ModelRule>
const selected = computed(() => modelRules[model.value])
const fields = [
  { group: 'input', id: 'input', zh: '文本消息与工具历史', en: 'Text messages and tool history' },
  { group: 'output', id: 'max_output_tokens', zh: '生成令牌上限', en: 'Maximum generated tokens' },
  { group: 'reasoning', id: 'reasoning_effort', zh: '推理强度与取值映射', en: 'Reasoning effort and value mapping' },
  { group: 'tools', id: 'tools', zh: '函数定义与工具结果', en: 'Function definitions and tool results' },
  { group: 'tools', id: 'parallel_tool_calls_false', zh: '禁止并行工具调用', en: 'Disable parallel tool calls' },
  { group: 'json', id: 'json_object', zh: 'JSON 对象输出', en: 'JSON object output' },
  { group: 'json', id: 'json_schema', zh: '严格 Schema 约束', en: 'Strict JSON Schema constraint' },
  { group: 'images', id: 'image_input', zh: '消息中的图片输入', en: 'Image input in messages' },
  { group: 'images', id: 'image_generation', zh: '图片生成操作', en: 'Image generation operation' },
  { group: 'sampling', id: 'temperature', zh: '采样温度', en: 'Sampling temperature' },
  { group: 'sampling', id: 'top_p', zh: '核采样概率', en: 'Nucleus sampling probability' },
] as const
const groupNames: Record<string, [string, string]> = {
  input: ['输入与对话', 'Input and conversation'],
  output: ['输出与流式', 'Output and streaming'],
  reasoning: ['思考设置', 'Reasoning'],
  tools: ['工具调用', 'Tools'],
  json: ['结构化结果', 'Structured output'],
  images: ['图片', 'Images'],
  sampling: ['采样', 'Sampling'],
}
const groupName = (group: string) => {
  const pair = groupNames[group]
  return pair ? tx(pair[0], pair[1]) : group
}
const isRequired = (field: string) => field === 'input'
  || (field === 'max_output_tokens' && protocol.value === 'anthropic_messages')
const statusLabel = (status: FieldStatus) => status === 'supported'
  ? tx('支持', 'Supported') : status === 'mapped' ? tx('映射', 'Mapped') : tx('不支持', 'Unsupported')
const wireProtocol = computed(() => selected.value.wire_by_public_protocol?.[protocol.value] ?? selected.value.upstream_protocol)
const fieldStatus = (field: string): FieldStatus => {
  const override = selected.value.protocol_fields?.[protocol.value]?.[field]
  if (override) return override
  const base = selected.value.fields[field] ?? 'unsupported'
  if (base === 'supported' && field === 'input' && protocol.value !== 'chat_completions' && wireProtocol.value === 'chat') return 'mapped'
  if (base === 'supported' && field === 'max_output_tokens' && wireProtocol.value === 'chat') return 'mapped'
  return base
}
const fieldPath = (field: string) => field === 'max_output_tokens'
  ? protocol.value === 'chat_completions' ? 'max_completion_tokens' : protocol.value === 'anthropic_messages' ? 'max_tokens' : 'max_output_tokens'
  : field === 'reasoning_effort' ? protocol.value === 'responses' ? 'reasoning.effort' : protocol.value === 'anthropic_messages' ? 'thinking' : 'reasoning_effort'
  : field
const mappings = (field: string) => {
  if (field === 'reasoning_effort') return Object.entries(selected.value.reasoning?.mapped ?? {})
  if (field === 'max_output_tokens' && wireProtocol.value === 'chat') return [[fieldPath(field), 'max_tokens']]
  if (field === 'input' && protocol.value === 'responses' && wireProtocol.value === 'chat') return [['input', 'messages']]
  if (field === 'top_p' && selected.value.provider === 'deepseek') return [['< 0.95', '0.95']]
  return []
}
const detail = (field: string, status: FieldStatus) => {
  const note = selected.value.field_notes?.[field]
  if (note) return tx(note.zh, note.en)
  return status === 'unsupported'
    ? tx('此协议与模型组合不开放该字段。请求在派发前被拒绝。', 'This protocol and model combination does not expose the field. The request is rejected before dispatch.')
    : status === 'mapped' ? tx('字段或取值按下表转换；严格模式拒绝近似取值映射。', 'The field or value follows the mapping below. Strict mode rejects approximate value mappings.')
      : tx('字段语义保留；可用取值仍由目标模型规则决定。', 'The field meaning is preserved. The target model rule determines valid values.')
}
</script>

<template>
  <div class="model-matrix">
    <div class="model-matrix-tabs">
      <div class="model-matrix-tab-row"><span>{{ tx('模型', 'Model') }}</span><div class="model-matrix-options" role="tablist" :aria-label="tx('选择模型', 'Select model')"><button v-for="(_, id) in modelRules" :key="id" type="button" role="tab" :aria-selected="model===id" :class="{active:model===id}" @click="model=id">{{ id }}</button></div></div>
      <div class="model-matrix-tab-row"><span>{{ tx('公开协议', 'Public protocol') }}</span><div class="model-matrix-options" role="tablist" :aria-label="tx('选择公开协议', 'Select public protocol')"><button type="button" role="tab" :aria-selected="protocol==='responses'" :class="{active:protocol==='responses'}" @click="protocol='responses'">Responses</button><button type="button" role="tab" :aria-selected="protocol==='chat_completions'" :class="{active:protocol==='chat_completions'}" @click="protocol='chat_completions'">Chat Completions</button><button type="button" role="tab" :aria-selected="protocol==='anthropic_messages'" :class="{active:protocol==='anthropic_messages'}" @click="protocol='anthropic_messages'">Anthropic Messages</button></div></div>
    </div>
    <div class="model-matrix-table">
      <table><thead><tr><th>{{ tx('分类', 'Category') }}</th><th>{{ tx('参数', 'Field') }}</th><th>{{ tx('必填', 'Required') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('用途与注意', 'Purpose and notes') }}</th></tr></thead><tbody>
        <tr v-for="field in fields" :key="field.id">
          <td class="model-matrix-category">{{ groupName(field.group) }}</td>
          <td><code>{{ fieldPath(field.id) }}</code></td>
          <td class="model-matrix-required">{{ isRequired(field.id) ? tx('是', 'Yes') : tx('否', 'No') }}</td>
          <td>
            <MatrixStatus :status="fieldStatus(field.id)" :label="statusLabel(fieldStatus(field.id))">
                <span>{{ detail(field.id, fieldStatus(field.id)) }}</span>
                <div v-if="mappings(field.id).length && fieldStatus(field.id) === 'mapped'" class="matrix-mapping" role="table">
                  <div class="matrix-mapping-row matrix-mapping-heading" role="row"><span role="columnheader">{{ tx('公开值', 'Public value') }}</span><span role="columnheader">{{ tx('上游值', 'Upstream value') }}</span></div>
                  <div v-for="[source, target] in mappings(field.id)" :key="source" class="matrix-mapping-row" role="row"><span role="cell"><code>{{ source }}</code></span><span role="cell"><code>{{ target }}</code></span></div>
                </div>
            </MatrixStatus>
          </td>
          <td>{{ tx(field.zh, field.en) }}</td>
        </tr>
      </tbody></table>
    </div>
    <p class="model-matrix-note">{{ tx('此表展示公开参数规则，不代表某个成员已获模型权限。当前接入实现的可用范围以实例中的模型能力与预检结果为准。', 'This table describes public field rules, not a member’s model grant. The instance model capabilities and preflight result determine availability.') }}</p>
  </div>
</template>
