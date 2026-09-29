<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { AButton, AEmpty, ALoadingState, ASelect, useToast } from '@aster/ui'
import { formatDate, formatMoney, request, type MoneyEntry, type MoneySnapshot } from '@aster/sdk'
import { locale } from '../i18n'

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const money = ref<MoneySnapshot | null>(null)
const loading = ref(true)
const model = ref('')
const status = ref('')
const modelOptions = computed(() => [{ value: '', label: tx('全部模型', 'All models') }, ...(money.value?.models ?? []).map(item => ({ value: item.model, label: item.model }))])
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'charge', label: tx('已结算', 'Settled') },
  { value: 'anomaly', label: tx('费用异常', 'Billing error') },
  { value: 'request_failed', label: tx('请求失败', 'Request failed') },
])
const rows = computed(() => (money.value?.entries ?? []).filter(item =>
  item.kind !== 'grant' && (!model.value || item.details.public_model === model.value) && (!status.value || item.kind === status.value)))
const cost = (entry: MoneyEntry) => entry.kind === 'charge' ? entry.amount.slice(1) : '0'
const statusLabel = (entry: MoneyEntry) => entry.kind === 'charge' ? tx('已结算', 'Settled') : entry.kind === 'anomaly' ? tx('费用异常，按 0 计', 'Billing error, zero charge') : tx('请求失败，未扣费', 'Request failed, no charge')

async function load() {
  loading.value = true
  try { money.value = await request<MoneySnapshot>('/api/member/money') }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('费用明细加载失败', 'Could not load cost details')) }
  finally { loading.value = false }
}
function exportCSV() {
  if (!money.value) return
  const header = ['time', 'status', 'model', 'actual_model', 'price_version', 'tier', 'currency', 'cost', 'reference_id']
  const values = rows.value.map(entry => [entry.created_at, entry.kind, entry.details.public_model, entry.details.actual_model,
    entry.details.price_version, entry.details.tier, money.value?.currency, cost(entry), entry.reference_id]
    .map(value => `"${String(value ?? '').replaceAll('"', '""')}"`).join(','))
  const url = URL.createObjectURL(new Blob([[header.join(','), ...values].join('\n')], { type: 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = `aster-costs-${new Date().toISOString().slice(0, 10)}.csv`
  link.click()
  URL.revokeObjectURL(url)
}
onMounted(load)
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head"><div><h1>{{ tx('费用明细', 'Cost details') }}</h1></div></header>
    <div class="filter-bar"><label class="field"><span>{{ tx('模型', 'Model') }}</span><ASelect v-model="model" :options="modelOptions" :aria-label="tx('模型', 'Model')" searchable /></label><label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="status" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label><AButton variant="secondary" :disabled="!rows.length" @click="exportCSV">{{ tx('导出 CSV', 'Export CSV') }}</AButton></div>
    <div class="table-wrap">
      <ALoadingState v-if="loading" :label="tx('正在读取明细…', 'Loading details…')" />
      <table v-else-if="rows.length" class="flat-data-table"><thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('价格版本', 'Price version') }}</th><th>{{ tx('档位', 'Tier') }}</th><th>{{ tx('费用', 'Cost') }}</th><th>{{ tx('请求 ID', 'Request ID') }}</th></tr></thead><tbody><tr v-for="entry in rows" :key="entry.id"><td>{{ formatDate(entry.created_at, locale) }}</td><td><span class="status" :class="{ off: entry.kind !== 'charge' }" :title="String(entry.details.failure || '')">{{ statusLabel(entry) }}</span></td><td>{{ entry.details.public_model || '—' }}</td><td>{{ entry.details.price_version || '—' }}</td><td>{{ entry.details.tier || '—' }}</td><td><strong>{{ formatMoney(cost(entry), money?.currency) }}</strong></td><td><code :title="entry.reference_id">{{ entry.reference_id.slice(-12) }}</code></td></tr></tbody></table>
      <AEmpty v-else icon="audit" :title="tx('暂无费用记录', 'No cost records yet')" />
    </div>
  </div>
</template>

<style scoped>
.filter-bar{display:flex;gap:12px;align-items:end;flex-wrap:wrap}.filter-bar .field{min-width:180px}.filter-bar>.a-button{margin-left:auto}.flat-data-table{min-width:1000px}.flat-data-table code{color:var(--accent)}
</style>
