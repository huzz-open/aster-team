<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, ACopyCode, AEmpty, ALoadingState, AModal, APagination, ASelect, useToast } from '@aster/ui'
import { createCommercialOrder, getCommercialOrder, getCurrentOperationsOperatorID, listCommercialOrders, listCommercialPlans, listCustomers, OperationsAPIError, type CommercialOrderRecord, type CommercialPlanRecord, type CreateCommercialOrderInput, type Customer } from '../api/client'
import CommercialPlanSummary from '../components/CommercialPlanSummary.vue'
import QuotationOrderCreate from '../components/QuotationOrderCreate.vue'
import { formatAmount, termAmount } from '../commercial/plan-form'
import { orderInput } from '../commercial/order-form'
import { submissionJournal } from '../commercial/submission-journal'
import { presentationLabel } from '../presentation'

const toast = useToast()
const router = useRouter() as ReturnType<typeof useRouter> | undefined
const items = ref<CommercialOrderRecord[]>([])
const loading = ref(false); const loadError = ref('')
const page = ref(1); const pageSize = ref(50); const total = ref(0)
const keyword = ref(''); const statusFilter = ref('')
let filterTimer: number | undefined
let loadRequest = 0
const open = ref(false); const saving = ref(false); const formError = ref('')
const choicesLoading = ref(false); const choicesError = ref('')
const customers = ref<Customer[]>([]); const nextCustomer = ref('')
const plans = ref<CommercialPlanRecord[]>([])
const form = reactive({ customerID: '', planID: '', years: '', startsAt: '' })
const journal = submissionJournal<CreateCommercialOrderInput>('order', getCurrentOperationsOperatorID())
const pending = journal.pending
const journalError = journal.error
if (pending.value) {
  const input = pending.value
  Object.assign(form, { customerID: input.customer_id, planID: input.plan_id, years: String(input.years), startsAt: input.starts_at?.slice(0, 19) ?? '' })
}
const detail = ref<CommercialOrderRecord | null>(null)
const detailOpen = ref(false); const detailLoading = ref(false); const detailError = ref('')
let detailRequest = 0
const selectedPlan = computed(() => plans.value.find(plan => plan.snapshot.plan_id === form.planID))
const annualOffer = computed(() => { const offer = selectedPlan.value?.snapshot.definition.offer; return offer?.kind === 'annual' ? offer : undefined })
const customerOptions = computed(() => customers.value.filter(customer => customer.status !== 'inactive').map(customer => ({ value: customer.id, label: customer.name, description: customer.legal_name || customer.id })))
const planOptions = computed(() => plans.value.filter(plan => plan.snapshot.definition.offer.kind === 'annual').map(plan => ({ value: plan.snapshot.plan_id, label: `${plan.snapshot.definition.name} · v${plan.snapshot.version}`, description: plan.snapshot.definition.code })))
const yearOptions = computed(() => annualOffer.value?.terms.map(term => ({ value: String(term.years), label: `${term.years} 年 · ${term.discount_basis_points / 100}%` })) ?? [])
const statusOptions = [
  { value: '', label: '全部状态' },
  { value: 'pending_payment', label: '待收款' },
  { value: 'fulfillment_pending', label: '待授权' },
  { value: 'fulfilled', label: '已签发' },
  { value: 'cancelled', label: '已取消' },
  { value: 'refunded', label: '已退款' },
]
const estimate = computed(() => {
  const offer = annualOffer.value; const term = offer?.terms.find(item => item.years === Number(form.years))
  return offer && term ? formatAmount(termAmount(offer.annual_amount_minor, term.years, term.discount_basis_points), offer.currency) : ''
})
function message(error: unknown, fallback: string) { return error instanceof Error ? error.message : fallback }
async function load() {
	const request = ++loadRequest
	loading.value = true; loadError.value = ''
	try {
		const result = await listCommercialOrders({ limit: pageSize.value, offset: (page.value - 1) * pageSize.value, keyword: keyword.value.trim(), status: statusFilter.value as CommercialOrderRecord['status'] | '' })
		if (request !== loadRequest) return
		items.value = result.items; total.value = result.total
		const maximumPage = Math.max(1, Math.ceil(total.value / pageSize.value))
		if (page.value > maximumPage) page.value = maximumPage
	}
	catch (error) { if (request === loadRequest) loadError.value = message(error, '读取订单失败') }
	finally { if (request === loadRequest) loading.value = false }
}
async function loadChoices() {
  if (choicesLoading.value || pending.value) return
  choicesLoading.value = true; choicesError.value = ''
  const [customerResult, planResult] = await Promise.allSettled([listCustomers('', 100), listCommercialPlans(100)])
  const errors: string[] = []
  if (customerResult.status === 'fulfilled') { customers.value = customerResult.value.items; nextCustomer.value = customerResult.value.next }
  else errors.push(message(customerResult.reason, '读取客户失败'))
  if (planResult.status === 'fulfilled') plans.value = planResult.value
  else errors.push(message(planResult.reason, '读取套餐失败'))
  choicesError.value = errors.join('；'); choicesLoading.value = false
}
async function moreCustomers() {
  if (!nextCustomer.value || choicesLoading.value) return
  choicesLoading.value = true; choicesError.value = ''
  try { const page = await listCustomers(nextCustomer.value, 100); customers.value.push(...page.items); nextCustomer.value = page.next }
  catch (error) { choicesError.value = message(error, '读取更多客户失败') }
  finally { choicesLoading.value = false }
}
function start() {
  if (!pending.value) { Object.assign(form, { customerID: '', planID: '', years: '', startsAt: '' }); formError.value = ''; void loadChoices() }
  open.value = true
}
async function save() {
  if (saving.value || choicesLoading.value || journalError.value || (!pending.value && choicesError.value)) return
  formError.value = ''
  if (!pending.value) {
    try { journal.prepare(orderInput(form.customerID, selectedPlan.value, Number(form.years), form.startsAt)) }
    catch (error) { formError.value = message(error, '请检查订单'); return }
  }
  saving.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    attempt = journal.begin()
    const record = await createCommercialOrder(pending.value!)
    total.value += items.value.some(item => item.snapshot.order_id === record.snapshot.order_id) ? 0 : 1
    if (page.value === 1) items.value = [record, ...items.value.filter(item => item.snapshot.order_id !== record.snapshot.order_id)].slice(0, pageSize.value)
    journal.clear(); open.value = false; detail.value = record; detailOpen.value = true; detailError.value = ''
    toast.success('订单已创建')
  } catch (error) {
    const definitive = !!attempt && error instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(error.status) && journal.reject(attempt)
    formError.value = message(error, '创建订单失败') + (definitive ? '。输入已保留，可修正后重试' : '。结果尚未确认，请重试原请求')
  } finally { saving.value = false }
}
async function show(record: CommercialOrderRecord) {
  const request = ++detailRequest
  detail.value = record; detailOpen.value = true; detailLoading.value = true; detailError.value = ''
  try { const result = await getCommercialOrder(record.snapshot.order_id); if (request === detailRequest) detail.value = result }
  catch (error) { if (request === detailRequest) detailError.value = message(error, '读取订单详情失败') }
  finally { if (request === detailRequest) detailLoading.value = false }
}
function closeDetail() { detailRequest++; detailOpen.value = false; detailLoading.value = false }
function quotationCreated(record: CommercialOrderRecord) {
  total.value += items.value.some(item => item.snapshot.order_id === record.snapshot.order_id) ? 0 : 1
  if (page.value === 1) items.value = [record, ...items.value.filter(item => item.snapshot.order_id !== record.snapshot.order_id)].slice(0, pageSize.value)
  void show(record)
  toast.success('报价订单已保存')
}
async function copy(text: string) { try { await navigator.clipboard.writeText(text); toast.success('已复制') } catch { toast.error('复制失败，请检查剪贴板权限') } }
function resetFilters() { keyword.value = ''; statusFilter.value = '' }
onMounted(() => void load())
watch([keyword, statusFilter], () => {
  window.clearTimeout(filterTimer)
  filterTimer = window.setTimeout(() => { if (page.value === 1) void load(); else page.value = 1 }, 200)
})
watch([page, pageSize], () => void load())
onBeforeUnmount(() => window.clearTimeout(filterTimer))
</script>

<template>
  <section class="content paginated-page">
    <div class="page-head"><div><p class="eyebrow">COMMERCIAL</p><h1>订单与收款</h1><p>管理订单快照；到账确认和授权签发统一在付费授权工作台办理。</p></div><div class="inline-actions"><QuotationOrderCreate @created="quotationCreated" /><AButton variant="secondary" icon="shield" @click="router?.push('/commercial/fulfillments')">付费授权</AButton><AButton icon="plus" @click="start">{{ pending ? '继续创建' : '新增订单' }}</AButton></div></div>
    <div class="filter-bar">
      <label class="field search-field"><span>搜索</span><input v-model="keyword" placeholder="订单、客户编号或客户名称"></label>
      <label class="field"><span>状态</span><ASelect v-model="statusFilter" aria-label="订单状态" :options="statusOptions" /></label>
      <div class="filter-actions"><AButton variant="secondary" @click="resetFilters">重置</AButton></div>
    </div>
    <p v-if="loadError" class="commercial-error" role="alert">{{ loadError }}</p>
    <p v-if="journalError" class="commercial-error" role="alert">{{ journalError }}</p>
    <p v-else-if="pending" class="commercial-note">有一笔尚未确认结果的订单，请使用继续创建核对原请求。</p>
    <div class="table-wrap paginated-scroll"><ALoadingState v-if="loading && !items.length" label="正在读取订单" />
      <table v-else-if="items.length" class="flat-data-table"><thead><tr><th>订单</th><th>客户</th><th>客户编号</th><th>套餐</th><th>版本</th><th>期限</th><th>金额</th><th>状态</th><th>操作</th></tr></thead><tbody>
        <tr v-for="item in items" :key="item.snapshot.order_id"><td>{{ item.snapshot.order_id }}</td><td><strong>{{ item.customer_name || '—' }}</strong></td><td class="code">{{ item.snapshot.customer_id }}</td><td>{{ item.snapshot.plan.definition.name }}</td><td>v{{ item.snapshot.plan.version }}</td><td>{{ item.snapshot.years }} 年</td><td>{{ formatAmount(item.snapshot.amount_minor, item.snapshot.currency) }}</td><td>{{ presentationLabel('orderStatus', item.status) }}</td><td><div class="row-actions"><AButton variant="secondary" size="small" @click="show(item)">查看</AButton><AButton v-if="item.status !== 'cancelled' && item.status !== 'refunded'" size="small" @click="router?.push({ path: '/commercial/fulfillments', query: { order: item.snapshot.order_id } })">办理</AButton></div></td></tr>
      </tbody></table><AEmpty v-else-if="!loadError" title="还没有权益订单" text="选择客户及固定套餐版本创建订单。免费套餐和联系报价不生成年度付费订单。" />
    </div>
    <APagination v-if="total > 0" v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="loading" />
    <AModal :open="open" title="新增权益订单" description="订单保留所选版本的完整权益，后续修订套餐不会改变此订单。创建订单不代表已收款或批准签发。" :close-disabled="saving" @close="open = false">
      <form class="form" @submit.prevent="save">
        <p v-if="formError" class="commercial-error" role="alert">{{ formError }}</p>
        <p v-if="choicesError" class="commercial-error" role="alert">{{ choicesError }} <AButton variant="secondary" :disabled="choicesLoading" @click="loadChoices">重新读取</AButton></p>
        <ALoadingState v-if="choicesLoading" label="正在读取客户和套餐" />
        <dl v-if="pending" class="commercial-order-info"><dt>客户</dt><dd>{{ pending.customer_id }}</dd><dt>套餐版本</dt><dd>{{ pending.plan_id }} v{{ pending.plan_version }}</dd><dt>订阅期限</dt><dd>{{ pending.years }} 年</dd><dt>合同开始</dt><dd>{{ pending.starts_at }}</dd></dl>
        <fieldset v-show="!pending" class="commercial-fields" :disabled="saving || choicesLoading || !!pending">
          <label class="field"><span>客户</span><ASelect v-model="form.customerID" aria-label="客户" :options="customerOptions" required searchable /></label>
          <AButton v-if="nextCustomer" variant="secondary" @click="moreCustomers">读取更多客户</AButton>
          <label class="field"><span>套餐版本</span><ASelect v-model="form.planID" aria-label="套餐版本" :options="planOptions" required searchable @change="form.years = ''" /></label>
          <p v-if="!choicesLoading && !choicesError && !planOptions.length" class="commercial-note">还没有可选的按年订阅套餐，请先保存一个套餐版本。</p>
          <label class="field"><span>订阅期限</span><ASelect v-model="form.years" aria-label="订阅期限" :options="yearOptions" required /></label>
          <label class="field"><span>合同开始时间（UTC）</span><input v-model="form.startsAt" type="datetime-local" step="1" aria-label="合同开始时间（UTC）" required><small>明确填写 UTC 时间，北京时间减去 8 小时。到期时间按套餐的合同日历计算。</small></label>
        </fieldset>
        <p v-if="estimate" class="commercial-price">应付金额 {{ estimate }}</p>
        <details v-if="selectedPlan"><summary>核对本版本权益</summary><CommercialPlanSummary :definition="selectedPlan.snapshot.definition" /></details>
        <div class="form-actions"><AButton variant="secondary" type="button" :disabled="saving" @click="open = false">返回</AButton><AButton type="submit" :loading="saving" :disabled="!!journalError || choicesLoading || (!pending && !!choicesError)">{{ pending ? '重试原请求' : '创建订单' }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="detailOpen" title="订单权益快照" @close="closeDetail">
      <p v-if="detailError" class="commercial-error" role="alert">{{ detailError }}</p>
      <ALoadingState v-if="detailLoading" label="正在读取订单详情" />
      <template v-else-if="detail">
        <ACopyCode :value="detail.snapshot.order_id" label="复制订单编号" copied-label="已复制" @copy="copy(detail.snapshot.order_id)" />
        <dl class="commercial-order-info"><dt>客户</dt><dd>{{ detail.snapshot.customer_id }}</dd><dt>订单状态</dt><dd>{{ presentationLabel('orderStatus', detail.status) }}</dd><dt>固定套餐版本</dt><dd>{{ detail.snapshot.plan.definition.name }} v{{ detail.snapshot.plan.version }}</dd><dt>成交金额</dt><dd>{{ formatAmount(detail.snapshot.amount_minor, detail.snapshot.currency) }}</dd><dt>合同开始</dt><dd>{{ detail.snapshot.starts_at }}</dd><dt>合同结束</dt><dd>{{ detail.snapshot.ends_at }}</dd></dl>
        <CommercialPlanSummary :definition="detail.snapshot.plan.definition" />
        <dl v-if="detail.snapshot.schema === 'aster.order-snapshot.v2'" class="commercial-order-info"><dt>报价来源</dt><dd>{{ detail.snapshot.source.environment === 'local' ? '本地验证' : '生产官网' }}</dd><dt>发布记录</dt><dd>{{ detail.snapshot.source.publication_id }}</dd><dt>公开目录</dt><dd>{{ detail.snapshot.source.catalog_revision }}</dd><dt>受理时间</dt><dd>{{ detail.snapshot.source.ordered_at }}</dd><dt>受理截止</dt><dd>{{ detail.snapshot.source.accept_until }}</dd></dl>
        <p v-else class="commercial-note">手工固定套餐订单，不包含官网发布来源</p>
        <p class="commercial-note">订单摘要</p><ACopyCode :value="detail.sha256" label="复制订单摘要" copied-label="已复制" @copy="copy(detail.sha256)" />
      </template>
    </AModal>
  </section>
</template>

<style scoped>
.commercial-toolbar { display: flex; flex-wrap: wrap; gap: 12px; justify-content: space-between; align-items: center; margin: 12px 0 18px; }
.commercial-toolbar a, .commercial-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; }
.commercial-note { margin: 16px 0; }
.commercial-error { border: 1px solid var(--line); border-radius: 10px; padding: 12px; font-size:var(--font-size-body); line-height: 1.6; }
.commercial-fields { display: grid; gap: 16px; border: 0; padding: 0; margin: 0; min-width: 0; }
.commercial-price { font-size:var(--font-size-title); font-weight: 650; }
.commercial-order-info { display: grid; grid-template-columns: 100px minmax(0, 1fr); gap: 12px; font-size:var(--font-size-body); margin: 20px 0; }
.commercial-order-info dt { color: var(--muted); }
.commercial-order-info dd { margin: 0; overflow-wrap: anywhere; }
</style>
