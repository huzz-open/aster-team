<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('member')
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { AButton, AEmpty, AInfoTip, ALoadingState, AModal, APagination, ASelect, useToast } from '@aster/ui'
import { formatDate, formatMoney, request } from '@aster/sdk'
import { locale } from '../i18n'

type QuotaRequest = {
  id: string
  user_id: string
  user_email: string
  user_display_name: string
  amount: string
  currency: 'CNY' | 'USD'
  reason: string
  status: 'pending' | 'approved' | 'rejected'
  review_note: string
  reviewer_email?: string | null
  reviewed_at?: string
  created_at: string
}

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const items = ref<QuotaRequest[]>([])
const page = ref(1)
const pageSize = ref(50)
const total = ref(0)
const loading = ref(true)
const filter = ref('pending')
const keyword = ref('')
let filterTimer: number | undefined
const selected = ref<QuotaRequest | null>(null)
const decision = ref<'approved' | 'rejected'>('approved')
const reviewNote = ref('')
const saving = ref(false)
const filterOptions = computed(() => [
  { value: 'pending', label: tx('待审批', 'Pending') },
  { value: 'all', label: tx('全部', 'All') },
  { value: 'approved', label: tx('已通过', 'Approved') },
  { value: 'rejected', label: tx('已驳回', 'Rejected') },
])

async function load() {
  loading.value = true
  try {
    const query = new URLSearchParams({ limit: String(pageSize.value), offset: String((page.value - 1) * pageSize.value) })
    if (filter.value !== 'all') query.set('status', filter.value)
    if (keyword.value.trim()) query.set('keyword', keyword.value.trim())
    const result = await request<{ items: QuotaRequest[]; total: number }>(`/api/admin/quota-requests?${query}`)
    items.value = result.items; total.value = result.total
    const maximumPage = Math.max(1, Math.ceil(total.value / pageSize.value))
    if (page.value > maximumPage) page.value = maximumPage
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('金额申请加载失败', 'Could not load amount requests'))
  } finally { loading.value = false }
}

function resetFilters() {
  const unchanged = !keyword.value && filter.value === 'pending'
  keyword.value = ''
  filter.value = 'pending'
  if (unchanged) void load()
}

function openReview(item: QuotaRequest, next: 'approved' | 'rejected') {
  selected.value = item
  decision.value = next
  reviewNote.value = ''
}

async function submitReview() {
  if (!selected.value) return
  saving.value = true
  try {
    await request(`/api/admin/quota-requests/${selected.value.id}`, {
      method: 'PATCH', body: JSON.stringify({ status: decision.value, review_note: reviewNote.value }),
    })
    toast.success(decision.value === 'approved'
      ? tx('申请已通过，金额已发放到成员余额。', 'Request approved and amount added to the member balance.')
      : tx('申请已驳回。', 'Request rejected.'))
    selected.value = null
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('审批失败', 'Could not review request'))
  } finally { saving.value = false }
}

function statusLabel(status: QuotaRequest['status']) {
  return status === 'pending' ? tx('待审批', 'Pending') : status === 'approved' ? tx('已通过', 'Approved') : tx('已驳回', 'Rejected')
}

onMounted(load)
watch([page, pageSize], () => void load())
watch([keyword, filter], () => {
  window.clearTimeout(filterTimer)
  filterTimer = window.setTimeout(() => {
    if (page.value === 1) void load()
    else page.value = 1
  }, 250)
})
onBeforeUnmount(() => window.clearTimeout(filterTimer))
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head"><div><h1>{{ tx('金额申请审批', 'Amount request reviews') }}</h1></div></header>
    <div class="filter-bar quota-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :placeholder="tx('成员、邮箱、申请原因或审批说明', 'Member, email, reason, or review note')"></label>
      <label class="field status-field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="filter" :options="filterOptions" :aria-label="tx('状态', 'Status')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>
    <section class="card section paginated-panel">
      <div class="section-head quota-toolbar"><h2>{{ tx('申请工单', 'Request tickets') }}<AInfoTip :text="tx('每个成员同时只能有一条待审批申请。', 'A member can have only one pending request.')" /></h2></div>
      <ALoadingState v-if="loading" :label="tx('正在读取金额申请…', 'Loading amount requests…')" />
      <div v-else-if="items.length" class="table-wrap paginated-scroll"><table class="flat-data-table"><thead><tr><th>{{ tx('成员', 'Member') }}</th><th>{{ tx('邮箱', 'Email') }}</th><th>{{ tx('申请金额', 'Requested amount') }}</th><th>{{ tx('申请原因', 'Reason') }}</th><th>{{ tx('审批说明', 'Review note') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('审批人', 'Reviewer') }}</th><th>{{ tx('提交时间', 'Submitted') }}</th><th>{{ tx('审批时间', 'Reviewed') }}</th><th>{{ tx('操作', 'Actions') }}</th></tr></thead><tbody><tr v-for="item in items" :key="item.id"><td><strong>{{ item.user_display_name }}</strong></td><td>{{ item.user_email }}</td><td><strong>{{ formatMoney(item.amount, item.currency, 2) }}</strong></td><td class="reason-cell">{{ item.reason }}</td><td class="reason-cell">{{ item.review_note || '—' }}</td><td><span class="status" :class="{ off:item.status==='rejected', warning:item.status==='pending' }">{{ statusLabel(item.status) }}</span></td><td>{{ item.reviewer_email || '—' }}</td><td>{{ formatDate(item.created_at, locale) }}</td><td>{{ formatDate(item.reviewed_at, locale) }}</td><td><div v-if="item.status==='pending'" class="table-actions"><AButton icon="check" size="small" :disabled="!canWrite" @click="openReview(item,'approved')">{{ tx('通过', 'Approve') }}</AButton><AButton size="small" variant="secondary" :disabled="!canWrite" @click="openReview(item,'rejected')">{{ tx('驳回', 'Reject') }}</AButton></div><span v-else>—</span></td></tr></tbody></table></div>
      <AEmpty v-else icon="ticket" :title="tx('没有匹配的金额申请', 'No matching amount requests')" />
      <APagination v-if="total > 0" v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="loading" :locale="locale" />
    </section>
    <AModal :open="!!selected" :title="decision==='approved'?tx('通过金额申请','Approve amount request'):tx('驳回金额申请','Reject amount request')" :description="selected ? `${selected.user_display_name} · ${formatMoney(selected.amount, selected.currency, 2)}` : ''" :close-label="tx('关闭','Close')" :close-disabled="saving" @close="selected=null">
      <form class="form" @submit.prevent="submitReview"><div v-if="selected" class="review-request"><strong>{{ selected.reason }}</strong><span>{{ selected.user_email }}</span></div><label class="field"><span>{{ tx('审批说明（可选）', 'Review note (optional)') }}</span><textarea v-model="reviewNote" rows="4" maxlength="500" :placeholder="decision==='approved'?tx('例如：用于本月项目开发','For example: approved for this month’s project'):tx('请说明驳回原因','Explain why the request was rejected')"></textarea><small>{{ reviewNote.length }}/500</small></label><div class="form-actions"><AButton variant="secondary" :disabled="saving" @click="selected=null">{{ tx('取消','Cancel') }}</AButton><AButton type="submit" :variant="decision==='rejected'?'danger':'primary'" :loading="saving" :disabled="!canWrite">{{ decision==='approved'?tx('确认通过并发放','Approve and grant'):tx('确认驳回','Reject request') }}</AButton></div></form>
    </AModal>
  </div>
</template>

<style scoped>
.quota-filter .search-field{min-width:280px;flex:1 1 520px}.quota-filter .status-field{max-width:240px}.quota-filter>.a-button{margin-left:auto}.quota-toolbar{align-items:flex-start}.reason-cell{min-width:240px;max-width:380px;white-space:normal!important}.review-request{display:grid;gap:6px;padding:13px;border:1px solid var(--line);border-radius:11px;background:var(--surface-2)}.review-request span{color:var(--muted);font-size:var(--font-size-caption)}@media(max-width:760px){.quota-toolbar{display:grid}}
</style>
