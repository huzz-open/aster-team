<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { AButton, AEmpty, ALoadingState, ASelect, useToast } from '@aster/ui'
import { formatDate, formatMoney, request, type AdminBillingOverview, type MoneyEntry } from '@aster/sdk'
import { locale } from '../i18n'

type TeamMoneyEntry = MoneyEntry & { identity_id: string; member_name: string; member_email: string }
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const overview = ref<AdminBillingOverview | null>(null)
const loaded = computed(() => overview.value?.configured ? overview.value : null)
const loading = ref(true)
const member = ref('')
const model = ref('')
const status = ref('')
const memberOptions = computed(() => [{ value: '', label: tx('全部成员', 'All members') }, ...(loaded.value?.members ?? []).map(item => ({ value: item.identity_id, label: `${item.name} · ${item.email}` }))])
const modelOptions = computed(() => [{ value: '', label: tx('全部模型', 'All models') }, ...(loaded.value?.models ?? []).map(item => ({ value: item.model, label: item.model }))])
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'charge', label: tx('已结算', 'Settled') },
  { value: 'anomaly', label: tx('费用异常', 'Billing error') },
  { value: 'request_failed', label: tx('请求失败', 'Request failed') },
])
const rows = computed(() => (loaded.value?.entries ?? []).filter(item =>
  item.kind !== 'grant'
  && (!member.value || item.identity_id === member.value)
  && (!model.value || item.details.public_model === model.value)
  && (!status.value || item.kind === status.value)))
const cost = (entry: TeamMoneyEntry) => entry.kind === 'charge' ? entry.amount.slice(1) : '0'
const statusLabel = (entry: TeamMoneyEntry) => entry.kind === 'charge'
  ? tx('已结算', 'Settled')
  : entry.kind === 'anomaly' ? tx('费用异常，按 0 计', 'Billing error, zero charge') : tx('请求失败，未扣费', 'Request failed, no charge')

async function load() {
  loading.value = true
  try { overview.value = await request<AdminBillingOverview>('/api/admin/billing/overview') }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('团队费用加载失败', 'Could not load team costs')) }
  finally { loading.value = false }
}
function exportCSV() {
  if (!loaded.value) return
  const header = ['time', 'member', 'email', 'status', 'model', 'actual_model', 'price_version', 'tier', 'currency', 'cost', 'reference_id']
  const values = rows.value.map(entry => [entry.created_at, entry.member_name, entry.member_email, entry.kind,
    entry.details.public_model, entry.details.actual_model, entry.details.price_version, entry.details.tier,
    loaded.value?.currency, cost(entry), entry.reference_id]
    .map(value => `"${String(value ?? '').replaceAll('"', '""')}"`).join(','))
  const url = URL.createObjectURL(new Blob([[header.join(','), ...values].join('\n')], { type: 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = `aster-team-costs-${new Date().toISOString().slice(0, 10)}.csv`
  link.click()
  URL.revokeObjectURL(url)
}
onMounted(load)
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head"><div><h1>{{ tx('团队费用日志', 'Team cost log') }}</h1></div></header>
    <div class="filter-bar"><label class="field"><span>{{ tx('成员', 'Member') }}</span><ASelect v-model="member" :options="memberOptions" :aria-label="tx('成员', 'Member')" searchable /></label><label class="field"><span>{{ tx('模型', 'Model') }}</span><ASelect v-model="model" :options="modelOptions" :aria-label="tx('模型', 'Model')" searchable /></label><label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="status" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label><AButton variant="secondary" :disabled="!rows.length" @click="exportCSV">{{ tx('导出 CSV', 'Export CSV') }}</AButton></div>
    <div class="table-wrap">
      <ALoadingState v-if="loading" :label="tx('正在读取团队费用…', 'Loading team costs…')" />
      <table v-else-if="rows.length" class="flat-data-table"><thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('成员', 'Member') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('价格版本', 'Price version') }}</th><th>{{ tx('费用', 'Cost') }}</th><th>{{ tx('请求 ID', 'Request ID') }}</th></tr></thead><tbody><tr v-for="entry in rows" :key="entry.id"><td>{{ formatDate(entry.created_at, locale) }}</td><td :title="entry.member_email">{{ entry.member_name }}</td><td><span class="status" :class="{ off: entry.kind !== 'charge' }" :title="String(entry.details.failure || '')">{{ statusLabel(entry) }}</span></td><td>{{ entry.details.public_model || '—' }}</td><td>{{ entry.details.price_version || '—' }}</td><td><strong>{{ formatMoney(cost(entry), loaded?.currency) }}</strong></td><td><code :title="entry.reference_id">{{ entry.reference_id.slice(-12) }}</code></td></tr></tbody></table>
      <AEmpty v-else icon="audit" :title="overview?.configured === false ? tx('请先配置费用结算', 'Configure billing first') : tx('暂无费用记录', 'No costs yet')" />
    </div>
  </div>
</template>

<style scoped>
.filter-bar{display:flex;gap:12px;align-items:end;flex-wrap:wrap}.filter-bar .field{min-width:160px;flex:1}.filter-bar>.a-button{margin-left:auto}.flat-data-table{min-width:980px}.flat-data-table code{color:var(--accent)}
</style>
