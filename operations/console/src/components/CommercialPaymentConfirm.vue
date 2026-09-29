<script setup lang="ts">
import { computed, ref } from 'vue'
import { AButton, ALoadingState, AModal } from '@aster/ui'
import { confirmCommercialPayment, getCommercialPayment, getCommercialPaymentContext, getCurrentOperationsOperatorID, OperationsAPIError, type CommercialPaymentContext, type CommercialPaymentRecord, type ConfirmCommercialPaymentInput } from '../api/client'
import { submissionJournal } from '../commercial/submission-journal'
import { formatAmount } from '../commercial/plan-form'
import ReauthActionModal from './ReauthActionModal.vue'

type Submission = ConfirmCommercialPaymentInput & { order_id: string; context: CommercialPaymentContext }
const props = withDefaults(defineProps<{ launcher?: boolean; embedded?: boolean }>(), { launcher: true, embedded: false })
const emit = defineEmits<{ confirmed: [] }>()
const journal = submissionJournal<Submission>('payment', getCurrentOperationsOperatorID())
const pending = journal.pending; const journalError = journal.error
const open = ref(false); const confirmOpen = ref(false); const busy = ref(false); const loading = ref(false); const error = ref('')
const orderID = ref(''); const reference = ref(''); const receivedAt = ref(''); const notes = ref(''); const checked = ref(false)
const context = ref<CommercialPaymentContext | null>(null); const receipt = ref<CommercialPaymentRecord | null>(null)
const shown = computed(() => pending.value?.context ?? context.value)
const surfaceProps = computed(() => props.embedded ? {} : {
  open: open.value,
  title: '核对订单到账',
  description: '根据原订单核对全额到账，保存凭据和确认记录。此操作不发起扣款，也不代表已经签发授权。',
  closeDisabled: busy.value || loading.value,
})
const canSubmit = computed(() => !!pending.value || (context.value?.status === 'pending_payment' && checked.value))
function message(e: unknown) { return e instanceof Error ? e.message : '操作失败' }
function reset() { context.value = null; receipt.value = null; checked.value = false }
function start() { open.value = true; error.value = ''; if (!pending.value) { reset(); receivedAt.value = new Date().toISOString().slice(0, 19) } }
async function startForOrder(id: string) {
  start()
  if (pending.value) {
    error.value = '请先处理浏览器中待确认的到账操作'
    return
  }
  orderID.value = id
  await load()
}
defineExpose({ startForOrder })
function close() { if (busy.value || loading.value) return; open.value = false; confirmOpen.value = false }
async function load() {
  if (busy.value || loading.value || pending.value || !orderID.value.trim()) return
  reset(); loading.value = true; error.value = ''
  try {
    context.value = await getCommercialPaymentContext(orderID.value.trim())
    if (context.value.status !== 'pending_payment') receipt.value = await getCommercialPayment(context.value.order_id)
  } catch (e) { error.value = message(e) }
  finally { loading.value = false }
}
function promptConfirmation() {
  if (busy.value || loading.value || journalError.value || !canSubmit.value) return
  error.value = ''
  if (!pending.value && (!reference.value.trim() || !receivedAt.value)) { error.value = '请填写到账凭据和实际到账时间'; return }
  confirmOpen.value = true
}
async function save(password: string) {
  if (busy.value || loading.value || journalError.value || !canSubmit.value || !password) return
  error.value = ''
  if (!pending.value) {
    const original = context.value
    if (!original || !reference.value.trim() || !receivedAt.value) { error.value = '请填写到账凭据和实际到账时间'; return }
    try { journal.prepare({ operation_id: crypto.randomUUID(), order_id: original.order_id, expected_order_sha256: original.order_sha256, payment_reference: reference.value.trim(), received_at: new Date(`${receivedAt.value}Z`).toISOString(), notes: notes.value.trim(), context: original }) }
    catch (e) { error.value = message(e); return }
  }
  busy.value = true
  let attempt: ReturnType<typeof journal.begin> | undefined
  try {
    attempt = journal.begin()
    const saved = pending.value!
    const input: ConfirmCommercialPaymentInput = { operation_id: saved.operation_id, expected_order_sha256: saved.expected_order_sha256, payment_reference: saved.payment_reference, received_at: saved.received_at, notes: saved.notes }
    const result = await confirmCommercialPayment(saved.order_id, input, password)
    orderID.value = saved.order_id
    context.value = { ...saved.context, status: 'fulfillment_pending' }
    journal.clear(); receipt.value = result; checked.value = false; confirmOpen.value = false; emit('confirmed')
  } catch (e) {
    const rejected = !!attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(e.status) && journal.reject(attempt)
    error.value = `${message(e)}。${rejected ? '本次请求被拒绝，请重新读取订单核对' : '结果尚未确认，请保留原请求并重试'}`
  } finally { busy.value = false }
}
</script>

<template>
  <AButton v-if="props.launcher" variant="secondary" @click="start">{{ pending ? '继续确认到账' : '核对订单到账' }}</AButton>
  <component :is="props.embedded ? 'section' : AModal" v-bind="surfaceProps" :class="{ 'payment-workspace': props.embedded }" @close="close">
    <form class="form" @submit.prevent="promptConfirmation">
      <p v-if="journalError || error" class="payment-error" role="alert">{{ journalError || error }}</p>
      <fieldset v-if="!props.embedded && !pending && !receipt" class="payment-fields payment-lookup" :disabled="busy || loading">
        <label class="field"><span>订单编号</span><input v-model="orderID" aria-label="到账订单编号" required @input="reset"></label>
        <AButton variant="secondary" :disabled="!orderID.trim()" @click="load">读取到账订单</AButton>
      </fieldset>
      <AButton v-if="props.embedded && error && !shown && !loading && orderID" variant="secondary" @click="load">重新读取订单</AButton>
      <ALoadingState v-if="loading" label="正在读取原订单" />
      <p v-if="receipt" class="payment-success" role="status">已记录全额到账</p>
      <div v-if="shown" class="payment-overview">
        <div><span>应收全额</span><strong>{{ formatAmount(shown.amount_minor, shown.currency) }}</strong></div>
        <div><span>订单来源</span><strong>{{ shown.source === 'manual' ? '手工固定套餐' : shown.source === 'local' ? '本地验证' : '生产官网' }}</strong></div>
      </div>
      <details v-if="shown" class="payment-order-details" :open="!props.embedded">
        <summary>订单核对信息</summary>
        <dl class="payment-info"><dt>订单</dt><dd>{{ shown.order_id }}</dd><dt>客户</dt><dd>{{ shown.customer_id }}</dd><dt>合同开始</dt><dd>{{ shown.starts_at }}</dd><dt>合同结束</dt><dd>{{ shown.ends_at }}</dd><dt>原订单摘要</dt><dd>{{ shown.order_sha256 }}</dd></dl>
      </details>
      <template v-if="receipt">
        <dl class="payment-info payment-receipt"><dt>到账凭据</dt><dd>{{ receipt.snapshot.request.payment_reference }}</dd><dt>实际到账</dt><dd>{{ receipt.snapshot.request.received_at }}</dd><dt>确认时间</dt><dd>{{ receipt.snapshot.confirmed_at }}</dd><dt>备注</dt><dd>{{ receipt.snapshot.request.notes || '无' }}</dd><dt>记录摘要</dt><dd>{{ receipt.sha256 }}</dd></dl>
      </template>
      <template v-else-if="pending || context?.status === 'pending_payment'">
        <dl v-if="pending" class="payment-info"><dt>原到账凭据</dt><dd>{{ pending.payment_reference }}</dd><dt>实际到账</dt><dd>{{ pending.received_at }}</dd><dt>原备注</dt><dd>{{ pending.notes || '无' }}</dd></dl>
        <fieldset v-else class="payment-fields payment-confirm-fields" :disabled="busy || loading">
          <label class="field"><span>到账凭据</span><input v-model="reference" aria-label="到账凭据" maxlength="256" required placeholder="银行流水号或可核对的收款记录编号"></label>
          <label class="field"><span>实际到账时间（UTC）</span><input v-model="receivedAt" aria-label="实际到账时间（UTC）" type="datetime-local" step="1" required></label>
          <label class="field"><span>到账备注</span><textarea v-model="notes" aria-label="到账备注" maxlength="2000" rows="3" /></label>
          <label class="payment-check"><input v-model="checked" type="checkbox" required><span>已核对该订单币种和应收全额均已到账</span></label>
          <AButton type="submit" :loading="busy" :disabled="!canSubmit || !!journalError || loading">确认全额到账</AButton>
        </fieldset>
        <AButton v-if="pending" type="submit" :loading="busy" :disabled="!!journalError || loading">重试原到账确认</AButton>
      </template>
      <AButton v-if="!props.embedded" variant="secondary" :disabled="busy || loading" @click="close">关闭</AButton>
    </form>
  </component>
  <ReauthActionModal :open="confirmOpen" title="确认全额到账" confirm-label="确认到账" :busy="busy" :error="error" @close="confirmOpen = false" @submit="save" />
</template>

<style scoped>
.payment-fields { display: grid; gap: 16px; min-width: 0; border: 0; margin: 0; padding: 0; }
.payment-info { display: grid; grid-template-columns: 96px minmax(0, 1fr); gap: 12px; font-size:var(--font-size-body); margin: 20px 0; }
.payment-info dt, .payment-note { color: var(--muted); }
.payment-info dd { margin: 0; overflow-wrap: anywhere; }
.payment-note, .payment-check { font-size:var(--font-size-body); line-height: 1.7; }
.payment-check { display: flex; gap: 10px; align-items: flex-start; }
.payment-check input { flex: 0 0 auto; width: 16px; height: 16px; margin-top: 3px; }
.payment-error, .payment-success { border: 1px solid var(--line); border-radius: 10px; padding: 12px; font-size:var(--font-size-body); line-height: 1.6; }
.payment-workspace { min-width:0; }
.payment-lookup { grid-template-columns:minmax(0,1fr) max-content;align-items:end; }
.payment-lookup .a-button { align-self:end; }
.payment-overview { display:flex;gap:48px;align-items:center;padding:18px 22px;border:1px solid var(--line);border-radius:12px;background:var(--surface-2); }
.payment-overview > div { display:grid;gap:5px; }
.payment-overview span { color:var(--muted); }
.payment-overview strong { font-size:var(--font-size-title); }
.payment-order-details summary { cursor:pointer;color:var(--text-soft);font-weight:650; }
.payment-workspace .form { grid-template-columns:repeat(2,minmax(0,1fr));align-content:start;align-items:end;gap:20px 24px; }
.payment-workspace .payment-error,.payment-workspace .payment-success,.payment-workspace .payment-overview,.payment-workspace .payment-order-details,.payment-workspace .payment-info,.payment-workspace .payment-fields { grid-column:1/-1; }
.payment-workspace .payment-confirm-fields { grid-template-columns:repeat(2,minmax(0,1fr));gap:20px 24px; }
.payment-workspace .payment-confirm-fields > .field:nth-child(3) { grid-column:1/-1; }
.payment-workspace .payment-confirm-fields > .a-button { justify-self:end;align-self:center; }
.payment-workspace .payment-confirm-fields textarea { min-height:72px;resize:vertical; }
.payment-workspace .payment-info { margin:8px 0; }
.payment-workspace .payment-order-details .payment-info { margin-top:14px; }
.payment-workspace .form > .a-button { justify-self:start; }
@media(max-width:900px){.payment-workspace .form,.payment-workspace .payment-confirm-fields{grid-template-columns:1fr}.payment-workspace .payment-confirm-fields > .field:nth-child(3){grid-column:1}.payment-workspace .payment-confirm-fields > .a-button{justify-self:start}}
</style>
