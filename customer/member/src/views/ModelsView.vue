<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { ACopyCode, AEmpty, AIcon, AIconButton, ALoadingState, AModal, APagination, ASegmentedControl, ASelect, useToast } from '@aster/ui'
import { copyText, formatDate, request, type Model } from '@aster/sdk'
import { locale, t } from '../i18n'

type FieldStatus = 'supported' | 'mapped' | 'unsupported'
type DocsInfo = { usage_multiplier: number; public_api_base_url: string; protocols: Array<{ id:string;name:string;path:string }> }
type ModelCapabilities = {
  public_model: string
  source: string
  provider?: string
  channel_id?: string
  upstream_model?: string
  rule_revision?: string
  compatibility_mode?: 'compatible' | 'strict'
  quota_unit?: 'token' | 'image'
  image_quota?: { available_images: number; reserved_images: number; consumed_images: number } | null
  protocols: string[]
  rules?: {
    fields: Record<string, FieldStatus>
    protocol_fields?: Record<string, Record<string, FieldStatus>>
    field_notes?: Record<string, { zh: string; en: string }>
  } | null
}
const models=ref<Model[]>([]),docs=ref<DocsInfo|null>(null),keyword=ref(''),statusFilter=ref(''),providerFilter=ref(''),multiplierFilter=ref(''),view=ref<'cards'|'table'>('cards'),loading=ref(true),copied=ref('')
const page=ref(1),pageSize=ref(24)
const toast=useToast()
const capabilities=ref<ModelCapabilities|null>(null),detailsOpen=ref(false),detailsLoading=ref(false),detailsAvailable=ref(false),detailsProtocol=ref('responses')
const tx=(zh:string,en:string)=>locale.value==='en-US'?en:zh
const protocolDefinitions:Record<string,{label:string;path:string;fields:Array<{canonical:string;name:string;description:[string,string]}>}>={
  responses:{label:'Responses',path:'/v1/responses',fields:[
    {canonical:'input',name:'input',description:['输入消息','Input messages']},{canonical:'max_output_tokens',name:'max_output_tokens',description:['最大输出令牌数','Maximum output tokens']},{canonical:'reasoning_effort',name:'reasoning.effort',description:['推理强度','Reasoning effort']},{canonical:'tools',name:'tools',description:['工具定义','Tool definitions']},{canonical:'parallel_tool_calls_false',name:'parallel_tool_calls',description:['并行工具调用','Parallel tool calls']},{canonical:'json_object',name:'text.format',description:['结构化输出','Structured output']},{canonical:'temperature',name:'temperature',description:['采样温度','Sampling temperature']},{canonical:'top_p',name:'top_p',description:['核采样概率','Nucleus sampling']},
  ]},
  chat_completions:{label:'Chat Completions',path:'/v1/chat/completions',fields:[
    {canonical:'input',name:'messages',description:['对话消息','Conversation messages']},{canonical:'max_output_tokens',name:'max_completion_tokens',description:['最大输出令牌数','Maximum output tokens']},{canonical:'reasoning_effort',name:'reasoning_effort',description:['推理强度','Reasoning effort']},{canonical:'tools',name:'tools',description:['工具定义','Tool definitions']},{canonical:'parallel_tool_calls_false',name:'parallel_tool_calls',description:['并行工具调用','Parallel tool calls']},{canonical:'json_object',name:'response_format',description:['结构化输出','Structured output']},{canonical:'temperature',name:'temperature',description:['采样温度','Sampling temperature']},{canonical:'top_p',name:'top_p',description:['核采样概率','Nucleus sampling']},
  ]},
  anthropic_messages:{label:'Anthropic Messages',path:'/v1/messages',fields:[
    {canonical:'input',name:'messages',description:['对话消息','Conversation messages']},{canonical:'max_output_tokens',name:'max_tokens',description:['最大输出令牌数','Maximum output tokens']},{canonical:'reasoning_effort',name:'thinking',description:['思考配置','Thinking configuration']},{canonical:'tools',name:'tools',description:['工具定义','Tool definitions']},{canonical:'temperature',name:'temperature',description:['采样温度','Sampling temperature']},{canonical:'top_p',name:'top_p',description:['核采样概率','Nucleus sampling']},
  ]},
  'images/generations':{label:'Image generation',path:'/v1/images/generations',fields:[
    {canonical:'prompt',name:'prompt',description:['图片描述','Image prompt']},{canonical:'n',name:'n',description:['生成数量','Image count']},{canonical:'size',name:'size',description:['图片尺寸','Image size']},{canonical:'quality',name:'quality',description:['生成质量','Generation quality']},{canonical:'output_format',name:'output_format',description:['输出格式','Output format']},
  ]},
  'images/edits':{label:'Image editing',path:'/v1/images/edits',fields:[
    {canonical:'prompt',name:'prompt',description:['编辑说明','Edit prompt']},{canonical:'mask',name:'mask',description:['蒙版图片','Mask image']},{canonical:'n',name:'n',description:['生成数量','Image count']},{canonical:'size',name:'size',description:['图片尺寸','Image size']},{canonical:'quality',name:'quality',description:['生成质量','Generation quality']},
  ]},
}
const selectedProtocol=computed(()=>protocolDefinitions[detailsProtocol.value] ?? protocolDefinitions.responses!)
const effectiveStatus=(field:string):FieldStatus=>{
  const rules=capabilities.value?.rules
  if (!rules) return 'unsupported'
  if (capabilities.value?.quota_unit === 'image' && detailsProtocol.value === 'images/generations' && field === 'mask') return 'unsupported'
  return rules.protocol_fields?.[detailsProtocol.value]?.[field] ?? rules.fields[field] ?? 'unsupported'
}
const detailRows=computed(()=>selectedProtocol.value.fields.map(field=>({...field,status:effectiveStatus(field.canonical)})))
const detailStatus=(status:string)=>status==='supported' ? tx('支持','Supported') : status==='mapped' ? tx('自动转换','Converted') : tx('不支持','Unsupported')
const protocolChoices=computed(()=>(capabilities.value?.protocols ?? []).map(protocol=>({value:protocol,label:protocolDefinitions[protocol]?.label ?? protocol})))
const detailEndpoint=computed(()=>`${(docs.value?.public_api_base_url || window.location.origin).replace(/\/$/,'')}${selectedProtocol.value.path}`)
const detailCapabilities=computed(()=>[
  {field:'input',label:tx('文本对话','Text chat')},{field:'tools',label:tx('工具调用','Tool calls')},{field:'json_object',label:tx('结构化输出','Structured output')},{field:'image_input',label:tx('图片输入','Image input')},
].filter(item=>capabilities.value?.rules?.fields[item.field]).map(item=>({...item,status:effectiveStatus(item.field)})))
const detailRestrictions=computed(()=>Object.entries(capabilities.value?.rules?.field_notes ?? {}).map(([field,note])=>({field,text:locale.value==='en-US'?note.en:note.zh})))
const docsModelPath=computed(()=>{
  const model=capabilities.value?.public_model || ''
  const documented=['deepseek-flash','deepseek-v4-pro','glm-5.2'].includes(model) ? model : ''
  return `${window.location.origin}/docs/${locale.value==='en-US'?'en':'zh-cn'}/models/${documented}`
})
const rawRequestExample=computed(()=>{
  const endpoint=detailEndpoint.value
  const model=capabilities.value?.public_model || 'MODEL_ID'
  if(detailsProtocol.value==='anthropic_messages') return `curl ${endpoint} \\\n+  -H "x-api-key: $ASTER_API_KEY" \\\n+  -H "anthropic-version: 2023-06-01" \\\n+  -H "content-type: application/json" \\\n+  -d '{"model":"${model}","max_tokens":1024,"messages":[{"role":"user","content":"Hello"}]}'`
  if(detailsProtocol.value==='chat_completions') return `curl ${endpoint} \\\n+  -H "Authorization: Bearer $ASTER_API_KEY" \\\n+  -H "Content-Type: application/json" \\\n+  -d '{"model":"${model}","messages":[{"role":"user","content":"Hello"}]}'`
  if(detailsProtocol.value.startsWith('images/')) return `curl ${endpoint} \\\n+  -H "Authorization: Bearer $ASTER_API_KEY" \\\n+  -H "Content-Type: application/json" \\\n+  -d '{"model":"${model}","prompt":"A quiet mountain lake"}'`
  return `curl ${endpoint} \\\n+  -H "Authorization: Bearer $ASTER_API_KEY" \\\n+  -H "Content-Type: application/json" \\\n+  -d '{"model":"${model}","input":"Hello"}'`
})
const requestExample=computed(()=>rawRequestExample.value.replace(/\n\+/g,'\n'))
async function copyGuide(value:string){await copyText(value);toast.success(tx('已复制','Copied'))}
async function showCapabilities(item:Model){
  detailsOpen.value=true;detailsLoading.value=true;detailsAvailable.value=modelAvailable(item);capabilities.value=null;detailsProtocol.value='responses'
  try{
    capabilities.value=await request<ModelCapabilities>(`/api/member/models/${encodeURIComponent(item.id)}/capabilities`)
    detailsProtocol.value=capabilities.value.protocols[0] ?? 'responses'
  }
  catch(value){toast.error(value instanceof Error?value.message:tx('模型能力加载失败','Could not load model capabilities'));detailsOpen.value=false}
  finally{detailsLoading.value=false}
}

function modelProvider(item: Model) { return (item.provider || 'openai').toLocaleLowerCase() }
function providerLabel(provider: string) { return provider === 'openai' ? 'OpenAI' : provider.replace(/(^|[-_])\w/g, value => value.toUpperCase().replace(/[-_]/, ' ')) }
function modelAvailable(item: Model) { return item.enabled && item.available !== false }
function modelMultiplier() {
  const value = Number(docs.value?.usage_multiplier ?? 1)
  return Number.isFinite(value) && value > 0 ? value : 1
}
function multiplierKey() { return modelMultiplier().toFixed(2) }

const statusChoices=computed(()=>[
  {value:'',label:t('allStatuses')},
  {value:'available',label:t('available')},
  {value:'unavailable',label:t('unavailable')},
])
const providerChoices=computed(()=>[
  {value:'',label:t('allProviders')},
  ...[...new Set(models.value.map(modelProvider))].sort().map(value=>({value,label:providerLabel(value)})),
])
const multiplierChoices=computed(()=>[
  {value:'',label:t('allSettlementRates')},
  ...(models.value.length ? [{value:multiplierKey(),label:`${multiplierKey()}×`}] : []),
])
const filtered=computed(()=>models.value.filter(item=>{
  const available=modelAvailable(item)
  const provider=modelProvider(item)
  const multiplier=multiplierKey()
  const needle=keyword.value.trim().toLocaleLowerCase()
  return (!needle||`${item.public_name} ${item.display_name} ${provider} ${providerLabel(provider)} ${multiplier}`.toLocaleLowerCase().includes(needle))
    &&(!statusFilter.value||(statusFilter.value==='available' ? available : !available))
    &&(!providerFilter.value||provider===providerFilter.value)
    &&(!multiplierFilter.value||multiplier===multiplierFilter.value)
}))
const paged=computed(()=>filtered.value.slice((page.value-1)*pageSize.value,page.value*pageSize.value))
async function copy(value:string,id:string){await copyText(value);copied.value=id;window.setTimeout(()=>{copied.value=''},1500)}
async function load(){loading.value=true;try{const [modelResult,docsResult]=await Promise.all([request<{items:Model[]}>('/api/member/models'),request<DocsInfo>(`/api/member/docs?locale=${locale.value}`)]);models.value=modelResult.items;docs.value=docsResult}catch(value){toast.error(value instanceof Error?value.message:t('modelsLoadFailed'))}finally{loading.value=false}}
onMounted(load)
watch([keyword,statusFilter,providerFilter,multiplierFilter],()=>{page.value=1})
watch(pageSize,()=>{page.value=1})
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head"><div><h1>{{ t('modelMarketplace') }}</h1></div></header>
    <ALoadingState v-if="loading" :label="t('loadingOverview')" />
    <template v-else>
      <section class="market-toolbar">
        <label class="market-search"><AIcon name="model" /><input v-model="keyword" :aria-label="t('searchModels')" :placeholder="t('searchModels')"></label>
        <ASelect v-model="statusFilter" class="market-filter status-filter" :options="statusChoices" :aria-label="t('status')" />
        <ASelect v-model="providerFilter" class="market-filter provider-filter" :options="providerChoices" :aria-label="t('provider')" />
        <ASelect v-model="multiplierFilter" class="market-filter multiplier-filter" :options="multiplierChoices" :aria-label="t('settlementRate')" />
        <div class="view-switch" role="group" :aria-label="locale === 'zh-CN' ? '视图方式' : 'View mode'"><AIconButton icon="dashboard" size="small" :variant="view==='cards'?'accent':'neutral'" :label="t('cardView')" :aria-pressed="view==='cards'" @click="view='cards'" /><AIconButton icon="audit" size="small" :variant="view==='table'?'accent':'neutral'" :label="t('tableView')" :aria-pressed="view==='table'" @click="view='table'" /></div>
      </section>
      <div v-if="filtered.length && view==='cards'" class="model-market-grid"><article v-for="item in paged" :key="item.id" :class="{ 'is-unavailable': !modelAvailable(item) }"><div class="model-card-head"><span class="model-avatar">{{ (item.display_name||item.public_name).slice(0,2).toUpperCase() }}</span><span class="provider-badge">{{ providerLabel(modelProvider(item)) }}</span><span class="status" :class="{ off: !modelAvailable(item) }">{{ modelAvailable(item) ? t('available') : t('unavailable') }}</span></div><h2 :title="item.display_name||item.public_name">{{ item.display_name||item.public_name }}</h2><div class="model-id"><code :title="item.public_name">{{ item.public_name }}</code><AIconButton icon="copy" size="small" :label="copied===item.id?t('copied'):t('copy')" @click="copy(item.public_name,item.id)" /></div><div class="model-facts"><span>{{ t('settlementRate') }}<b>{{ multiplierKey() }}×</b></span><button type="button" @click="showCapabilities(item)">{{ tx('查看模型能力','View capabilities') }}</button></div><p>{{ t('discoveredAt') }} {{ formatDate(item.discovered_at, locale) }}</p></article></div>
      <div v-else-if="filtered.length" class="table-wrap"><table><thead><tr><th>{{ t('model') }}</th><th>{{ t('publicModelID') }}</th><th>{{ t('provider') }}</th><th>{{ t('status') }}</th><th>{{ t('multiplier') }}</th><th>{{ t('discoveredAt') }}</th><th>{{ t('actions') }}</th></tr></thead><tbody><tr v-for="item in paged" :key="item.id"><td><strong>{{ item.display_name||item.public_name }}</strong></td><td><code>{{ item.public_name }}</code></td><td>{{ providerLabel(modelProvider(item)) }}</td><td><span class="status" :class="{ off: !modelAvailable(item) }">{{ modelAvailable(item) ? t('available') : t('unavailable') }}</span></td><td>{{ multiplierKey() }}×</td><td>{{ formatDate(item.discovered_at,locale) }}</td><td><button type="button" class="capability-link" @click="showCapabilities(item)">{{ tx('能力','Capabilities') }}</button><AIconButton icon="copy" size="small" :label="t('copy')" @click="copy(item.public_name,item.id)" /></td></tr></tbody></table></div>
      <AEmpty v-else icon="model" :title="models.length ? t('noMatchingModels') : locale === 'zh-CN' ? '尚未分配可用模型，请联系管理员' : 'No models have been assigned. Contact your administrator.'" />
      <APagination v-if="filtered.length > 0" v-model:page="page" v-model:page-size="pageSize" :page-size-options="[12, 24, 48]" :total="filtered.length" :locale="locale" />
    </template>
    <AModal :open="detailsOpen" :title="capabilities?.public_model || tx('模型调用说明','Model call guide')" :close-label="tx('关闭','Close')" wide @close="detailsOpen=false">
      <ALoadingState v-if="detailsLoading" :label="tx('加载模型规则','Loading model rules')" />
      <div v-else-if="capabilities" class="capability-details">
        <div class="capability-intro"><span class="status" :class="{off:!detailsAvailable}">{{ detailsAvailable?tx('当前可用','Available now'):tx('当前不可用','Currently unavailable') }}</span><p>{{ detailsAvailable?tx('以下内容使用对外公开的模型 ID 和接口字段，可直接用于调用。','Use the public model ID and request fields below to make a call.'):tx('此模型当前不可调用；恢复可用后，可按以下公开接口接入。','This model cannot be called now. Use the public API below after it becomes available.') }}</p></div>
        <section class="guide-section"><h3>{{ tx('公开模型 ID','Public model ID') }}</h3><ACopyCode :value="capabilities.public_model" :label="tx('复制模型 ID','Copy model ID')" :copied-label="tx('已复制','Copied')" @copy="copyGuide(capabilities.public_model)" /></section>
        <section class="guide-section"><h3>{{ tx('调用协议','API protocol') }}</h3><ASegmentedControl v-model="detailsProtocol" :options="protocolChoices" :label="tx('调用协议','API protocol')" stretch /></section>
        <section class="guide-section"><h3>{{ tx('请求地址','Endpoint') }}</h3><ACopyCode :value="detailEndpoint" :label="tx('复制请求地址','Copy endpoint')" :copied-label="tx('已复制','Copied')" @copy="copyGuide(detailEndpoint)" /></section>
        <section v-if="detailCapabilities.length" class="guide-section"><h3>{{ tx('主要能力','Key capabilities') }}</h3><div class="capability-summary"><span v-for="item in detailCapabilities" :key="item.field" :class="`is-${item.status}`"><AIcon :name="item.status==='unsupported'?'close':'check'" :size="14" />{{ item.label }}<small>{{ detailStatus(item.status) }}</small></span></div></section>
        <section v-if="capabilities.rules" class="guide-section"><h3>{{ tx('常用参数','Common parameters') }}</h3><div class="parameter-list"><div v-for="row in detailRows" :key="row.name"><code>{{ row.name }}</code><span>{{ tx(row.description[0],row.description[1]) }}</span><b :class="`is-${row.status}`">{{ detailStatus(row.status) }}</b></div></div></section>
        <section v-if="detailRestrictions.length" class="guide-section"><h3>{{ tx('使用限制','Restrictions') }}</h3><ul class="restriction-list"><li v-for="item in detailRestrictions" :key="item.field"><code>{{ item.field }}</code><span>{{ item.text }}</span></li></ul></section>
        <section class="guide-section"><h3>{{ tx('最小调用示例','Minimal example') }}</h3><ACopyCode :value="requestExample" layout="block" :label="tx('复制示例','Copy example')" :copied-label="tx('已复制','Copied')" @copy="copyGuide(requestExample)" /></section>
        <div class="guide-footer"><span>{{ tx('组合参数仍会在请求时校验。','Parameter combinations are validated when the request is made.') }}</span><a :href="docsModelPath" target="_blank" rel="noreferrer">{{ tx('查看完整文档','View complete documentation') }} →</a></div>
      </div>
    </AModal>
  </div>
</template>

<style scoped>
.market-toolbar{display:flex;align-items:center;gap:10px;margin-bottom:14px}
.market-toolbar>label{height:40px;display:flex;align-items:center;gap:9px;flex:1;padding:0 12px;color:var(--muted);border:1px solid var(--line);border-radius:11px;background:var(--surface);transition:color .15s ease,border-color .15s ease,box-shadow .15s ease}
.market-toolbar>label:hover{border-color:color-mix(in srgb,var(--accent) 28%,var(--line-strong))}
.market-toolbar>label:focus-within{color:var(--accent);border-color:var(--accent);box-shadow:var(--focus-ring)}
.market-toolbar input,.market-toolbar input:hover,.market-toolbar input:focus,.market-toolbar input:focus-visible{padding:0;border:0;outline:0;background:transparent;box-shadow:none}
.market-filter{flex:0 0 124px;min-width:0}
.provider-filter{flex-basis:136px}
.multiplier-filter{flex-basis:128px}
.view-switch{display:flex;padding:3px;border:1px solid var(--line);border-radius:10px;background:var(--surface-2)}
.view-switch button{width:34px;height:32px;display:grid;place-items:center;color:var(--muted);border:0;border-radius:7px;background:transparent}
.view-switch button.active{color:var(--accent);background:var(--surface);box-shadow:var(--shadow-sm)}
.model-market-grid{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:9px}
.model-market-grid article{display:grid;min-width:0;gap:8px;padding:12px;border:1px solid var(--line);border-radius:var(--radius);background:var(--surface);transition:border-color .16s}
.model-market-grid article.is-unavailable{background:color-mix(in srgb,var(--surface) 86%,var(--surface-2))}
.model-card-head,.model-id{display:flex;align-items:center;justify-content:space-between;min-width:0;gap:7px}
.model-avatar{width:30px;height:30px;display:grid;place-items:center;flex:0 0 30px;border-radius:9px;color:var(--accent);background:var(--accent-soft);font-size:var(--font-size-caption);font-weight:800}
.provider-badge{margin-left:auto;padding:3px 6px;color:var(--text-soft);background:var(--surface-3)}
.provider-badge,.model-card-head .status{min-width:0;overflow:hidden;border-radius:999px;font-size:var(--font-size-caption);text-overflow:ellipsis;white-space:nowrap}
.model-card-head .status{padding:3px 6px}
.model-market-grid h2{overflow:hidden;margin:0;font-size:var(--font-size-body);text-overflow:ellipsis;white-space:nowrap}
.model-id{padding:6px 7px;border-radius:8px;background:var(--surface-2)}
.model-id code,.table-wrap code{min-width:0;overflow:hidden;color:var(--text-soft);font:var(--font-size-caption) var(--font-ui);text-overflow:ellipsis;white-space:nowrap}
.model-facts{display:grid;grid-template-columns:1fr 1fr;gap:6px}
.model-facts span{display:flex;justify-content:space-between;gap:4px;padding:6px;border:1px solid var(--line);border-radius:8px;color:var(--muted);font-size:var(--font-size-caption);white-space:nowrap}
.model-facts b{color:var(--text)}
.model-facts button,.capability-link{border:0;background:transparent;color:var(--accent);cursor:pointer;text-align:left;font-size:var(--font-size-body)}
.model-facts button:hover,.capability-link:hover{text-decoration:underline}
.capability-details{display:grid;gap:18px;padding-bottom:2px}.capability-details p{margin:0;color:var(--text-soft);font-size:var(--font-size-body)}
.capability-intro{display:flex;align-items:center;gap:10px}.capability-intro .status{flex:0 0 auto}.guide-section{display:grid;gap:8px}.guide-section h3{margin:0;color:var(--text);font-size:var(--font-size-body)}
.capability-summary{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:8px}.capability-summary>span{display:grid;grid-template-columns:auto 1fr;align-items:center;gap:6px;padding:10px;border:1px solid var(--line);border-radius:10px;color:var(--text);background:var(--surface-2);font-size:var(--font-size-body)}.capability-summary>span small{grid-column:2;color:var(--muted);font-size:var(--font-size-caption)}.capability-summary>span.is-unsupported{color:var(--muted)}
.parameter-list{overflow:hidden;border:1px solid var(--line);border-radius:12px}.parameter-list>div{display:grid;grid-template-columns:minmax(150px,.9fr) minmax(170px,1.4fr) auto;align-items:center;gap:12px;padding:10px 12px}.parameter-list>div+div{border-top:1px solid var(--line)}.parameter-list code{color:var(--text);font-size:var(--font-size-caption)}.parameter-list span{color:var(--text-soft);font-size:var(--font-size-body)}.parameter-list b{font-size:var(--font-size-caption);font-weight:600}.parameter-list .is-supported{color:var(--positive)}.parameter-list .is-mapped{color:var(--accent)}.parameter-list .is-unsupported{color:var(--muted)}
.restriction-list{display:grid;gap:8px;margin:0;padding:0;list-style:none}.restriction-list li{display:grid;grid-template-columns:minmax(110px,auto) 1fr;gap:10px;padding:10px 12px;border-radius:10px;background:var(--warning-soft);color:var(--text-soft);font-size:var(--font-size-body)}.restriction-list code{color:var(--text)}
.guide-footer{display:flex;align-items:center;justify-content:space-between;gap:14px;padding-top:4px;color:var(--muted);font-size:var(--font-size-caption)}.guide-footer a{color:var(--accent);font-size:var(--font-size-body);font-weight:600;text-decoration:none;white-space:nowrap}.guide-footer a:hover{text-decoration:underline}
.model-market-grid p{overflow:hidden;margin:0;color:var(--muted);font-size:var(--font-size-caption);text-overflow:ellipsis;white-space:nowrap}
@media(max-width:1180px){.model-market-grid{grid-template-columns:repeat(4,minmax(0,1fr))}.market-toolbar{flex-wrap:wrap}.market-search{flex-grow:1!important}}
@media(max-width:900px){.model-market-grid{grid-template-columns:repeat(3,minmax(0,1fr))}}
@media(max-width:760px){.model-market-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:560px){.model-market-grid{grid-template-columns:1fr}.market-filter{flex:1 1 calc(50% - 5px)}.view-switch{margin-left:auto}}
@media(max-width:680px){.capability-summary{grid-template-columns:repeat(2,minmax(0,1fr))}.parameter-list>div{grid-template-columns:minmax(0,1fr) auto}.parameter-list>div>span{grid-column:1/-1}.guide-footer{align-items:flex-start;flex-direction:column}}
</style>

<style scoped>
.view-switch {
  gap: 2px;
}
.view-switch .a-icon-button {
  border-color: transparent;
  box-shadow: none;
}
.view-switch .a-icon-button--neutral {
  color: var(--muted);
  background: transparent;
}
.view-switch .a-icon-button--accent {
  color: var(--accent);
  background: var(--surface);
  box-shadow: var(--shadow-sm);
}
.model-market-grid article:hover{transform:none;border-color:var(--line)}
.market-toolbar>label:hover{border-color:var(--line)}
.market-toolbar>.market-search{width:min(360px,100%);flex:0 1 360px}
.market-toolbar input{min-width:0}
.view-switch{margin-left:auto}
@media(max-width:560px){.market-toolbar>.market-search{flex:1 1 100%}}
</style>
