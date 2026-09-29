<script setup lang="ts">
import { computed, ref } from 'vue'
import { AButton, ALoadingState, AModal, ASelect } from '@aster/ui'
import { createQuotationOrder, getCurrentOperationsOperatorID, listCustomers, getQuotationSource, OperationsAPIError, type CommercialOrderRecord, type CreateQuotationOrderInput, type Customer, type QuotationSource } from '../api/client'
import { submissionJournal } from '../commercial/submission-journal'
import { formatAmount, termAmount } from '../commercial/plan-form'
import CommercialPlanSummary from './CommercialPlanSummary.vue'

const emit = defineEmits<{ created: [record: CommercialOrderRecord] }>()
const journal = submissionJournal<CreateQuotationOrderInput>('quotation-order', getCurrentOperationsOperatorID())
const pending = journal.pending; const journalError = journal.error
const open = ref(false); const busy = ref(false); const loading = ref(false); const error = ref(''); const choicesError = ref('')
const publication = ref<QuotationSource | null>(null); const reference = ref(''); const customers = ref<Customer[]>([]); const next = ref('')
const customerID = ref(''); const planID = ref(''); const years = ref(''); const startsAt = ref('')
const plan = computed(() => publication.value?.plans.find(p => p.plan_id === planID.value))
const offer = computed(() => { const o = plan.value?.definition.offer; return o?.kind === 'annual' ? o : undefined })
const customerOptions = computed(() => customers.value.filter(c => c.status !== 'inactive').map(c => ({ value: c.id, label: c.name, description: c.legal_name || c.id })))
const planOptions = computed(() => publication.value?.plans.filter(p => p.definition.offer.kind === 'annual').map(p => ({ value: p.plan_id, label: `${p.definition.name} · v${p.version}`, description: p.definition.code })) ?? [])
const yearOptions = computed(() => offer.value?.terms.map(t => ({ value: String(t.years), label: `${t.years} 年` })) ?? [])
const amount = computed(() => { const o = offer.value; const term = o?.terms.find(t => t.years === Number(years.value)); return o && term ? formatAmount(termAmount(o.annual_amount_minor, term.years, term.discount_basis_points), o.currency) : '' })
function message(e: unknown) { return e instanceof Error ? e.message : '读取失败' }
async function load() {
  if (loading.value || pending.value) return
  loading.value = true; choicesError.value = ''
  try { const page = await listCustomers('', 100); customers.value = page.items; next.value = page.next }
  catch (e) { customers.value = []; choicesError.value = message(e) }
  finally { loading.value = false }
}
function resetSource() { publication.value = null; planID.value = ''; years.value = '' }
async function loadSource() {
  if (loading.value || busy.value || pending.value) return
  resetSource(); loading.value = true; error.value = ''
  try { publication.value = await getQuotationSource(reference.value.trim()) }
  catch (e) { error.value = message(e) }
  finally { loading.value = false }
}
async function more() {
  if (loading.value || !next.value) return
  loading.value = true
  try { const page = await listCustomers(next.value, 100); customers.value.push(...page.items); next.value = page.next }
  catch (e) { choicesError.value = message(e) }
  finally { loading.value = false }
}
function start() { open.value = true; if (!pending.value) void load() }
async function save() {
  if (busy.value || loading.value || journalError.value || (!pending.value && choicesError.value)) return
  error.value = ''
  if (!pending.value) {
    const p = publication.value; const selected = plan.value
    if (!p || !selected || !customerID.value || !offer.value?.terms.some(t => t.years === Number(years.value)) || !startsAt.value) { error.value = '请完整选择报价来源、客户及合同期限'; return }
    try {
      journal.prepare({ operation_id: crypto.randomUUID(), customer_id: customerID.value, publication_id: p.publication_id, catalog_revision: p.catalog_revision, plan_id: selected.plan_id, plan_version: selected.version, years: Number(years.value), starts_at: new Date(`${startsAt.value}Z`).toISOString() })
    } catch (e) { error.value = message(e); return }
  }
  busy.value = true
  let attempt: ReturnType<typeof journal.begin> | undefined
  try {
    attempt = journal.begin()
    const record = await createQuotationOrder(pending.value!)
    journal.clear(); open.value = false; emit('created', record)
  } catch (e) {
    const rejected = !!attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(e.status) && journal.reject(attempt)
    error.value = `${message(e)}。${rejected ? '输入已保留，请核对后重试' : '结果尚未确认，请重试原请求'}`
  } finally { busy.value = false }
}
</script>

<template>
  <AButton variant="secondary" @click="start">{{ pending ? '继续报价订单' : '按官网报价建单' }}</AButton>
  <AModal :open="open" title="按官网报价建单" description="选择客户咨询时的公开目录。服务端核对发布记录、套餐版本及受理期限后保存订单。" :close-disabled="busy" @close="open = false">
    <form class="form" @submit.prevent="save">
      <p v-if="journalError || error" class="quote-error" role="alert">{{ journalError || error }}</p>
      <p v-if="choicesError" class="quote-error" role="alert">{{ choicesError }} <AButton variant="secondary" :disabled="loading" @click="load">重新读取</AButton></p>
      <ALoadingState v-if="loading" label="正在读取报价来源和客户" />
      <dl v-if="pending" class="quote-info"><dt>客户</dt><dd>{{ pending.customer_id }}</dd><dt>发布记录</dt><dd>{{ pending.publication_id }}</dd><dt>公开目录</dt><dd>{{ pending.catalog_revision }}</dd><dt>套餐</dt><dd>{{ pending.plan_id }} v{{ pending.plan_version }}</dd><dt>期限</dt><dd>{{ pending.years }} 年</dd><dt>合同开始</dt><dd>{{ pending.starts_at }}</dd></dl>
      <fieldset v-else class="quote-fields" :disabled="busy || loading">
        <label class="field"><span>客户</span><ASelect v-model="customerID" aria-label="报价客户" :options="customerOptions" required searchable /></label>
        <AButton v-if="next" variant="secondary" @click="more">读取更多客户</AButton>
        <label class="field"><span>客户咨询中的目录编号</span><input v-model="reference" aria-label="报价来源编号" placeholder="目录编号或发布记录编号" required @input="resetSource"></label>
        <AButton variant="secondary" :disabled="loading || !reference.trim()" @click="loadSource">读取报价来源</AButton>
        <p class="quote-note">按完整编号查询历史报价，受理环境由运营服务器配置</p>
        <p v-if="publication" class="quote-note">{{ publication.environment === 'local' ? '本地验证' : '生产官网' }}<br>受理截止 {{ publication.accept_until }}<br>发布记录 {{ publication.publication_id }}</p>
        <label class="field"><span>公开套餐版本</span><ASelect v-model="planID" aria-label="报价套餐版本" :options="planOptions" required @change="years = ''" /></label>
        <label class="field"><span>订阅期限</span><ASelect v-model="years" aria-label="报价订阅期限" :options="yearOptions" required /></label>
        <label class="field"><span>合同开始时间（UTC）</span><input v-model="startsAt" type="datetime-local" step="1" aria-label="报价合同开始时间（UTC）" required><small>北京时间减去 8 小时，到期时间按固定套餐的合同日历计算</small></label>
      </fieldset>
      <p v-if="amount && !pending" class="quote-amount">应付金额 {{ amount }}</p>
      <details v-if="plan && !pending"><summary>核对报价权益</summary><CommercialPlanSummary :definition="plan.definition" /></details>
      <div class="form-actions"><AButton variant="secondary" :disabled="busy" @click="open = false">返回</AButton><AButton type="submit" :loading="busy" :disabled="!!journalError || loading || (!pending && !!choicesError)">{{ pending ? '重试原报价请求' : '保存报价订单' }}</AButton></div>
    </form>
  </AModal>
</template>

<style scoped>
.quote-fields { display: grid; gap: 16px; border: 0; padding: 0; margin: 0; min-width: 0; }
.quote-error { padding: 12px; border: 1px solid var(--line); border-radius: 10px; font-size:var(--font-size-body); }
.quote-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; overflow-wrap: anywhere; }
.quote-amount { font-size:var(--font-size-title); font-weight: 650; }
.quote-info { display: grid; grid-template-columns: 90px minmax(0, 1fr); gap: 12px; font-size:var(--font-size-body); }
.quote-info dt { color: var(--muted); }
.quote-info dd { margin: 0; overflow-wrap: anywhere; }
</style>
