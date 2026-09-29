<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { AButton, AEmpty, AInfoTip, ALoadingState, AModal, useToast } from '@aster/ui'
import { formatDate, formatMoney, request, type MoneySnapshot } from '@aster/sdk'
import { locale } from '../i18n'
import { memberCanWrite as canWrite } from '../license-status'

type AmountRequest = { id: string; amount: string; currency: 'CNY' | 'USD'; reason: string; status: 'pending' | 'approved' | 'rejected'; review_note: string; created_at: string }
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const money = ref<MoneySnapshot | null>(null)
const requests = ref<AmountRequest[]>([])
const loading = ref(true)
const saving = ref(false)
const editing = ref<AmountRequest | null>(null)
const formOpen = ref(false)
const withdrawing = ref<AmountRequest | null>(null)
const amount = ref('')
const reason = ref('')
const grants = computed(() => money.value?.entries.filter(entry => entry.kind === 'grant') ?? [])
const hasPending = computed(() => requests.value.some(item => item.status === 'pending'))

async function load() {
  loading.value = true
  try {
    const [snapshot, result] = await Promise.all([request<MoneySnapshot>('/api/member/money'), request<{ items: AmountRequest[] }>('/api/member/quota-requests?limit=100')])
    money.value = snapshot
    requests.value = result.items
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('费用加载失败', 'Could not load billing data')) }
  finally { loading.value = false }
}
function openForm(item: AmountRequest | null = null) {
  editing.value = item
  amount.value = item?.amount ?? ''
  reason.value = item?.reason ?? ''
  formOpen.value = true
}
async function submit() {
  saving.value = true
  try {
    await request(editing.value ? `/api/member/quota-requests/${editing.value.id}` : '/api/member/quota-requests', {
      method: editing.value ? 'PATCH' : 'POST', body: JSON.stringify({ amount: amount.value.trim(), reason: reason.value.trim() }),
    })
    formOpen.value = false
    toast.success(editing.value ? tx('申请已更新', 'Request updated') : tx('申请已提交', 'Request submitted'))
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('申请提交失败', 'Could not submit request')) }
  finally { saving.value = false }
}
async function withdraw() {
  if (!withdrawing.value) return
  saving.value = true
  try {
    await request(`/api/member/quota-requests/${withdrawing.value.id}`, { method: 'DELETE' })
    withdrawing.value = null
    toast.success(tx('申请已撤回', 'Request withdrawn'))
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('撤回失败', 'Could not withdraw request')) }
  finally { saving.value = false }
}
const statusLabel = (status: AmountRequest['status']) => status === 'pending' ? tx('待审批', 'Pending') : status === 'approved' ? tx('已通过', 'Approved') : tx('已驳回', 'Rejected')
onMounted(load)
</script>

<template>
  <div class="content money-page">
    <header class="page-head"><div><h1>{{ tx('费用中心', 'Billing center') }}</h1></div></header>
    <ALoadingState v-if="loading" :label="tx('正在读取费用…', 'Loading billing data…')" />
    <template v-else-if="money">
      <div class="money-summary">
        <article class="card"><span>{{ tx('当前余额', 'Current balance') }}</span><strong>{{ formatMoney(money.balance, money.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('累计发放', 'Total granted') }}</span><strong>{{ formatMoney(money.credited, money.currency, 2) }}</strong></article>
        <article class="card"><span>{{ tx('累计费用', 'Total cost') }} <AInfoTip :text="tx('费用按已确认用量和管理员配置价格计算；实际结算以模型官方账单为准。', 'Costs use confirmed usage and configured prices; the provider’s official bill is authoritative.')" /></span><strong>{{ formatMoney(money.debited, money.currency) }}</strong></article>
      </div>
      <section class="card money-history"><div class="section-head"><h2>{{ tx('金额申请', 'Amount requests') }}</h2><AButton size="small" :disabled="!canWrite || hasPending" @click="openForm()">{{ tx('申请金额', 'Request amount') }}</AButton></div>
        <div v-if="requests.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('提交时间', 'Submitted') }}</th><th>{{ tx('申请金额', 'Requested amount') }}</th><th>{{ tx('申请原因', 'Reason') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('审批说明', 'Review note') }}</th><th>{{ tx('操作', 'Actions') }}</th></tr></thead><tbody><tr v-for="item in requests" :key="item.id"><td>{{ formatDate(item.created_at, locale) }}</td><td>{{ formatMoney(item.amount, item.currency, 2) }}</td><td>{{ item.reason }}</td><td><span class="status" :class="{ warning: item.status === 'pending', off: item.status === 'rejected' }">{{ statusLabel(item.status) }}</span></td><td>{{ item.review_note || '—' }}</td><td><div v-if="item.status === 'pending'" class="request-actions"><AButton size="small" variant="secondary" :disabled="!canWrite" @click="openForm(item)">{{ tx('修改', 'Edit') }}</AButton><AButton size="small" variant="secondary" :disabled="!canWrite" @click="withdrawing = item">{{ tx('撤回', 'Withdraw') }}</AButton></div><span v-else>—</span></td></tr></tbody></table></div>
        <AEmpty v-else icon="ticket" :title="tx('暂无金额申请', 'No amount requests yet')" />
      </section>
      <section class="card money-history"><div class="section-head"><h2>{{ tx('金额发放记录', 'Grant history') }}</h2></div>
        <div v-if="grants.length" class="table-wrap"><table class="flat-data-table"><thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('金额', 'Amount') }}</th><th>{{ tx('原因', 'Reason') }}</th></tr></thead><tbody><tr v-for="item in grants" :key="item.id"><td>{{ formatDate(item.created_at, locale) }}</td><td class="grant-amount">+{{ formatMoney(item.amount, money.currency, 2) }}</td><td>{{ item.details.reason || '—' }}</td></tr></tbody></table></div>
        <AEmpty v-else icon="audit" :title="tx('暂无发放记录', 'No grants yet')" />
      </section>
    </template>
    <AModal :open="formOpen" :title="editing ? tx('修改金额申请', 'Edit amount request') : tx('申请金额', 'Request amount')" :close-label="tx('关闭', 'Close')" :close-disabled="saving" @close="formOpen = false">
      <form class="form" @submit.prevent="submit"><label class="field"><span>{{ tx('申请金额', 'Requested amount') }}（{{ money?.currency }}）</span><input v-model="amount" type="text" inputmode="decimal" required placeholder="100.00"></label><label class="field"><span>{{ tx('申请原因', 'Reason') }}</span><textarea v-model="reason" rows="4" minlength="5" maxlength="500" required></textarea></label><div class="form-actions"><AButton variant="secondary" :disabled="saving" @click="formOpen = false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="saving" :disabled="!canWrite">{{ tx('提交申请', 'Submit request') }}</AButton></div></form>
    </AModal>
    <AModal :open="!!withdrawing" :title="tx('撤回金额申请', 'Withdraw amount request')" :close-label="tx('关闭', 'Close')" :close-disabled="saving" @close="withdrawing = null">
      <div class="form-actions"><AButton variant="secondary" :disabled="saving" @click="withdrawing = null">{{ tx('取消', 'Cancel') }}</AButton><AButton variant="danger" :loading="saving" @click="withdraw">{{ tx('确认撤回', 'Confirm withdrawal') }}</AButton></div>
    </AModal>
  </div>
</template>

<style scoped>
.money-summary{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:16px}.money-summary article{display:grid;gap:10px;padding:20px}.money-summary span{color:var(--muted)}.money-summary strong{font-size:var(--font-size-title);font-variant-numeric:tabular-nums;overflow-wrap:anywhere}.money-history{margin-top:16px;padding:20px}.grant-amount{color:var(--positive);font-weight:700}.request-actions{display:flex;gap:6px}@media(max-width:700px){.money-summary{grid-template-columns:1fr}}
</style>
