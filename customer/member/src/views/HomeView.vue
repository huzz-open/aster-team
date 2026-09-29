<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, AEmpty, AInfoTip, ALoadingState, useToast } from '@aster/ui'
import { formatDate, formatMoney, request, type APIKey, type Model, type MoneySnapshot, type User } from '@aster/sdk'
import { locale, t } from '../i18n'

const router = useRouter()
const toast = useToast()
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const user = ref<User | null>(null)
const money = ref<MoneySnapshot | null>(null)
const keys = ref<APIKey[]>([])
const models = ref<Model[]>([])
const loading = ref(true)
const recent = computed(() => money.value?.entries.filter(item => item.kind === 'charge' || item.kind === 'anomaly').slice(0, 5) ?? [])
const maxDailyCost = computed(() => Math.max(0, ...((money.value?.daily ?? []).map(item => Number(item.cost)))))
const activeKeys = computed(() => keys.value.filter(item => item.status === 'active').length)
const availableModels = computed(() => models.value.filter(item => item.available !== false).length)

async function load() {
  loading.value = true
  try {
    const [profile, snapshot, keyResult, modelResult] = await Promise.all([
      request<User>('/api/member/me'),
      request<MoneySnapshot>('/api/member/money'),
      request<{ items: APIKey[] }>('/api/member/keys'),
      request<{ items: Model[] }>('/api/member/models'),
    ])
    user.value = profile
    money.value = snapshot
    keys.value = keyResult.items
    models.value = modelResult.items
  } catch (value) { toast.error(value instanceof Error ? value.message : t('overviewLoadFailed')) }
  finally { loading.value = false }
}
onMounted(load)
</script>

<template>
  <div class="content money-home">
    <header class="page-head"><div><h1>{{ tx('费用概览', 'Cost overview') }}</h1></div><AButton variant="secondary" @click="router.push('/logs')">{{ tx('查看费用明细', 'View cost details') }}</AButton></header>
    <ALoadingState v-if="loading" :label="tx('正在读取费用…', 'Loading costs…')" />
    <template v-else-if="money">
      <div class="money-metrics">
        <article class="card"><span>{{ tx('当前余额', 'Current balance') }}</span><strong>{{ formatMoney(money.balance, money.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('累计使用', 'Total spent') }} <AInfoTip :text="tx('费用按已确认用量和管理员配置价格计算；实际结算以模型官方账单为准。', 'Costs use confirmed usage and configured prices; the provider’s official bill is authoritative.')" /></span><strong>{{ formatMoney(money.debited, money.currency) }}</strong></article>
        <article class="card"><span>{{ tx('累计发放', 'Total granted') }}</span><strong>{{ formatMoney(money.credited, money.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('可用模型 / 活跃 Key', 'Models / active keys') }}</span><strong>{{ availableModels }} / {{ activeKeys }}</strong></article>
      </div>
      <div class="money-grid">
        <section class="card"><div class="section-head"><h2>{{ tx('每日费用', 'Daily costs') }}</h2></div>
          <div v-if="money.daily.length" class="daily-chart" role="img" :aria-label="tx('每日费用柱状图', 'Daily cost bar chart')"><div v-for="item in money.daily.slice(-14)" :key="item.date" class="daily-column" :title="`${item.date} · ${formatMoney(item.cost, money.currency)} · ${item.requests} ${tx('次请求', 'requests')}`"><div class="daily-bar" :style="{ height: `${maxDailyCost ? Math.max(2, Number(item.cost) / maxDailyCost * 100) : 2}%` }"></div><small>{{ item.date.slice(5) }}</small></div></div>
          <AEmpty v-else icon="chart" :title="tx('暂无费用记录', 'No costs yet')" />
        </section>
        <section class="card"><div class="section-head"><h2>{{ tx('按模型费用', 'Costs by model') }}</h2></div>
          <div v-if="money.models.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('请求', 'Requests') }}</th><th>{{ tx('费用', 'Cost') }}</th></tr></thead><tbody><tr v-for="item in [...money.models].sort((a,b) => Number(b.cost) - Number(a.cost))" :key="item.model"><td>{{ item.model }}</td><td>{{ item.requests }}</td><td>{{ formatMoney(item.cost, money.currency) }}</td></tr></tbody></table></div>
          <AEmpty v-else icon="model" :title="tx('暂无模型费用', 'No model costs yet')" />
        </section>
      </div>
      <section class="card recent-costs"><div class="section-head"><h2>{{ tx('最近结算', 'Recent settlements') }}</h2><AButton size="small" variant="secondary" @click="router.push('/logs')">{{ tx('全部记录', 'All entries') }}</AButton></div>
        <div v-if="recent.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('模型', 'Model') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('费用', 'Cost') }}</th></tr></thead><tbody><tr v-for="item in recent" :key="item.id"><td>{{ formatDate(item.created_at, locale) }}</td><td>{{ item.details.public_model || '—' }}</td><td>{{ item.kind === 'anomaly' ? tx('费用异常，按 0 计', 'Billing error, zero charge') : tx('已结算', 'Settled') }}</td><td>{{ formatMoney(item.kind === 'charge' ? item.amount.slice(1) : '0', money.currency) }}</td></tr></tbody></table></div>
        <AEmpty v-else icon="audit" :title="tx('暂无结算记录', 'No settlements yet')" />
      </section>
    </template>
  </div>
</template>

<style scoped>
.money-metrics{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:14px;margin-bottom:16px}.money-metrics article{display:grid;gap:10px;padding:20px}.money-metrics span{color:var(--muted)}.money-metrics strong{font-size:var(--font-size-title);font-variant-numeric:tabular-nums;overflow-wrap:anywhere}.money-grid{display:grid;grid-template-columns:1fr 1fr;gap:16px}.money-grid>.card,.recent-costs{padding:20px}.recent-costs{margin-top:16px}.daily-chart{height:200px;display:flex;align-items:end;gap:8px;padding-top:16px}.daily-column{height:100%;min-width:0;flex:1;display:flex;align-items:center;justify-content:end;flex-direction:column;gap:8px}.daily-bar{width:100%;max-width:32px;min-height:2px;background:var(--accent);border-radius:5px 5px 0 0}.daily-column small{white-space:nowrap;color:var(--muted);font-size:var(--font-size-caption)}@media(max-width:1000px){.money-metrics{grid-template-columns:repeat(2,minmax(0,1fr))}.money-grid{grid-template-columns:1fr}}@media(max-width:600px){.money-metrics{grid-template-columns:1fr 1fr}.money-metrics article{padding:14px}.money-metrics strong{font-size:var(--font-size-title)}.daily-column small{writing-mode:vertical-rl}}
</style>
