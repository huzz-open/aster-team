<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('gateway')
import { computed, onMounted, ref, watch } from 'vue'
import { AButton, AEmpty, AIconButton, AInfoTip, ALoadingState, APagination, ASelect, useToast } from '@aster/ui'
import { formatDate, request, type Model } from '@aster/sdk'
import { locale } from '../i18n'
import BillingPriceEditor from '../components/BillingPriceEditor.vue'

const items = ref<Model[]>([])
const loading = ref(false)
const toast = useToast()
const operatingID = ref('')
const pricingModel = ref<Model | null>(null)
const page = ref(1)
const pageSize = ref(50)
const keyword = ref('')
const providerFilter = ref('')
const statusFilter = ref('')
const priceFilter = ref('')
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const providerLabel = (provider: string) => ({ openai: 'OpenAI', deepseek: 'DeepSeek', glm: 'GLM' } as Record<string, string>)[provider.toLowerCase()] ?? provider
const providerOptions = computed(() => [
  { value: '', label: tx('全部提供方', 'All providers') },
  ...Array.from(new Set(items.value.map(item => item.provider).filter((value): value is string => !!value)))
    .sort((left, right) => providerLabel(left).localeCompare(providerLabel(right)))
    .map(provider => ({ value: provider, label: providerLabel(provider) })),
])
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'available', label: tx('已开放', 'Available') },
  { value: 'unavailable', label: tx('无可用连接', 'No active connection') },
  { value: 'closed', label: tx('已关闭', 'Closed') },
])
const priceOptions = computed(() => [
  { value: '', label: tx('全部价格', 'All prices') },
  { value: 'builtin', label: tx('预设价格', 'Preset') },
  { value: 'official', label: tx('已同步价格', 'Synced') },
  { value: 'manual', label: tx('手动设置', 'Custom') },
  { value: 'unpriced', label: tx('未定价', 'Unpriced') },
])
const filteredItems = computed(() => {
  const needle = keyword.value.trim().toLocaleLowerCase()
  return items.value.filter((item) => {
    const matchesKeyword = !needle || `${item.public_name} ${item.display_name || ''} ${item.provider || ''}`.toLocaleLowerCase().includes(needle)
    const state = !item.enabled ? 'closed' : item.available ? 'available' : 'unavailable'
    return matchesKeyword
      && (!providerFilter.value || item.provider === providerFilter.value)
      && (!statusFilter.value || state === statusFilter.value)
      && (!priceFilter.value || (priceFilter.value === 'unpriced' ? !item.price_source : item.price_source === priceFilter.value))
  })
})
const pagedItems = computed(() => filteredItems.value.slice((page.value - 1) * pageSize.value, page.value * pageSize.value))

async function load() {
  loading.value = true
  try { items.value = (await request<{ items: Model[] }>('/api/admin/models')).items }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('模型加载失败', 'Could not load models')) }
  finally { loading.value = false }
}

async function toggle(item: Model) {
  operatingID.value = item.id
  try {
    await request(`/api/admin/models/${item.id}`, { method: 'PATCH', body: JSON.stringify({ enabled: !item.enabled }) })
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('模型状态更新失败', 'Could not update model status')) }
  finally { operatingID.value = '' }
}

function resetFilters() {
  keyword.value = ''
  providerFilter.value = ''
  statusFilter.value = ''
  priceFilter.value = ''
}

function priceSaved() {
  pricingModel.value = null
  void load()
}

function priceLabel(source: Model['price_source']) {
  if (source === 'manual') return tx('管理员设置', 'Custom')
  if (source === 'official') return tx('已同步', 'Synced')
  if (source === 'builtin') return tx('预设价格', 'Preset')
  return tx('未定价', 'Unpriced')
}

function amount(nanos: number | null | undefined, currency: 'USD' | 'CNY'): string {
  if (nanos == null) return '—'
  const symbol = currency === 'USD' ? '$' : '¥'
  return `${symbol}${new Intl.NumberFormat(locale.value, { maximumFractionDigits: 5 }).format(nanos / 1_000_000_000)}`
}

function priceSummary(item: Model): string {
  const preview = item.price_preview
  if (!preview) return '—'
  const { rate, currency } = preview
  if (rate.kind === 'images') {
    const prices = rate.rates.unit_prices.map(price => price.unit_price.nanos)
    return prices.length ? `${amount(Math.min(...prices), currency)}${tx(' 起/张', ' per image and up')}` : '—'
  }
  const prices = rate.kind === 'context_tokens' ? rate.rates.short : rate.rates
  const base = `${amount(prices.input, currency)} / ${amount(prices.output, currency)}`
  if (rate.kind !== 'context_tokens') return base
  const long = `${amount(rate.rates.long.input, currency)} / ${amount(rate.rates.long.output, currency)}`
  return `${tx('短', 'Short')} ${base} · ${tx('长', 'Long')} ${long}`
}

onMounted(load)
watch(pageSize, () => { page.value = 1 })
watch([keyword, providerFilter, statusFilter, priceFilter], () => { page.value = 1 })
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head"><div><h1>{{ tx('可用模型', 'Available models') }}</h1></div></header>
    <div class="filter-bar model-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :placeholder="tx('模型名称、模型 ID 或厂商', 'Model name, model ID, or provider')"></label>
      <label class="field"><span>{{ tx('提供方', 'Provider') }}</span><ASelect v-model="providerFilter" :options="providerOptions" :aria-label="tx('提供方', 'Provider')" /></label>
      <label class="field status-field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="statusFilter" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label>
      <label class="field"><span>{{ tx('价格状态', 'Pricing') }}</span><ASelect v-model="priceFilter" :options="priceOptions" :aria-label="tx('价格状态', 'Pricing')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>
    <div class="table-wrap paginated-scroll">
      <ALoadingState v-if="loading && !items.length" :label="tx('正在读取模型…', 'Loading models…')" /><table v-else-if="filteredItems.length" class="model-table"><thead><tr><th>{{ tx('模型 ID', 'Model ID') }}</th><th>{{ tx('显示名称', 'Display name') }}</th><th>{{ tx('提供方', 'Provider') }}</th><th>{{ tx('发现时间', 'Discovered') }}</th><th>{{ tx('开放状态', 'Availability') }}</th><th><span class="inline-actions">{{ tx('基础价（输入 / 输出）', 'Base price (input / output)') }}<AInfoTip :text="tx('Token 价格以每百万计。长上下文、快速处理和特殊时段的价格请在详情中查看。', 'Token rates are per million. Open pricing details for long-context, fast-processing, and scheduled rates.')" /></span></th><th><span class="inline-actions">{{ tx('价格来源', 'Price source') }}<AInfoTip :text="tx('预设价格可直接使用；“已同步”表示从官方价格页更新。保存新价格或再次同步后，新价格生效。', 'Preset prices are ready to use. Synced prices were refreshed from the official page. Saving or syncing activates the new price.')" /></span></th><th>{{ tx('操作', 'Actions') }}</th></tr></thead>
        <tbody><tr v-for="item in pagedItems" :key="item.id">
          <td><strong class="code">{{ item.public_name }}</strong></td><td>{{ item.display_name || item.public_name }}</td>
          <td>{{ item.provider ? providerLabel(item.provider) : '—' }}</td><td>{{ formatDate(item.discovered_at, locale) }}</td><td><span class="status" :class="{ off: !item.enabled || !item.available, warning: item.enabled && !item.available }">{{ !item.enabled ? tx('已关闭', 'Closed') : item.available ? tx('已开放', 'Available') : tx('无可用连接', 'No active connection') }}</span></td><td class="model-price" :title="item.price_preview?.has_other_prices ? tx('还有快速处理或特殊时段价格，点击价格操作查看', 'Additional fast-processing or scheduled rates are available. Open pricing for details.') : undefined">{{ priceSummary(item) }}</td><td><span class="status" :class="{ warning: !item.price_source }">{{ priceLabel(item.price_source) }}</span></td>
          <td><div class="model-actions"><AIconButton icon="payment" size="small" variant="accent" :label="tx('配置模型价格', 'Configure model pricing')" :disabled="!canWrite" @click="pricingModel = item" /><AIconButton :icon="item.enabled ? 'pause' : 'play'" size="small" :variant="item.enabled ? 'neutral' : 'accent'" :label="item.enabled ? tx('停止向成员开放', 'Stop member access') : tx('向成员开放', 'Enable member access')" :disabled="!canWrite || operatingID === item.id" @click="toggle(item)" /></div></td>
        </tr></tbody>
      </table>
      <AEmpty v-else :title="items.length ? tx('没有匹配的模型', 'No matching models') : tx('尚无模型', 'No models yet')" />
    </div>
    <APagination v-if="filteredItems.length > 0" v-model:page="page" v-model:page-size="pageSize" :total="filteredItems.length" :locale="locale" inline-page-sizes />
    <BillingPriceEditor v-if="pricingModel" :model="pricingModel" @close="pricingModel = null" @saved="priceSaved" @updated="load" />
  </div>
</template>

<style scoped>
.model-filter{flex-wrap:wrap}.model-filter .search-field{min-width:250px;flex:1 1 320px}.model-filter .field:not(.search-field){min-width:150px;max-width:190px;flex:1 1 150px}.model-filter>.a-button{margin-left:auto}.model-filter+.table-wrap{margin-top:0}.model-table{min-width:1120px}.model-table td,.model-table th{white-space:nowrap}.model-price{font-variant-numeric:tabular-nums}.model-actions{display:flex;align-items:center;gap:8px}@media(max-width:760px){.model-filter .field,.model-filter .search-field{min-width:100%;max-width:none;flex-basis:100%}}
</style>
