<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AEmpty, ALoadingState, APagination, ASelect, useToast } from '@aster/ui'
import {
  createCommercialOrder,
  getCommercialOrder,
  getCurrentOperationsOperatorID,
  listCommercialOrders,
  listCommercialPlans,
  listCustomers,
  OperationsAPIError,
  type CommercialOrderRecord,
  type CommercialPlanRecord,
  type CreateCommercialOrderInput,
  type Customer,
} from '../api/client'
import { formatAmount, termAmount } from '../commercial/plan-form'
import { orderInput } from '../commercial/order-form'
import { submissionJournal } from '../commercial/submission-journal'
import CommercialFulfillment from '../components/CommercialFulfillment.vue'
import CommercialPaymentConfirm from '../components/CommercialPaymentConfirm.vue'
import WorkflowStepper from '../components/WorkflowStepper.vue'

type Stage = 'pending_payment' | 'pending_approval' | 'pending_issue' | 'issued'
type Workspace = 'workflow' | 'records'
type PaymentController = { startForOrder: (orderID: string) => Promise<void> }
type FulfillmentController = {
  startForOrder: (orderID: string) => Promise<void>
  startSeeded: (orderID: string, kind: 'renewal' | 'upgrade', sourceID: string) => Promise<void>
}

const steps = ['选择客户', '创建订单', '确认到账', '导入申请', '核对批准', '选择证书', '签发授权', '下载交付']
const route = useRoute()
const router = useRouter()
const toast = useToast()
const payment = ref<PaymentController | null>(null)
const fulfillment = ref<FulfillmentController | null>(null)
const workspace = ref<Workspace>('records')
const currentStep = ref(1)
const maxReachedStep = ref(1)
const activeOrder = ref<CommercialOrderRecord | null>(null)
const customers = ref<Customer[]>([])
const plans = ref<CommercialPlanRecord[]>([])
const choicesLoading = ref(false)
const choicesError = ref('')
const form = reactive({ customerID: '', planID: '', years: '', startsAt: '' })
const saving = ref(false)
const formError = ref('')
const journal = submissionJournal<CreateCommercialOrderInput>('order', getCurrentOperationsOperatorID())
const draftStorageKey = `aster-operations-order-draft:${getCurrentOperationsOperatorID()}`

const records = ref<CommercialOrderRecord[]>([])
const page = ref(1)
const pageSize = ref(50)
const total = ref(0)
const keyword = ref('')
const stageFilter = ref<Stage | ''>('')
const recordsLoading = ref(false)
const recordsError = ref('')
let filterTimer: number | undefined
let loadRequest = 0
let routeRequest = 0

const selectedCustomer = computed(() => customers.value.find((customer) => customer.id === form.customerID))
const selectedPlan = computed(() => plans.value.find((plan) => plan.snapshot.plan_id === form.planID))
const annualOffer = computed(() => {
  const offer = selectedPlan.value?.snapshot.definition.offer
  return offer?.kind === 'annual' ? offer : undefined
})
const customerOptions = computed(() => customers.value
  .filter((customer) => customer.status !== 'inactive')
  .map((customer) => ({ value: customer.id, label: customer.name })))
const planOptions = computed(() => plans.value
  .filter((plan) => plan.snapshot.definition.offer.kind === 'annual')
  .map((plan) => ({ value: plan.snapshot.plan_id, label: `${plan.snapshot.definition.name} · v${plan.snapshot.version}` })))
const yearOptions = computed(() => annualOffer.value?.terms.map((term) => ({
  value: String(term.years), label: `${term.years} 年 · ${term.discount_basis_points / 100}%`,
})) ?? [])
const estimate = computed(() => {
  const offer = annualOffer.value
  const term = offer?.terms.find((item) => item.years === Number(form.years))
  return offer && term ? formatAmount(termAmount(offer.annual_amount_minor, term.years, term.discount_basis_points), offer.currency) : '—'
})
const contractEnd = computed(() => activeOrder.value?.snapshot.ends_at || '—')
const activeCustomerName = computed(() => activeOrder.value?.customer_name || selectedCustomer.value?.name || '—')
const activePlanName = computed(() => activeOrder.value?.snapshot.plan.definition.name || selectedPlan.value?.snapshot.definition.name || '—')

const stageOptions = [
  { value: '', label: '全部进度' },
  { value: 'pending_payment', label: '待确认到账' },
  { value: 'pending_approval', label: '待导入申请' },
  { value: 'pending_issue', label: '待签发' },
  { value: 'issued', label: '已签发' },
]

function message(value: unknown, fallback: string) { return value instanceof Error ? value.message : fallback }
function stageOf(item: CommercialOrderRecord): Stage {
  if (item.status === 'pending_payment') return 'pending_payment'
  if (item.fulfillment_status === 'approved' || item.fulfillment_status === 'prepared') return 'pending_issue'
  if (item.fulfillment_status === 'issued' || item.status === 'fulfilled') return 'issued'
  return 'pending_approval'
}
function stepOf(item: CommercialOrderRecord) {
  if (item.fulfillment_status === 'prepared') return 7
  return { pending_payment: 3, pending_approval: 4, pending_issue: 6, issued: 8 }[stageOf(item)]
}
function stageLabel(item: CommercialOrderRecord) {
  return {
    pending_payment: '待确认到账', pending_approval: '待导入申请',
    pending_issue: item.fulfillment_status === 'prepared' ? '签发待恢复' : '待签发', issued: '已签发',
  }[stageOf(item)]
}

async function loadChoices() {
  choicesLoading.value = true
  choicesError.value = ''
  const [customerResult, planResult] = await Promise.allSettled([listCustomers('', 100), listCommercialPlans(100)])
  const errors: string[] = []
  if (customerResult.status === 'fulfilled') customers.value = customerResult.value.items
  else errors.push(message(customerResult.reason, '读取客户失败'))
  if (planResult.status === 'fulfilled') plans.value = planResult.value
  else errors.push(message(planResult.reason, '读取套餐失败'))
  choicesError.value = errors.join('；')
  choicesLoading.value = false
}

async function loadRecords() {
  const request = ++loadRequest
  recordsLoading.value = true
  recordsError.value = ''
  try {
    const result = await listCommercialOrders({
      limit: pageSize.value,
      offset: (page.value - 1) * pageSize.value,
      keyword: keyword.value.trim(),
      stage: stageFilter.value || 'all',
    })
    if (request !== loadRequest) return
    records.value = result.items
    total.value = result.total
  } catch (value) {
    if (request === loadRequest) recordsError.value = message(value, '读取办理记录失败')
  } finally {
    if (request === loadRequest) recordsLoading.value = false
  }
}

function resetWorkflow() {
  workspace.value = 'workflow'
  currentStep.value = 1
  maxReachedStep.value = 1
  activeOrder.value = null
  formError.value = ''
  Object.assign(form, { customerID: '', planID: '', years: '', startsAt: '' })
  sessionStorage.removeItem(draftStorageKey)
  writeLocation(1)
}

function writeLocation(step?: number, orderID?: string, method: 'push' | 'replace' = 'push') {
  const query = { ...route.query }
  delete query.order; delete query.fulfillment_order; delete query.step
  if (!orderID || orderID !== route.query.order) {
    delete query.lifecycle_kind; delete query.lifecycle_source
  }
  if (step) query.step = String(step)
  if (orderID) query.order = orderID
  if (route.query.step === query.step && route.query.order === query.order && !route.query.fulfillment_order) return
  void router[method]({ path: route.path, query })
}

function showRecords() {
  workspace.value = 'records'
  activeOrder.value = null
  writeLocation()
  void loadRecords()
}

function nextFromCustomer() {
  if (!form.customerID) { formError.value = '请选择客户'; return }
  if (!form.planID) { formError.value = '请选择套餐版本'; return }
  formError.value = ''
  currentStep.value = 2
  maxReachedStep.value = 2
  writeLocation(2)
}

async function selectStep(step: number) {
  if (step < 1 || step > maxReachedStep.value) return
  const previous = currentStep.value
  currentStep.value = step
  writeLocation(step, activeOrder.value?.snapshot.order_id)
  if (!activeOrder.value) return
  if (step === 3 && previous !== 3) {
    await nextTick()
    await payment.value?.startForOrder(activeOrder.value.snapshot.order_id)
  } else if (step >= 4 && previous < 4) {
    await nextTick()
    await loadFulfillment(activeOrder.value.snapshot.order_id)
  }
}

function readyToIssue() {
  if (!activeOrder.value || maxReachedStep.value < 6) return
  maxReachedStep.value = Math.max(maxReachedStep.value, 7)
  currentStep.value = 7
  writeLocation(7, activeOrder.value.snapshot.order_id)
}

async function loadFulfillment(orderID: string) {
  const kind = route.query.lifecycle_kind
  const sourceID = typeof route.query.lifecycle_source === 'string' ? route.query.lifecycle_source : ''
  if (sourceID && (kind === 'renewal' || kind === 'upgrade')) await fulfillment.value?.startSeeded(orderID, kind, sourceID)
  else await fulfillment.value?.startForOrder(orderID)
}

async function createOrder() {
  if (saving.value || choicesLoading.value || choicesError.value || journal.error.value) return
  formError.value = ''
  if (!journal.pending.value) {
    try { journal.prepare(orderInput(form.customerID, selectedPlan.value, Number(form.years), form.startsAt)) }
    catch (value) { formError.value = message(value, '请检查订单'); return }
  }
  saving.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    attempt = journal.begin()
    const record = await createCommercialOrder(journal.pending.value!)
    journal.clear()
    sessionStorage.removeItem(draftStorageKey)
    activeOrder.value = record
    currentStep.value = 3
    maxReachedStep.value = 3
    writeLocation(3, record.snapshot.order_id, 'replace')
    await loadRecords()
    await nextTick()
    await payment.value?.startForOrder(record.snapshot.order_id)
    toast.success('订单已创建')
  } catch (value) {
    const definitive = Boolean(attempt) && value instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(value.status) && journal.reject(attempt!)
    formError.value = `${message(value, '创建订单失败')}${definitive ? '。请修正后重试' : '。结果尚未确认，请重试原请求'}`
  } finally {
    saving.value = false
  }
}

async function paymentConfirmed() {
  currentStep.value = 4
  maxReachedStep.value = Math.max(maxReachedStep.value, 4)
  if (activeOrder.value) writeLocation(4, activeOrder.value.snapshot.order_id, 'replace')
  await loadRecords()
  await nextTick()
  if (activeOrder.value) await loadFulfillment(activeOrder.value.snapshot.order_id)
}

async function fulfillmentCompleted() {
  currentStep.value = 8
  maxReachedStep.value = 8
  if (activeOrder.value) writeLocation(8, activeOrder.value.snapshot.order_id, 'replace')
  await loadRecords()
}

function updateFulfillmentStage(step: number) {
  const previous = maxReachedStep.value
  if (step === 4 && previous === 5) {
    maxReachedStep.value = 4
    currentStep.value = 4
    if (activeOrder.value) writeLocation(4, activeOrder.value.snapshot.order_id, 'replace')
    return
  }
  maxReachedStep.value = Math.max(previous, step)
  if (step > previous && currentStep.value >= previous) {
    currentStep.value = step
    if (activeOrder.value) writeLocation(step, activeOrder.value.snapshot.order_id, 'replace')
  }
}

async function openRecord(item: CommercialOrderRecord) {
  workspace.value = 'workflow'
  activeOrder.value = item
  currentStep.value = stepOf(item)
  maxReachedStep.value = currentStep.value
  writeLocation(currentStep.value, item.snapshot.order_id)
  await nextTick()
  if (stageOf(item) === 'pending_payment') await payment.value?.startForOrder(item.snapshot.order_id)
  else await loadFulfillment(item.snapshot.order_id)
}

async function openHandoff() {
  const request = ++routeRequest
  const orderID = typeof route.query.order === 'string'
    ? route.query.order
    : typeof route.query.fulfillment_order === 'string' ? route.query.fulfillment_order : ''
  if (!orderID) {
    if (route.query.step === '1' || route.query.step === '2') {
      workspace.value = 'workflow'
      activeOrder.value = null
      currentStep.value = Number(route.query.step)
      maxReachedStep.value = currentStep.value
    } else if (journal.pending.value) {
      workspace.value = 'workflow'
      activeOrder.value = null
      currentStep.value = 2
      maxReachedStep.value = 2
    } else {
      workspace.value = 'records'
      activeOrder.value = null
    }
    return
  }
  if (workspace.value === 'workflow' && activeOrder.value?.snapshot.order_id === orderID && route.query.step === String(currentStep.value)) return
  let item: CommercialOrderRecord
  try { item = await getCommercialOrder(orderID) }
  catch (value) {
    if (request === routeRequest) { workspace.value = 'records'; recordsError.value = message(value, '读取指定订单失败') }
    return
  }
  if (request !== routeRequest) return
  workspace.value = 'workflow'
  activeOrder.value = item
  const serverStep = stepOf(item)
  const requestedStep = Number(route.query.step)
  maxReachedStep.value = serverStep === 6 && requestedStep === 7 ? 7 : serverStep
  currentStep.value = Number.isInteger(requestedStep) && requestedStep >= 1 && requestedStep <= maxReachedStep.value ? requestedStep : maxReachedStep.value
  await nextTick()
  if (currentStep.value === 3) await payment.value?.startForOrder(orderID)
  else if (currentStep.value >= 4) await loadFulfillment(orderID)
  if (request === routeRequest) writeLocation(currentStep.value, orderID, 'replace')
}

onMounted(async () => {
  await Promise.all([loadChoices(), loadRecords()])
  try {
    const saved = sessionStorage.getItem(draftStorageKey)
    if (saved) {
      const value: unknown = JSON.parse(saved)
      if (value && typeof value === 'object') {
        const draft = value as Record<string, unknown>
        for (const key of ['customerID', 'planID', 'years', 'startsAt'] as const) {
          if (typeof draft[key] === 'string') form[key] = draft[key]
        }
      }
    }
  } catch { sessionStorage.removeItem(draftStorageKey) }
  if (journal.pending.value) {
    const pending = journal.pending.value
    Object.assign(form, { customerID: pending.customer_id, planID: pending.plan_id, years: String(pending.years), startsAt: pending.starts_at.slice(0, 19) })
    workspace.value = 'workflow'
    currentStep.value = 2
    maxReachedStep.value = 2
    if (!route.query.order && !route.query.fulfillment_order) writeLocation(2, undefined, 'replace')
  }
  await openHandoff()
})
watch(() => [route.query.order, route.query.fulfillment_order, route.query.step], () => { void openHandoff() })
watch(form, value => {
  if (!activeOrder.value) sessionStorage.setItem(draftStorageKey, JSON.stringify(value))
}, { deep: true })
watch([keyword, stageFilter], () => {
  window.clearTimeout(filterTimer)
  filterTimer = window.setTimeout(() => { if (page.value === 1) void loadRecords(); else page.value = 1 }, 200)
})
watch([page, pageSize], () => void loadRecords())
onBeforeUnmount(() => window.clearTimeout(filterTimer))
</script>

<template>
  <section class="content business-workflow" :class="{ 'is-workflow': workspace === 'workflow' }">
    <header class="page-head">
      <h1 v-if="workspace === 'records'">业务办理</h1>
      <div v-else class="workflow-summary">
        <h1>本次办理</h1>
        <dl>
          <dt>客户</dt><dd>{{ activeCustomerName }}</dd>
          <dt>套餐</dt><dd>{{ activePlanName }}</dd>
          <dt>金额</dt><dd>{{ activeOrder ? formatAmount(activeOrder.snapshot.amount_minor, activeOrder.snapshot.currency) : estimate }}</dd>
          <dt>开始</dt><dd :title="activeOrder?.snapshot.starts_at || form.startsAt">{{ activeOrder?.snapshot.starts_at.slice(0, 10) || form.startsAt.slice(0, 10) || '—' }}</dd>
          <dt>结束</dt><dd :title="contractEnd">{{ contractEnd === '—' ? contractEnd : contractEnd.slice(0, 10) }}</dd>
          <dt>订单</dt><dd class="code" :title="activeOrder?.snapshot.order_id || ''">{{ activeOrder?.snapshot.order_id || '—' }}</dd>
        </dl>
      </div>
      <div class="inline-actions">
        <AButton v-if="workspace === 'workflow'" variant="secondary" @click="showRecords">办理列表</AButton>
        <AButton icon="plus" @click="resetWorkflow">新建办理</AButton>
      </div>
    </header>

    <template v-if="workspace === 'workflow'">
      <WorkflowStepper :steps="steps" :current="currentStep" :max-reached="maxReachedStep" interactive vertical @select="selectStep" />

      <div class="workflow-viewport">
        <main class="workflow-main">
          <p v-if="choicesError || formError || journal.error.value" class="workflow-error" role="alert">
            {{ choicesError || formError || journal.error.value }}
          </p>
          <ALoadingState v-if="choicesLoading && currentStep <= 2" label="正在读取基础数据" />

          <section v-else-if="currentStep === 1" class="step-panel">
            <h2>选择客户与套餐</h2>
            <template v-if="activeOrder"><dl class="selection-card"><dt>客户</dt><dd>{{ activeOrder.customer_name || activeOrder.snapshot.customer_id }}</dd><dt>套餐版本</dt><dd>{{ activeOrder.snapshot.plan.definition.name }} · v{{ activeOrder.snapshot.plan.version }}</dd></dl></template>
            <template v-else><label class="field"><span>客户</span><ASelect v-model="form.customerID" :options="customerOptions" searchable required aria-label="客户" /></label>
            <label class="field"><span>套餐版本</span><ASelect v-model="form.planID" :options="planOptions" searchable required aria-label="套餐版本" @change="form.years = ''" /></label></template>
            <div v-if="!activeOrder && selectedCustomer" class="selection-card">
              <div><strong>{{ selectedCustomer.name }}</strong><span>{{ selectedCustomer.legal_name || '—' }}</span></div>
              <dl><dt>联系人</dt><dd>{{ selectedCustomer.contact_name || '—' }}</dd><dt>邮箱</dt><dd>{{ selectedCustomer.contact_email || '—' }}</dd><dt>电话</dt><dd>{{ selectedCustomer.contact_phone || '—' }}</dd></dl>
            </div>
          </section>

          <section v-else-if="currentStep === 2" class="step-panel">
            <h2>创建订单</h2>
            <dl v-if="activeOrder" class="selection-card"><dt>订单编号</dt><dd class="code">{{ activeOrder.snapshot.order_id }}</dd><dt>订阅期限</dt><dd>{{ activeOrder.snapshot.years }} 年</dd><dt>合同金额</dt><dd>{{ formatAmount(activeOrder.snapshot.amount_minor, activeOrder.snapshot.currency) }}</dd><dt>合同开始</dt><dd>{{ activeOrder.snapshot.starts_at }}</dd><dt>合同结束</dt><dd>{{ activeOrder.snapshot.ends_at }}</dd></dl>
            <div v-else class="order-fields">
              <label class="field"><span>订阅期限</span><ASelect v-model="form.years" :options="yearOptions" required aria-label="订阅期限" /></label>
              <label class="field"><span>合同开始时间（UTC）</span><input v-model="form.startsAt" type="datetime-local" step="1" required></label>
              <div class="price-card"><span>合同金额</span><strong>{{ estimate }}</strong></div>
            </div>
          </section>

          <CommercialPaymentConfirm
            v-else-if="currentStep === 3"
            ref="payment"
            embedded
            :launcher="false"
            @confirmed="paymentConfirmed"
          />

          <CommercialFulfillment
            v-else
            ref="fulfillment"
            embedded
            :launcher="false"
            :view-step="currentStep"
            @stage-change="updateFulfillmentStage"
            @ready-to-issue="readyToIssue"
            @completed="fulfillmentCompleted"
          />
        </main>

      </div>

      <footer class="workflow-actions">
        <AButton v-if="currentStep > 1" variant="secondary" @click="selectStep(currentStep - 1)">上一步</AButton>
        <span v-else></span>
        <AButton v-if="activeOrder && currentStep < maxReachedStep" @click="selectStep(currentStep + 1)">下一步</AButton>
        <AButton v-else-if="currentStep === 1" @click="nextFromCustomer">下一步</AButton>
        <AButton v-else-if="currentStep === 2 && !activeOrder" :loading="saving" @click="createOrder">创建订单并继续</AButton>
      </footer>
    </template>

    <template v-else>
      <div class="record-toolbar">
        <label class="field"><span>搜索</span><input v-model="keyword" placeholder="客户、订单编号或客户编号"></label>
        <label class="field"><span>办理进度</span><ASelect v-model="stageFilter" :options="stageOptions" aria-label="办理进度" /></label>
      </div>
      <p v-if="recordsError" class="workflow-error" role="alert">{{ recordsError }}</p>
      <div class="table-wrap paginated-scroll">
        <ALoadingState v-if="recordsLoading && !records.length" label="正在读取办理记录" />
        <table v-else-if="records.length" class="flat-data-table">
          <thead><tr><th>客户</th><th>订单</th><th>套餐</th><th>金额</th><th>办理进度</th><th>操作</th></tr></thead>
          <tbody><tr v-for="item in records" :key="item.snapshot.order_id">
            <td><strong>{{ item.customer_name || item.snapshot.customer_id }}</strong></td>
            <td class="code">{{ item.snapshot.order_id }}</td>
            <td>{{ item.snapshot.plan.definition.name }} · v{{ item.snapshot.plan.version }}</td>
            <td>{{ formatAmount(item.snapshot.amount_minor, item.snapshot.currency) }}</td>
            <td><span class="status" :class="{ warning: stageOf(item) !== 'issued' }">{{ stageLabel(item) }}</span></td>
            <td><AButton size="small" @click="openRecord(item)">{{ stageOf(item) === 'issued' ? '查看' : '继续办理' }}</AButton></td>
          </tr></tbody>
        </table>
        <AEmpty v-else-if="!recordsError" title="暂无办理记录" />
      </div>
      <APagination v-if="total" v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="recordsLoading" />
    </template>
  </section>
</template>

<style scoped>
.business-workflow{display:flex;min-height:0;flex-direction:column}.page-head{display:flex;align-items:center;justify-content:space-between;margin-bottom:18px}.page-head h1{margin:0}.workflow-viewport{display:grid;min-height:0;flex:1;grid-template-columns:minmax(0,1fr) 290px;gap:14px;margin-top:18px}.workflow-main,.workflow-summary{min-height:0;border:1px solid var(--line);border-radius:14px;background:var(--surface)}.workflow-main{padding:22px 26px}.workflow-summary{padding:20px}.workflow-summary h3,.step-panel h2{margin:0 0 20px}.workflow-summary dl{display:grid;grid-template-columns:76px minmax(0,1fr);gap:15px 10px;margin:0}.workflow-summary dt{color:var(--muted);font-size:var(--font-size-body)}.workflow-summary dd{min-width:0;margin:0;overflow-wrap:anywhere;font-size:var(--font-size-body)}.step-panel{display:grid;align-content:start;gap:18px;max-width:760px}.step-panel>.field{max-width:520px}.selection-card{display:grid;gap:16px;border:1px solid var(--line);border-radius:12px;padding:18px}.selection-card>div{display:grid;gap:5px}.selection-card>div span{color:var(--muted);font-size:var(--font-size-body)}.selection-card dl{display:grid;grid-template-columns:70px minmax(0,1fr);gap:10px;margin:0}.selection-card dt{color:var(--muted)}.selection-card dd{margin:0}.order-fields{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:18px}.price-card{display:flex;align-items:center;justify-content:space-between;border:1px solid var(--line);border-radius:11px;padding:12px 15px}.price-card span{color:var(--muted);font-size:var(--font-size-body)}.price-card strong{font-size:var(--font-size-title)}.workflow-actions{display:flex;align-items:center;justify-content:space-between;padding-top:14px}.workflow-error{margin:0 0 14px;border:1px solid var(--danger);border-radius:10px;padding:11px 13px;color:var(--danger)}.record-toolbar{display:grid;grid-template-columns:minmax(280px,1fr) 240px;gap:14px;margin-bottom:14px}.status{display:inline-flex;border-radius:999px;padding:4px 9px;background:var(--success-soft);color:var(--success);font-size:var(--font-size-body)}.status.warning{background:var(--warning-soft);color:var(--warning)}@media(max-width:900px){.workflow-viewport{grid-template-columns:1fr}.workflow-summary{display:none}.order-fields,.record-toolbar{grid-template-columns:1fr}}
.business-workflow.is-workflow{display:grid;grid-template-columns:230px minmax(0,1fr);grid-template-rows:auto minmax(0,1fr) auto;gap:0 18px;align-content:stretch}
.business-workflow.is-workflow>.page-head{grid-column:1/-1}
.business-workflow.is-workflow>.workflow-stepper{grid-column:1;grid-row:2/4;align-self:stretch}
.business-workflow.is-workflow>.workflow-viewport{grid-column:2;grid-row:2;display:flex;flex-direction:column;gap:14px;margin:0}
.business-workflow.is-workflow>.workflow-actions{grid-column:2;grid-row:3}
.business-workflow .step-panel>dl.selection-card{grid-template-columns:120px minmax(0,1fr);max-width:900px;gap:12px 20px;margin:0}
.business-workflow .step-panel>dl.selection-card dt{color:var(--muted)}
.business-workflow .step-panel>dl.selection-card dd{min-width:0;margin:0;overflow-wrap:anywhere}
.business-workflow.is-workflow{grid-template-columns:190px minmax(0,1fr);grid-template-rows:auto minmax(0,1fr) auto;gap:0 14px}
.business-workflow.is-workflow>.page-head{min-width:0;align-items:center;gap:16px;margin-bottom:12px}
.business-workflow.is-workflow>.page-head>.inline-actions{flex:0 0 auto}
.business-workflow.is-workflow .workflow-summary{display:grid;grid-template-columns:max-content minmax(0,1fr);align-items:center;min-width:0;flex:1;gap:18px;padding:0;border:0;border-radius:0;background:none}
.business-workflow.is-workflow .workflow-summary h1{margin:0;font-size:var(--font-size-title);white-space:nowrap}
.business-workflow.is-workflow .workflow-summary dl{display:grid;grid-template-columns:repeat(3,max-content minmax(0,1fr));align-items:center;min-width:0;gap:5px 9px;margin:0}
.business-workflow.is-workflow .workflow-summary dt,.business-workflow.is-workflow .workflow-summary dd{min-width:0;font-size:var(--font-size-body);line-height:1.45}
.business-workflow.is-workflow .workflow-summary dd{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.business-workflow.is-workflow>.workflow-stepper{min-height:0}
.business-workflow.is-workflow>.workflow-viewport{min-height:0}
.business-workflow.is-workflow .workflow-main{flex:1 1 auto;min-height:0;overflow:auto;padding:18px 20px}
.business-workflow.is-workflow>.workflow-actions{min-height:40px;padding-top:8px}
@media(max-width:1100px){.business-workflow.is-workflow{grid-template-columns:1fr;grid-template-rows:auto auto minmax(0,1fr) auto}.business-workflow.is-workflow>.workflow-stepper{grid-column:1;grid-row:2}.business-workflow.is-workflow>.workflow-viewport{grid-column:1;grid-row:3;margin-top:12px}.business-workflow.is-workflow>.workflow-actions{grid-column:1;grid-row:4}.business-workflow.is-workflow .workflow-summary dl{grid-template-columns:repeat(2,max-content minmax(0,1fr))}}
@media(max-width:760px){.business-workflow.is-workflow>.page-head{align-items:flex-start;flex-direction:column}.business-workflow.is-workflow .workflow-summary{grid-template-columns:1fr;gap:6px;width:100%}.business-workflow.is-workflow .workflow-summary dl{grid-template-columns:repeat(2,max-content minmax(0,1fr))}}
</style>
