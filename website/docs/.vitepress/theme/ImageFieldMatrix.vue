<script setup lang="ts">
import { computed, ref } from 'vue'
import MatrixStatus from './MatrixStatus.vue'
import rules from '../../../../customer/plugins/shared/public-rules.json'

const props = defineProps<{ language: 'zh' | 'en' }>()
const tx = (zh: string, en: string) => props.language === 'zh' ? zh : en
type Status = 'supported' | 'mapped' | 'unsupported'
type ImageRule = {
  provider: string
  generate: boolean
  edit: boolean
  output: string
  fields: Record<string, Status>
  mappings?: Record<string, Record<string, string>>
}
const catalog = rules.image_models as Record<string, ImageRule>
const modelIds = Object.keys(catalog).sort((a, b) => {
  if (a === 'chatgpt-image-latest') return -1
  if (b === 'chatgpt-image-latest') return 1
  return b.localeCompare(a, 'en', { numeric: true })
})
const model = ref(modelIds[0]!)
const operation = ref<'generate' | 'edit'>('generate')
const selected = computed(() => catalog[model.value]!)
const fields = [
  { group: ['输入', 'Input'], id: 'prompt', required: true, zh: '提示词', en: 'Prompt' },
  { group: ['生成', 'Generation'], id: 'n', required: false, zh: '生成张数', en: 'Image count' },
  { group: ['生成', 'Generation'], id: 'size', required: false, zh: '图片尺寸', en: 'Image size' },
  { group: ['生成', 'Generation'], id: 'quality', required: false, zh: '生成质量', en: 'Quality' },
  { group: ['生成', 'Generation'], id: 'background', required: false, zh: '透明背景', en: 'Background' },
  { group: ['输出', 'Output'], id: 'output_format', required: false, zh: '图片编码', en: 'Output encoding' },
  { group: ['输出', 'Output'], id: 'output_compression', required: false, zh: '压缩程度', en: 'Compression' },
  { group: ['编辑', 'Editing'], id: 'mask', required: false, zh: '编辑遮罩', en: 'Edit mask' },
] as const
const status = (field: string): Status => {
  if (operation.value === 'edit' && !selected.value.edit) return 'unsupported'
  if (operation.value === 'generate' && field === 'mask') return 'unsupported'
  return selected.value.fields[field] ?? 'unsupported'
}
const label = (value: Status) => value === 'supported' ? tx('支持', 'Supported')
  : value === 'mapped' ? tx('映射', 'Mapped') : tx('不支持', 'Unsupported')
const mappings = (field: string) => Object.entries(selected.value.mappings?.[field] ?? {})
const note = (field: string, value: Status) => {
  if (operation.value === 'edit' && !selected.value.edit) return tx('该模型不开放图片编辑。', 'Image editing is unavailable for this model.')
  if (field === 'mask' && operation.value === 'generate') return tx('遮罩仅用于图片编辑。', 'A mask applies only to editing.')
  if (value === 'unsupported') return tx('此字段在派发前被拒绝。', 'This field is rejected before dispatch.')
  if (value === 'mapped') return tx('Aster 按下表转换；具体取值仍需符合目标模型限制。', 'Aster applies the mapping below; values must meet the target model limits.')
  return tx('字段按目标模型能力校验。', 'The field is validated against the target model.')
}
</script>

<template>
  <div class="model-matrix">
    <div class="model-matrix-tabs">
      <div class="model-matrix-tab-row"><span>{{ tx('图片模型', 'Image model') }}</span><div class="model-matrix-options is-wrapping" role="tablist" :aria-label="tx('选择图片模型', 'Select image model')"><button v-for="id in modelIds" :key="id" type="button" role="tab" :aria-selected="model===id" :class="{active:model===id}" @click="model=id">{{ id }}</button></div></div>
      <div class="model-matrix-tab-row"><span>{{ tx('操作', 'Operation') }}</span><div class="model-matrix-options" role="tablist" :aria-label="tx('选择图片操作', 'Select image operation')"><button type="button" role="tab" :aria-selected="operation==='generate'" :class="{active:operation==='generate'}" @click="operation='generate'">{{ tx('生成', 'Generate') }}</button><button type="button" role="tab" :aria-selected="operation==='edit'" :class="{active:operation==='edit'}" @click="operation='edit'">{{ tx('编辑', 'Edit') }}</button></div></div>
    </div>
    <div class="model-matrix-table"><table><thead><tr><th>{{ tx('分类', 'Category') }}</th><th>{{ tx('参数', 'Field') }}</th><th>{{ tx('必填', 'Required') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('用途', 'Purpose') }}</th></tr></thead><tbody>
      <tr v-for="field in fields" :key="field.id">
        <td class="model-matrix-category">{{ tx(field.group[0], field.group[1]) }}</td>
        <td><code>{{ field.id }}</code></td>
        <td class="model-matrix-required">{{ field.required ? tx('是', 'Yes') : tx('否', 'No') }}</td>
        <td>
          <MatrixStatus :status="status(field.id)" :label="label(status(field.id))">
              <span>{{ note(field.id, status(field.id)) }}</span>
              <div v-if="status(field.id) === 'mapped' && mappings(field.id).length" class="matrix-mapping" role="table">
                <div class="matrix-mapping-row matrix-mapping-heading" role="row"><span role="columnheader">{{ tx('公开值', 'Public value') }}</span><span role="columnheader">{{ tx('上游值或执行方式', 'Upstream value or execution') }}</span></div>
                <div v-for="[source, target] in mappings(field.id)" :key="source" class="matrix-mapping-row" role="row"><span role="cell"><code>{{ source }}</code></span><span role="cell"><code>{{ target }}</code></span></div>
              </div>
          </MatrixStatus>
        </td>
        <td>{{ tx(field.zh, field.en) }}</td>
      </tr>
    </tbody></table></div>
    <p class="model-matrix-meta">{{ tx('目标厂商', 'Provider') }}: {{ selected.provider }} · {{ tx('统一交付', 'Unified delivery') }}: <code>data[].b64_json</code> · {{ tx('规则版本', 'Rule revision') }}: {{ rules.revision }}</p>
    <p v-if="operation === 'edit' && !selected.edit" class="model-matrix-note">{{ tx('此模型仅支持生成图片。编辑请求会在派发前拒绝。', 'This model supports generation only. Edit requests are rejected before dispatch.') }}</p>
    <p class="model-matrix-note">{{ tx('模型还须在当前实例中启用，并由管理员授予成员使用权限及图片张数额度。', 'The model must also be enabled in this installation, granted to the member, and have image count quota.') }}</p>
  </div>
</template>
