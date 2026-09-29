<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { AButton, ADatePeriodRange, AEmpty, ALoadingState, useToast } from '@aster/ui'
import { formatDate, formatMoney, request, sumMoneyAmounts, type MoneySnapshot } from '@aster/sdk'
import { locale } from '../i18n'

type Period = '1d' | '7d' | '30d' | 'custom'
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const today = new Date()
const localDate = (value: Date) => `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
const maxDate = localDate(today)
const period = ref<Period>('7d')
const from = ref(localDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - 6)))
const to = ref(maxDate)
const options = computed(() => [
  { value: '1d', label: tx('今天', 'Today') }, { value: '7d', label: tx('近 7 天', 'Last 7 days') },
  { value: '30d', label: tx('近 30 天', 'Last 30 days') },
])
const loading = ref(true)
const money = ref<MoneySnapshot | null>(null)
const dateBounds = computed(() => {
  if (period.value === 'custom') return { from: from.value, to: to.value }
  const days = period.value === '1d' ? 1 : period.value === '30d' ? 30 : 7
  return { from: localDate(new Date(today.getFullYear(), today.getMonth(), today.getDate() - days + 1)), to: maxDate }
})
const daily = computed(() => (money.value?.daily ?? []).filter(item => item.date >= dateBounds.value.from && item.date <= dateBounds.value.to))
const maxCost = computed(() => Math.max(0, ...daily.value.map(item => Number(item.cost))))
const periodRequests = computed(() => daily.value.reduce((total, item) => total + item.requests, 0))
const periodCost = computed(() => sumMoneyAmounts(daily.value.map(item => item.cost)))
const modelRows = computed(() => [...(money.value?.models ?? [])].sort((a, b) => Number(b.cost) - Number(a.cost)))
const recent = computed(() => (money.value?.entries ?? []).filter(item => item.kind === 'charge' || item.kind === 'anomaly').slice(0, 10))
async function load() {
  loading.value = true
  try { money.value = await request<MoneySnapshot>('/api/member/money') }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('费用分析加载失败', 'Could not load cost analytics')) }
  finally { loading.value = false }
}
onMounted(load)
</script>

<template>
  <div class="content cost-analytics">
    <header class="page-head"><div><h1>{{ tx('费用分析', 'Cost analytics') }}</h1></div><div class="analytics-actions"><ADatePeriodRange v-model="period" v-model:from="from" v-model:to="to" :options="options" :label="tx('统计周期', 'Reporting period')" :max="maxDate" :max-range-days="366" :start-label="tx('开始日期', 'Start date')" :end-label="tx('结束日期', 'End date')" :locale="locale" /><AButton variant="secondary" :loading="loading" @click="load">{{ tx('刷新', 'Refresh') }}</AButton></div></header>
    <ALoadingState v-if="loading" :label="tx('正在读取费用…', 'Loading costs…')" />
    <template v-else-if="money">
      <div class="summary-grid"><article class="card"><span>{{ tx('当前余额', 'Balance') }}</span><strong>{{ formatMoney(money.balance, money.currency, 2) }}</strong></article><article class="card"><span>{{ tx('本期费用', 'Period cost') }}</span><strong>{{ formatMoney(periodCost, money.currency) }}</strong></article><article class="card"><span>{{ tx('本期请求', 'Period requests') }}</span><strong>{{ periodRequests }}</strong></article><article class="card"><span>{{ tx('费用异常', 'Billing errors') }}</span><strong>{{ money.billing_errors }}</strong></article></div>
      <div class="analytics-grid"><section class="card"><h2>{{ tx('每日费用', 'Daily costs') }}</h2><div v-if="daily.length" class="cost-bars" role="img" :aria-label="tx('每日费用柱状图', 'Daily cost bar chart')"><div v-for="item in daily" :key="item.date" class="cost-bar-column" :title="`${item.date} · ${formatMoney(item.cost, money.currency)} · ${item.requests} ${tx('次请求', 'requests')}`"><i :style="{ height: `${maxCost ? Math.max(2, Number(item.cost) / maxCost * 100) : 2}%` }"></i><small>{{ item.date.slice(5) }}</small></div></div><AEmpty v-else icon="chart" :title="tx('本期暂无费用', 'No costs in this period')" /></section><section class="card"><h2>{{ tx('模型费用分布（累计）', 'Costs by model (all time)') }}</h2><div v-if="modelRows.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('请求', 'Requests') }}</th><th>{{ tx('费用', 'Cost') }}</th></tr></thead><tbody><tr v-for="item in modelRows" :key="item.model"><td>{{ item.model }}</td><td>{{ item.requests }}</td><td>{{ formatMoney(item.cost, money.currency) }}</td></tr></tbody></table></div><AEmpty v-else icon="model" :title="tx('暂无模型费用', 'No model costs yet')" /></section></div>
      <section class="card recent"><h2>{{ tx('最近结算', 'Recent settlements') }}</h2><div v-if="recent.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('费用', 'Cost') }}</th></tr></thead><tbody><tr v-for="item in recent" :key="item.id"><td>{{ formatDate(item.created_at, locale) }}</td><td>{{ item.details.public_model || '—' }}</td><td>{{ item.kind === 'charge' ? tx('已结算', 'Settled') : tx('费用异常', 'Billing error') }}</td><td>{{ formatMoney(item.kind === 'charge' ? item.amount.slice(1) : '0', money.currency) }}</td></tr></tbody></table></div><AEmpty v-else icon="audit" :title="tx('暂无结算记录', 'No settlements yet')" /></section>
    </template>
  </div>
</template>

<style scoped>
.analytics-actions{display:flex;gap:10px;align-items:center;flex-wrap:wrap}.summary-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:12px}.summary-grid article{display:grid;gap:10px;padding:18px}.summary-grid span{color:var(--muted)}.summary-grid strong{font-size:var(--font-size-title);font-variant-numeric:tabular-nums;overflow-wrap:anywhere}.analytics-grid{display:grid;grid-template-columns:1fr 1fr;gap:14px;margin-top:14px}.analytics-grid>.card,.recent{padding:18px}.analytics-grid h2,.recent h2{margin:0 0 14px}.cost-bars{height:220px;display:flex;align-items:end;gap:5px}.cost-bar-column{height:100%;flex:1;min-width:0;display:flex;align-items:center;justify-content:end;flex-direction:column;gap:7px}.cost-bar-column i{display:block;width:100%;max-width:30px;min-height:2px;background:var(--accent);border-radius:5px 5px 0 0}.cost-bar-column small{font-size:var(--font-size-caption);color:var(--muted);white-space:nowrap}.recent{margin-top:14px}@media(max-width:1000px){.summary-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.analytics-grid{grid-template-columns:1fr}}@media(max-width:560px){.summary-grid{grid-template-columns:1fr 1fr}.summary-grid article{padding:12px}.cost-bar-column small{writing-mode:vertical-rl}}
</style>
