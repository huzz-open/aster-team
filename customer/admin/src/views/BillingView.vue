<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { AEmpty, AInfoTip, ALoadingState, useToast } from '@aster/ui'
import { formatMoney, request, type AdminBillingOverview } from '@aster/sdk'
import { locale } from '../i18n'

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const overview = ref<AdminBillingOverview | null>(null)
const loading = ref(true)
const members = computed(() => overview.value?.configured ? overview.value.members : [])
const currency = computed(() => overview.value?.configured ? overview.value.currency : null)
const search = ref('')
const rows = computed(() => members.value.filter(member =>
  `${member.name} ${member.email}`.toLowerCase().includes(search.value.trim().toLowerCase())))

async function load() {
  loading.value = true
  try { overview.value = await request<AdminBillingOverview>('/api/admin/billing/overview') }
  catch (error) { toast.error(error instanceof Error ? error.message : tx('费用加载失败', 'Could not load costs')) }
  finally { loading.value = false }
}
onMounted(load)
</script>

<template>
  <div class="content billing-page">
    <header class="page-head"><div><h1>{{ tx('费用', 'Billing') }}</h1></div></header>
    <ALoadingState v-if="loading" :label="tx('正在读取费用…', 'Loading costs…')" />
    <template v-else-if="overview?.configured">
      <div class="summary-grid">
        <article class="card"><span>{{ tx('成员余额合计', 'Member balances') }}</span><strong>{{ formatMoney(overview.balance, overview.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('累计发放', 'Total granted') }}</span><strong>{{ formatMoney(overview.credited, overview.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('累计费用', 'Total cost') }} <AInfoTip :text="tx('费用按已确认用量和配置价格计算；实际结算以模型官方账单为准。', 'Costs use confirmed usage and configured prices; the provider’s official bill is authoritative.')" /></span><strong>{{ formatMoney(overview.debited, overview.currency) }}</strong></article>
      </div>
      <div class="filter-bar"><label class="field"><span>{{ tx('搜索成员', 'Search members') }}</span><input v-model="search" :placeholder="tx('名称或邮箱', 'Name or email')"></label></div>
      <div class="table-wrap"><table v-if="rows.length" class="flat-data-table"><thead><tr><th>{{ tx('成员', 'Member') }}</th><th>{{ tx('邮箱', 'Email') }}</th><th>{{ tx('余额', 'Balance') }}</th><th>{{ tx('累计发放', 'Granted') }}</th><th>{{ tx('累计费用', 'Cost') }}</th></tr></thead><tbody><tr v-for="member in rows" :key="member.identity_id"><td><strong>{{ member.name }}</strong></td><td>{{ member.email }}</td><td>{{ formatMoney(member.balance, currency, 2) }}</td><td>{{ formatMoney(member.credited, currency, 2) }}</td><td>{{ formatMoney(member.debited, currency) }}</td></tr></tbody></table><AEmpty v-else icon="users" :title="tx('暂无匹配成员', 'No matching members')" /></div>
      <div class="billing-links"><RouterLink to="/consumption-logs">{{ tx('查看费用明细', 'View cost details') }}</RouterLink><RouterLink to="/users">{{ tx('管理成员与发放金额', 'Manage members and grants') }}</RouterLink></div>
    </template>
    <AEmpty v-else icon="payment" :title="tx('费用尚未配置', 'Billing is not configured')" />
  </div>
</template>

<style scoped>
.summary-grid{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:14px;margin-bottom:18px}.summary-grid article{display:grid;gap:10px;padding:20px}.summary-grid span{color:var(--muted)}.summary-grid strong{font-size:var(--font-size-title);font-variant-numeric:tabular-nums}.filter-bar{margin-bottom:14px}.filter-bar .field{max-width:480px;width:100%}.billing-links{display:flex;gap:18px;margin-top:16px}.billing-links a{color:var(--accent)}@media(max-width:700px){.summary-grid{grid-template-columns:1fr}}
</style>
