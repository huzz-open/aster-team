<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { AButton, ACheckbox, AInfoTip, ALoadingState, AModal, ASelect, useToast } from '@aster/ui'
import { request, type Model } from '@aster/sdk'
import { locale } from '../i18n'

type Currency = 'CNY' | 'USD'
type PriceKind = 'tokens' | 'context_tokens' | 'images'
type TokenField = 'input' | 'cached_read' | 'cached_write' | 'output' | 'image_input' | 'image_output'
type TokenRates = Record<TokenField, number | null>
type RateForm = { tokens: Record<TokenField, string>; long_tokens: Record<TokenField, string>; input_threshold: string; images: Array<{ size: string; quality: string; amount: string }> }
type WindowForm = { start_date: string; end_date: string; weekdays: string; start_time: string; end_time: string; rate: RateForm }
type TierForm = { id: string; base: RateForm; windows: WindowForm[] }
type RateCard = { kind: 'tokens'; rates: TokenRates } | { kind: 'context_tokens'; rates: { input_threshold: number; short: TokenRates; long: TokenRates } } | { kind: 'images'; rates: { unit_prices: Array<{ spec: { size: string; quality: string }; unit_price: { currency: Currency; nanos: number } }> } }
type PriceWindow = { start_date: string | null; end_date: string | null; weekdays: number; start_minute: number; end_minute: number; price: RateCard }
type PricePlan = { public_model: string; version: string; currency: Currency; tiers: Array<{ id: string; schedule: { base: RateCard; windows: PriceWindow[] } }> }
type PriceBook = { active_version: string | null; sync_supported?: boolean; versions: Array<{ plan: PricePlan; source: 'manual' | 'official' | 'builtin'; source_url: string | null; verified_at: string | null }> }

const props = defineProps<{ model: Model }>()
const emit = defineEmits<{ close: []; saved: []; updated: [] }>()
const toast = useToast()
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const loading = ref(true)
const saving = ref(false)
const syncing = ref(false)
const existing = ref<PriceBook | null>(null)
const activePrice = computed(() => existing.value?.versions.find(item => item.plan.version === existing.value?.active_version))
const form = reactive<{ currency: Currency; kind: PriceKind; tiers: TierForm[] }>({
  currency: 'USD', kind: 'tokens', tiers: [],
})
const currencies = computed(() => [{ value: 'USD', label: tx('美元（USD）', 'US dollar (USD)') }, { value: 'CNY', label: tx('人民币（CNY）', 'Chinese yuan (CNY)') }])
const units = computed(() => [{ value: 'tokens', label: tx('每百万 Token', 'Per million tokens') }, { value: 'images', label: tx('每张图片', 'Per image') }])
const selectedUnit = computed(() => form.kind === 'images' ? 'images' : 'tokens')
const syncDisabledReason = computed(() => props.model.public_name.startsWith('gpt-image-')
  ? tx('官方分别公布文字输入、图片输入和图片输出价格；当前无法核对完整的分项用量。可按图片规格设置每张价格。', 'Official prices separate text input, image input, and image output. Complete itemized usage is unavailable here. You can set a price per image specification.')
  : tx('该模型暂无可同步的官方标准价，请手动设置。', 'No official Standard price can be synced for this model. Set a price manually.'))
function selectUnit(value: string | number) {
  form.kind = value === 'images' ? 'images' : 'tokens'
  if (form.kind === 'images') form.tiers = form.tiers.filter(tier => tier.id === 'standard')
}
function setLongContext(value: boolean | string[]) { form.kind = value === true ? 'context_tokens' : 'tokens' }
const tokenFields: Array<{ key: TokenField; zh: string; en: string }> = [
  { key: 'input', zh: '输入', en: 'Input' }, { key: 'cached_read', zh: '缓存读取', en: 'Cached read' },
  { key: 'cached_write', zh: '缓存写入', en: 'Cached write' }, { key: 'output', zh: '输出', en: 'Output' },
  { key: 'image_input', zh: '图片输入', en: 'Image input' }, { key: 'image_output', zh: '图片输出', en: 'Image output' },
]
const blankTokens = (): Record<TokenField, string> => ({ input: '', cached_read: '', cached_write: '', output: '', image_input: '', image_output: '' })
const blankRate = (): RateForm => ({
  tokens: blankTokens(), long_tokens: blankTokens(), input_threshold: '272000',
  images: [{ size: '1024x1024', quality: 'standard', amount: '' }],
})
const cloneRate = (value: RateForm): RateForm => JSON.parse(JSON.stringify(value)) as RateForm
const nanosText = (value: number): string => {
  if (!Number.isSafeInteger(value)) throw new Error(tx('价格超出界面可安全编辑范围', 'Price exceeds the safe editor range'))
  const whole = Math.trunc(value / 1_000_000_000)
  const fraction = String(value % 1_000_000_000).padStart(9, '0').replace(/0+$/, '')
  return fraction ? `${whole}.${fraction}` : String(whole)
}
function parseNanos(value: string): number {
  if (!/^\d+(?:\.\d{1,9})?$/.test(value)) throw new Error(tx('价格需填写最多 9 位小数的非负金额', 'Enter a non-negative price with up to 9 decimal places'))
  const [whole, fraction = ''] = value.split('.')
  const nanos = BigInt(whole!) * 1_000_000_000n + BigInt((fraction + '000000000').slice(0, 9))
  if (nanos > BigInt(Number.MAX_SAFE_INTEGER)) throw new Error(tx('价格过大', 'Price is too large'))
  return Number(nanos)
}
function timeMinute(value: string, end = false): number {
  const match = /^(\d{2}):(\d{2})$/.exec(value)
  if (!match) throw new Error(tx('时间需使用 HH:MM 格式', 'Use HH:MM time format'))
  const hour = Number(match[1]), minute = Number(match[2])
  if (hour > (end ? 24 : 23) || minute > 59 || (hour === 24 && minute !== 0)) throw new Error(tx('时间超出范围', 'Time is out of range'))
  return hour * 60 + minute
}
function dayMask(value: string): number {
  const days = value.split(',').map(item => Number(item.trim()))
  if (!days.length || days.some(day => !Number.isInteger(day) || day < 1 || day > 7)) throw new Error(tx('星期填写 1–7，以逗号分隔', 'Enter weekdays 1–7, separated by commas'))
  return [...new Set(days)].reduce((mask, day) => mask | (1 << (day - 1)), 0)
}
function fromCard(card: RateCard): RateForm {
  const rate = blankRate()
  if (card.kind === 'tokens') {
    for (const field of tokenFields) {
      const value = card.rates[field.key]
      rate.tokens[field.key] = value == null ? '' : nanosText(value)
    }
  } else if (card.kind === 'context_tokens') {
    rate.input_threshold = String(card.rates.input_threshold)
    for (const field of tokenFields) {
      const short = card.rates.short[field.key], long = card.rates.long[field.key]
      rate.tokens[field.key] = short == null ? '' : nanosText(short)
      rate.long_tokens[field.key] = long == null ? '' : nanosText(long)
    }
  } else {
    rate.images = card.rates.unit_prices
      .map(item => ({ size: item.spec.size, quality: item.spec.quality, amount: nanosText(item.unit_price.nanos) }))
  }
  return rate
}
function toCard(rate: RateForm): RateCard {
  const tokenPrices = (values: Record<TokenField, string>): TokenRates => Object.fromEntries(tokenFields.map(field => [field.key, values[field.key].trim() === '' ? null : parseNanos(values[field.key].trim())])) as TokenRates
  if (form.kind === 'tokens') {
    const prices = tokenPrices(rate.tokens)
    if (Object.values(prices).every(value => value == null)) throw new Error(tx('至少配置一类 Token 价格', 'Configure at least one token price'))
    return { kind: 'tokens', rates: prices }
  }
  if (form.kind === 'context_tokens') {
    const input_threshold = Number(rate.input_threshold)
    if (!Number.isSafeInteger(input_threshold) || input_threshold <= 0) throw new Error(tx('输入阈值需为正整数', 'Input threshold must be a positive integer'))
    const short = tokenPrices(rate.tokens), long = tokenPrices(rate.long_tokens)
    if (Object.values(short).every(value => value == null) || tokenFields.some(field => (short[field.key] == null) !== (long[field.key] == null))) throw new Error(tx('长短上下文须配置相同的 Token 类型', 'Configure the same token categories for both context lengths'))
    return { kind: 'context_tokens', rates: { input_threshold, short, long } }
  }
  if (!rate.images.length) throw new Error(tx('至少配置一种图片规格', 'Configure at least one image specification'))
  const unit_prices = rate.images.map(item => {
    if (!item.size.trim() || !item.quality.trim()) throw new Error(tx('填写图片尺寸与质量', 'Enter image size and quality'))
    return { spec: { size: item.size.trim(), quality: item.quality.trim() }, unit_price: { currency: form.currency, nanos: parseNanos(item.amount.trim()) } }
  })
  return { kind: 'images', rates: { unit_prices } }
}
function minuteText(minute: number): string { return `${String(Math.floor(minute / 60)).padStart(2, '0')}:${String(minute % 60).padStart(2, '0')}` }
function loadPlan(plan: PricePlan) {
  form.currency = plan.currency
  form.kind = plan.tiers[0]?.schedule.base.kind ?? 'tokens'
  form.tiers = plan.tiers.map(tier => ({
    id: tier.id, base: fromCard(tier.schedule.base),
    windows: tier.schedule.windows.map(window => ({
      start_date: window.start_date ?? '', end_date: window.end_date ?? '',
      weekdays: Array.from({ length: 7 }, (_, index) => index + 1).filter(day => window.weekdays & (1 << (day - 1))).join(','),
      start_time: minuteText(window.start_minute), end_time: minuteText(window.end_minute), rate: fromCard(window.price),
    })),
  }))
}
async function load() {
  loading.value = true
  try {
    existing.value = await request<PriceBook>(`/api/admin/billing/prices?public_model=${encodeURIComponent(props.model.public_name)}`)
    const active = existing.value.versions.find(item => item.plan.version === existing.value?.active_version)
    if (active) loadPlan(active.plan)
    else {
      form.kind = props.model.public_name.startsWith('gpt-image-') ? 'images' : 'tokens'
      form.tiers = [{ id: 'standard', base: blankRate(), windows: [] }]
    }
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('模型价格加载失败', 'Could not load model prices')) }
  finally { loading.value = false }
}
function addTier() { form.tiers.push({ id: 'fast', base: cloneRate(form.tiers[0]?.base ?? blankRate()), windows: [] }) }
function addWindow(tier: TierForm) { tier.windows.push({ start_date: '', end_date: '', weekdays: '1,2,3,4,5,6,7', start_time: '00:00', end_time: '24:00', rate: cloneRate(tier.base) }) }
async function save() {
  saving.value = true
  try {
    if (!form.tiers.length) throw new Error(tx('至少设置一种处理价格', 'Set at least one processing price'))
    const plan: PricePlan = {
      public_model: props.model.public_name, version: `manual-${Date.now()}-${Math.random().toString(36).slice(2, 8)}`, currency: form.currency,
      tiers: form.tiers.map(tier => ({
        id: tier.id.trim(),
        schedule: {
          base: toCard(tier.base),
          windows: tier.windows.map(window => ({
            start_date: window.start_date || null, end_date: window.end_date || null,
            weekdays: dayMask(window.weekdays), start_minute: timeMinute(window.start_time),
            end_minute: timeMinute(window.end_time, true), price: toCard({ ...window.rate, input_threshold: tier.base.input_threshold }),
          })),
        },
      })),
    }
    await request('/api/admin/billing/prices', { method: 'PUT', body: JSON.stringify({ plan }) })
    toast.success(tx('模型价格已保存，新请求将使用此版本。', 'Model price saved. New requests use this version.'))
    emit('saved')
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('价格保存失败', 'Could not save price')) }
  finally { saving.value = false }
}
async function syncOfficial() {
  syncing.value = true
  try {
    const result = await request<PriceBook>('/api/admin/billing/prices/sync', {
      method: 'POST', body: JSON.stringify({ public_model: props.model.public_name }),
    })
    existing.value = { ...result, sync_supported: true }
    if (activePrice.value) loadPlan(activePrice.value.plan)
    toast.success(activePrice.value?.source === 'official'
      ? tx('已同步官方价格，新请求将使用此版本。', 'Official price synced. New requests use this version.')
      : tx('官方价格暂不可读取，已使用内置参考价。', 'Official price unavailable. The built-in reference price is active.'))
    emit('updated')
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('价格同步失败', 'Could not sync price')) }
  finally { syncing.value = false }
}
onMounted(load)
</script>

<template>
  <AModal :open="true" :title="tx(`${model.public_name} · 价格`, `${model.public_name} · Pricing`)" :close-label="tx('关闭', 'Close')" :close-disabled="saving || syncing" @close="emit('close')">
    <ALoadingState v-if="loading" :label="tx('正在读取价格…', 'Loading prices…')" />
    <form v-else class="price-form" @submit.prevent="save">
      <div class="price-meta"><span class="status" :class="{ warning: !activePrice }">{{ activePrice?.source === 'official' ? tx('已同步官方价格', 'Official price synced') : activePrice?.source === 'builtin' ? tx('预设价格', 'Preset price') : activePrice ? tx('管理员设置', 'Custom price') : tx('尚未定价', 'No price set') }}</span><span v-if="activePrice?.verified_at" class="price-checked">{{ tx('核对于', 'Checked') }} {{ activePrice.verified_at }}</span><a v-if="activePrice?.source_url" :href="activePrice.source_url" target="_blank" rel="noopener noreferrer">{{ tx('查看价格来源', 'View price source') }}</a><AInfoTip :text="tx('预设价可直接使用；保存或同步新价格后，新价格生效。留空的用量类型无法计费。平台费用按用量估算，最终以模型官方账单为准；特殊时段按 UTC 时间计算。', 'Preset prices are ready to use. Saving or syncing activates the new price. Blank usage categories cannot be billed. Platform charges are estimates; the provider bill is final. Scheduled rates use UTC.')" :label="tx('价格说明', 'Pricing help')" :width="360" /></div>
      <div class="price-controls"><label class="field"><span>{{ tx('价格货币', 'Price currency') }}</span><ASelect v-model="form.currency" :options="currencies" :aria-label="tx('价格货币', 'Price currency')" /></label><label class="field"><span>{{ tx('计价单位', 'Price unit') }}</span><ASelect :model-value="selectedUnit" :options="units" :aria-label="tx('计价单位', 'Price unit')" @update:model-value="selectUnit" /></label></div>
      <ACheckbox v-if="selectedUnit === 'tokens'" :model-value="form.kind === 'context_tokens'" :label="tx('长上下文使用不同单价', 'Separate long-context rates')" @update:model-value="setLongContext" />
      <section v-for="(tier, tierIndex) in form.tiers" :key="tierIndex" class="price-tier">
        <div class="section-head"><h3>{{ form.kind === 'images' ? tx('图片价格', 'Image prices') : tier.id === 'fast' ? tx('快速处理', 'Fast processing') : tier.id === 'standard' ? tx('标准处理', 'Standard processing') : tx('其他处理价格', 'Other processing price') }}</h3><AButton v-if="tier.id !== 'standard'" size="small" variant="secondary" @click="form.tiers.splice(tierIndex, 1)">{{ tx('移除', 'Remove') }}</AButton></div>
        <h4>{{ form.kind === 'images' ? tx('按图片规格', 'By image specification') : tx('基础价格 · 每百万 Token', 'Base rates · per million tokens') }}</h4>
        <template v-if="form.kind !== 'images'"><label v-if="form.kind === 'context_tokens'" class="field"><span>{{ tx('长上下文门槛（输入 Token）', 'Long-context threshold (input tokens)') }}<AInfoTip :text="tx('输入超过这个数量时，整次请求使用长上下文单价。', 'Above this input amount, long-context rates apply to the entire request.')" /></span><input v-model.trim="tier.base.input_threshold" inputmode="numeric" required></label><h4 v-if="form.kind === 'context_tokens'">{{ tx('普通上下文', 'Short context') }}</h4><div class="rate-grid"><label v-for="field in tokenFields" :key="field.key" class="field"><span>{{ tx(field.zh, field.en) }}</span><input v-model.trim="tier.base.tokens[field.key]" inputmode="decimal" :placeholder="tx('未配置', 'Not set')"></label></div><template v-if="form.kind === 'context_tokens'"><h4>{{ tx('长上下文', 'Long context') }}</h4><div class="rate-grid"><label v-for="field in tokenFields" :key="field.key" class="field"><span>{{ tx(field.zh, field.en) }}</span><input v-model.trim="tier.base.long_tokens[field.key]" inputmode="decimal" :placeholder="tx('未配置', 'Not set')"></label></div></template></template>
        <div v-else class="image-rates"><div v-for="(item, index) in tier.base.images" :key="index" class="price-row"><label class="field"><span>{{ tx('尺寸', 'Size') }}</span><input v-model.trim="item.size" required></label><label class="field"><span>{{ tx('质量', 'Quality') }}</span><input v-model.trim="item.quality" required></label><label class="field"><span>{{ tx('每张价格', 'Price per image') }}</span><input v-model.trim="item.amount" inputmode="decimal" required></label><AButton v-if="tier.base.images.length > 1" size="small" variant="secondary" @click="tier.base.images.splice(index, 1)">{{ tx('移除', 'Remove') }}</AButton></div><AButton size="small" variant="secondary" @click="tier.base.images.push({ size: '', quality: '', amount: '' })">{{ tx('添加图片规格', 'Add image specification') }}</AButton></div>
        <div class="window-head"><h4>{{ tx('特殊时段', 'Time windows') }}</h4><AButton size="small" variant="secondary" @click="addWindow(tier)">{{ tx('添加时段', 'Add window') }}</AButton></div>
        <article v-for="(window, index) in tier.windows" :key="index" class="price-window">
          <div class="section-head"><strong>{{ tx('时段', 'Window') }} {{ index + 1 }}</strong><AButton size="small" variant="secondary" @click="tier.windows.splice(index, 1)">{{ tx('移除', 'Remove') }}</AButton></div>
          <div class="price-row"><label class="field"><span>{{ tx('开始日期（可空）', 'Start date (optional)') }}</span><input v-model="window.start_date" type="date"></label><label class="field"><span>{{ tx('结束日期（不含，可空）', 'End date (exclusive, optional)') }}</span><input v-model="window.end_date" type="date"></label><label class="field"><span>{{ tx('星期（1=周一，逗号分隔）', 'Weekdays (1=Mon, comma-separated)') }}</span><input v-model.trim="window.weekdays" required></label><label class="field"><span>{{ tx('开始时间', 'Start time') }}</span><input v-model.trim="window.start_time" placeholder="00:00" required></label><label class="field"><span>{{ tx('结束时间', 'End time') }}</span><input v-model.trim="window.end_time" placeholder="24:00" required></label></div>
          <template v-if="form.kind !== 'images'"><h4 v-if="form.kind === 'context_tokens'">{{ tx('短上下文 · 每百万 Token', 'Short context · per million tokens') }}</h4><div class="rate-grid"><label v-for="field in tokenFields" :key="field.key" class="field"><span>{{ tx(field.zh, field.en) }}</span><input v-model.trim="window.rate.tokens[field.key]" inputmode="decimal" :placeholder="tx('未配置', 'Unsupported')"></label></div><template v-if="form.kind === 'context_tokens'"><h4>{{ tx('长上下文 · 每百万 Token', 'Long context · per million tokens') }}</h4><div class="rate-grid"><label v-for="field in tokenFields" :key="field.key" class="field"><span>{{ tx(field.zh, field.en) }}</span><input v-model.trim="window.rate.long_tokens[field.key]" inputmode="decimal" :placeholder="tx('未配置', 'Unsupported')"></label></div></template></template>
          <div v-else class="image-rates"><div v-for="(item, itemIndex) in window.rate.images" :key="itemIndex" class="price-row"><label class="field"><span>{{ tx('尺寸', 'Size') }}</span><input v-model.trim="item.size" required></label><label class="field"><span>{{ tx('质量', 'Quality') }}</span><input v-model.trim="item.quality" required></label><label class="field"><span>{{ tx('每张价格', 'Price per image') }}</span><input v-model.trim="item.amount" inputmode="decimal" required></label></div></div>
        </article>
      </section>
      <div class="form-actions"><AButton v-if="form.kind !== 'images' && !form.tiers.some(tier => tier.id === 'fast')" variant="secondary" @click="addTier">{{ tx('添加快速处理价格', 'Add fast-processing rates') }}</AButton><div class="form-actions-right"><span :tabindex="existing?.sync_supported ? undefined : 0" :title="existing?.sync_supported ? undefined : syncDisabledReason"><AButton type="button" variant="secondary" :loading="syncing" :disabled="saving || !existing?.sync_supported" @click="syncOfficial">{{ tx('同步官方价格', 'Sync official price') }}</AButton></span><AButton type="submit" :loading="saving" :disabled="syncing">{{ tx('保存价格', 'Save prices') }}</AButton></div></div>
    </form>
  </AModal>
</template>

<style scoped>
.price-form{display:grid;gap:18px;max-height:min(72vh,800px);overflow:auto;padding-right:4px}.price-meta{display:flex;align-items:center;gap:8px;flex-wrap:wrap}.price-checked{color:var(--muted);font-size:var(--font-size-caption)}.price-controls{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px}.price-row,.rate-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(150px,1fr));gap:10px}.price-row>.a-button{align-self:end}.price-tier,.price-window{border:1px solid var(--line);border-radius:10px;padding:14px;display:grid;gap:12px}.price-tier .section-head{margin-bottom:0}.price-tier h4{margin:0}.window-head{display:flex;align-items:center;justify-content:space-between}.image-rates{display:grid;gap:10px}.price-window{background:var(--surface-2)}.form-actions,.form-actions-right{display:flex;justify-content:space-between;gap:12px;flex-wrap:wrap}@media(max-width:560px){.price-controls{grid-template-columns:1fr}}
</style>
