<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { AButton, ACopyCode, AFilePicker, ALoadingState, AModal, ASelect, useToast } from '@aster/ui'
import {
  approvePaidFulfillment,
  downloadPaidFulfillment,
  getCurrentOperationsOperatorID,
  getPaidFulfillment,
  getPaidFulfillmentContext,
  getPaidFulfillmentForOrder,
  getPaidLifecycleSource,
  issuePaidFulfillment,
  listV2IssuerProfiles,
  OperationsAPIError,
  recordPaidRedelivery,
  type ApprovePaidFulfillmentInput,
  type PaidFulfillmentContext,
  type PaidFulfillmentRecord,
  type PaidLifecycleSource,
  type RecordPaidRedeliveryInput,
  type V2IssuerProfile,
} from '../api/client'
import { readPaidLicenseRequest } from '../commercial/paid-fulfillment'
import { formatAmount, termAmount } from '../commercial/plan-form'
import { submissionJournal } from '../commercial/submission-journal'
import CommercialPlanSummary from './CommercialPlanSummary.vue'
import PaidTransfer from './PaidTransfer.vue'
import ReauthActionModal from './ReauthActionModal.vue'

type ApprovalSubmission = ApprovePaidFulfillmentInput & { order_id: string; context: PaidFulfillmentContext }
type IssueSubmission = { operation_id: string; fulfillment_id: string; key_id: string }
type RedeliverySubmission = RecordPaidRedeliveryInput & { fulfillment_id: string }

const props = withDefaults(defineProps<{ launcher?: boolean; embedded?: boolean; viewStep?: number }>(), { launcher: true, embedded: false, viewStep: 0 })
const emit = defineEmits<{ completed: []; 'stage-change': [step: number]; 'ready-to-issue': [] }>()
const toast = useToast()
const owner = getCurrentOperationsOperatorID()
const approvalJournal = submissionJournal<ApprovalSubmission>('fulfillment', owner)
const issueJournal = submissionJournal<IssueSubmission>('fulfillment-issue', owner)
const redeliveryJournal = submissionJournal<RedeliverySubmission>('fulfillment-redelivery', owner)
const pendingApproval = approvalJournal.pending
const pendingIssue = issueJournal.pending
const pendingRedelivery = redeliveryJournal.pending
function object(value: unknown): value is Record<string, unknown> { return !!value && typeof value === 'object' && !Array.isArray(value) }
function integerBetween(value: unknown, minimum: number, maximum: number) { return Number.isSafeInteger(value) && Number(value) >= minimum && Number(value) <= maximum }
function validPlanDefinition(value: unknown) {
  if (!object(value) || typeof value.name !== 'string' || typeof value.edition !== 'string' || typeof value.minimum_version !== 'string'
    || typeof value.support_terms_version !== 'string' || !integerBetween(value.transfer_limit, 0, 10000) || !object(value.entitlements) || !object(value.offer)) return false
  if (!Array.isArray(value.entitlements.features) || !value.entitlements.features.every(item => typeof item === 'string')
    || !Array.isArray(value.entitlements.quotas) || !value.entitlements.quotas.every(item => object(item) && typeof item.id === 'string' && object(item.limit)
      && (item.limit.mode === 'unlimited' || item.limit.mode === 'limited' && integerBetween(item.limit.value, 0, 4294967295)))) return false
  const offer = value.offer
  if (offer.kind === 'annual') {
    if (!integerBetween(offer.annual_amount_minor, 1, 9000000000000) || typeof offer.currency !== 'string'
      || typeof offer.tax_mode !== 'string' || typeof offer.term_timezone !== 'string' || !Array.isArray(offer.terms) || !offer.terms.length
      || !offer.terms.every(term => object(term) && integerBetween(term.years, 1, 5) && integerBetween(term.discount_basis_points, 1, 10000))) return false
    try { offer.terms.forEach(term => { if (!object(term)) throw new Error('invalid term'); termAmount(Number(offer.annual_amount_minor), Number(term.years), Number(term.discount_basis_points)) }) }
    catch { return false }
    return true
  }
  if (offer.kind === 'free') return object(offer.expiry) && (offer.expiry.mode === 'none' || offer.expiry.mode === 'fixed' && typeof offer.expiry.expires_at === 'string')
  return offer.kind === 'contact'
}
function validStoredApproval(value: ApprovalSubmission | null) {
  if (!value) return true
  const source: unknown = value.context
  return object(source) && value.order_id === source.order_id && typeof value.expected_order_sha256 === 'string'
    && typeof value.expected_payment_sha256 === 'string' && typeof value.license_request_json === 'string' && typeof value.reason === 'string'
    && typeof source.customer_id === 'string' && typeof source.order_sha256 === 'string' && typeof source.payment_id === 'string'
    && typeof source.payment_sha256 === 'string' && integerBetween(source.amount_minor, 1, 9000000000000) && typeof source.currency === 'string'
    && typeof source.starts_at === 'string' && typeof source.ends_at === 'string' && typeof source.status === 'string'
    && typeof source.source === 'string' && typeof source.environment === 'string' && object(source.plan)
    && integerBetween(source.plan.version, 1, 4294967295) && validPlanDefinition(source.plan.definition)
    && (!value.lifecycle || object(value.lifecycle) && ['renewal', 'upgrade'].includes(String(value.lifecycle.kind))
      && typeof value.lifecycle.source_id === 'string' && /^[a-f0-9]{64}$/.test(String(value.lifecycle.expected_document_sha256)))
}
function validStoredIssue(value: IssueSubmission | null) {
  return !value || typeof value.fulfillment_id === 'string' && !!value.fulfillment_id && typeof value.key_id === 'string' && !!value.key_id
}
function validStoredRedelivery(value: RedeliverySubmission | null) {
  return !value || typeof value.fulfillment_id === 'string' && !!value.fulfillment_id && /^[a-f0-9]{64}$/.test(value.expected_document_sha256) && typeof value.reason === 'string' && !!value.reason.trim()
}
const storedJournalError = computed(() => !validStoredApproval(pendingApproval.value) || !validStoredIssue(pendingIssue.value) || !validStoredRedelivery(pendingRedelivery.value) ? '待确认付费交付记录损坏，请保留浏览器记录并人工核对原操作' : '')
const journalError = computed(() => approvalJournal.error.value || issueJournal.error.value || redeliveryJournal.error.value || storedJournalError.value)

const open = ref(false)
const loading = ref(false)
const approving = ref(false)
const issuing = ref(false)
const downloading = ref(false)
const redelivering = ref(false)
const redeliveryOpen = ref(false)
const error = ref('')
const orderID = ref('')
const context = ref<PaidFulfillmentContext | null>(null)
const record = ref<PaidFulfillmentRecord | null>(null)
const requestFile = ref<File | null>(null)
const requestText = ref('')
const requestReading = ref(false)
let requestReadGeneration = 0
let activeRequestFile: File | null = null
const reason = ref('')
const checked = ref(false)
const approvalPassword = ref('')
const issuePassword = ref('')
const redeliveryReason = ref('')
const redeliveryPassword = ref('')
const confirmationAction = ref<'approval' | 'issue' | 'redelivery' | null>(null)
const confirmationTitle = computed(() => ({ approval: '确认批准交付', issue: '确认签发授权', redelivery: '确认补发授权' })[confirmationAction.value || 'approval'])
const confirmationBusy = computed(() => approving.value || issuing.value || redelivering.value)
const redeliveryChecked = ref(false)
const profiles = ref<V2IssuerProfile[]>([])
const profilesLoading = ref(false)
const profilesError = ref('')
const selectedKey = ref('')
const lifecycleKind = ref<'initial' | 'renewal' | 'upgrade'>('initial')
const lifecycleSourceID = ref('')
const lifecycleSource = ref<PaidLifecycleSource | null>(null)
const lifecycleLoading = ref(false)
const lifecycleOptions = [
  { value: 'initial', label: '首次购买', description: '为新安装建立第一份付费许可证' },
  { value: 'renewal', label: '续费', description: '新合同从当前许可证到期点连续生效' },
  { value: 'upgrade', label: '升级或扩容', description: '新套餐在当前许可证到期前替换生效' },
]

const shownContext = computed(() => validStoredApproval(pendingApproval.value) ? pendingApproval.value?.context ?? context.value : context.value)
const planDefinition = computed(() => record.value?.snapshot.payment.snapshot.order.plan.definition ?? shownContext.value?.plan.definition)
const recordLifecycle = computed(() => {
  const snapshot = record.value?.snapshot
  return snapshot && 'lifecycle' in snapshot ? snapshot.lifecycle : undefined
})
const issueConflict = computed(() => {
  const pending = pendingIssue.value
  const current = record.value
  return pending && current?.snapshot.id === pending.fulfillment_id && current.claims?.key_id && current.claims.key_id !== pending.key_id
    ? { local: pending.key_id, server: current.claims.key_id }
    : null
})
const keyOptions = computed(() => profiles.value
  .filter(value => value.policy.sources.includes('commercial_order') && value.policy.bindings.includes('installation') && value.policy.expiries.includes('fixed'))
  .map(value => ({ value: value.key_id, label: value.key_id })))
const actionLabel = computed(() => pendingApproval.value ? '继续批准交付' : pendingIssue.value ? '继续签发交付' : pendingRedelivery.value ? '继续补发授权' : '批准与签发')
const flowStep = computed(() => !record.value ? 1 : record.value.status === 'issued' ? 3 : 2)
const workflowNode = computed(() => !record.value
  ? requestText.value ? 5 : 4
  : record.value.status === 'issued' ? 8
    : record.value.status === 'prepared' ? 7 : 6)
const surfaceProps = computed(() => props.embedded ? {} : {
  open: open.value,
  title: '付费授权交付',
  description: '依据原订单与到账记录批准目标安装，冻结后签发并核对原授权文件。',
  closeDisabled: loading.value || approving.value || issuing.value || downloading.value || redelivering.value,
})
watch(workflowNode, value => emit('stage-change', value), { immediate: true })
const statusName = (value: string) => ({ approved: '已批准', prepared: '待完成签发', issued: '已签发' }[value] ?? value)
const message = (value: unknown, fallback: string) => value instanceof Error ? value.message : fallback

function clearTransient() {
  error.value = ''
  confirmationAction.value = null
  approvalPassword.value = ''
  issuePassword.value = ''
  redeliveryPassword.value = ''
  profilesError.value = ''
}
function resetLookup() {
  requestReadGeneration++
  activeRequestFile = null
  requestReading.value = false
  context.value = null
  record.value = null
  requestFile.value = null
  requestText.value = ''
  reason.value = ''
  redeliveryReason.value = ''
  redeliveryChecked.value = false
  redeliveryOpen.value = false
  checked.value = false
  profiles.value = []
  selectedKey.value = ''
  lifecycleKind.value = 'initial'
  lifecycleSourceID.value = ''
  lifecycleSource.value = null
}
function start() {
  open.value = true
  clearTransient()
  if (pendingApproval.value && validStoredApproval(pendingApproval.value)) {
    orderID.value = pendingApproval.value.order_id
    context.value = pendingApproval.value.context
    requestText.value = pendingApproval.value.license_request_json
    reason.value = pendingApproval.value.reason
    if (pendingApproval.value.lifecycle) {
      lifecycleKind.value = pendingApproval.value.lifecycle.kind
      lifecycleSourceID.value = pendingApproval.value.lifecycle.source_id
    }
  } else if (pendingIssue.value && validStoredIssue(pendingIssue.value)) {
    orderID.value = ''
    resetLookup()
    selectedKey.value = pendingIssue.value.key_id
    void recoverIssue()
  } else if (pendingRedelivery.value && validStoredRedelivery(pendingRedelivery.value)) {
    orderID.value = ''
    resetLookup()
    redeliveryReason.value = pendingRedelivery.value.reason
    redeliveryOpen.value = true
    void recoverRedelivery()
  } else {
    orderID.value = ''
    resetLookup()
  }
}
async function startSeeded(order: string, kind: 'renewal' | 'upgrade', sourceID: string) {
  start()
  if (pendingApproval.value || pendingIssue.value || pendingRedelivery.value) {
    error.value = '请先处理浏览器中待确认的交付操作，再开始新的许可证生命周期'
    return
  }
  orderID.value = order
  await loadOrder()
  if (!context.value || record.value) return
  lifecycleKind.value = kind
  lifecycleSourceID.value = sourceID
  await loadLifecycle()
}
async function startForOrder(order: string) {
  start()
  if (pendingApproval.value || pendingIssue.value || pendingRedelivery.value) {
    error.value = '请先处理浏览器中待确认的交付操作'
    return
  }
  orderID.value = order
  await loadOrder()
}
defineExpose({ startSeeded, startForOrder })
function close() {
  if (loading.value || approving.value || issuing.value || downloading.value || redelivering.value) return
  requestReadGeneration++
  activeRequestFile = null
  requestReading.value = false
  open.value = false
  confirmationAction.value = null
  approvalPassword.value = ''
  issuePassword.value = ''
  redeliveryPassword.value = ''
}
function acceptRecord(value: PaidFulfillmentRecord) {
  record.value = value
  orderID.value = value.snapshot.payment.snapshot.order.order_id
  context.value = null
  selectedKey.value = value.claims?.key_id ?? pendingIssue.value?.key_id ?? ''
  if (pendingIssue.value && value.status === 'issued' && value.claims?.key_id === pendingIssue.value.key_id) issueJournal.clear()
  if (props.embedded && value.status === 'approved') void loadProfiles()
}
async function recoverIssue() {
  if (!pendingIssue.value || !validStoredIssue(pendingIssue.value) || loading.value) return
  loading.value = true
  error.value = ''
  try { acceptRecord(await getPaidFulfillment(pendingIssue.value.fulfillment_id)) }
  catch (value) { error.value = message(value, '读取待恢复签发记录失败') }
  finally { loading.value = false }
}
async function recoverRedelivery() {
  if (!pendingRedelivery.value || !validStoredRedelivery(pendingRedelivery.value) || loading.value) return
  loading.value = true
  error.value = ''
  try { acceptRecord(await getPaidFulfillment(pendingRedelivery.value.fulfillment_id)) }
  catch (value) { error.value = message(value, '读取待恢复补发记录失败') }
  finally { loading.value = false }
}
async function loadOrder() {
  const id = orderID.value.trim()
  if (!id || loading.value || pendingApproval.value || pendingIssue.value || pendingRedelivery.value) return
  resetLookup()
  orderID.value = id
  loading.value = true
  error.value = ''
  try {
    try { acceptRecord(await getPaidFulfillmentForOrder(id)) }
    catch (value) {
      if (!(value instanceof OperationsAPIError) || value.status !== 404) throw value
      const loaded = await getPaidFulfillmentContext(id)
      if (loaded.status !== 'fulfillment_pending') throw new Error('该订单当前不能建立初次付费交付')
      context.value = loaded
    }
  } catch (value) { error.value = message(value, '读取付费交付信息失败') }
  finally { loading.value = false }
}
async function selectRequest(file: File) {
  const generation = ++requestReadGeneration
  activeRequestFile = file
  requestText.value = ''
  error.value = ''
  requestReading.value = true
  try {
    const text = await readPaidLicenseRequest(file)
    if (generation === requestReadGeneration && open.value && activeRequestFile === file) requestText.value = text
  } catch (value) {
    if (generation === requestReadGeneration && open.value && activeRequestFile === file) {
      requestFile.value = null
      error.value = message(value, '读取安装请求失败')
    }
  } finally {
    if (generation === requestReadGeneration) requestReading.value = false
  }
}
async function loadLifecycle() {
  if (lifecycleKind.value === 'initial' || !lifecycleSourceID.value.trim() || lifecycleLoading.value) return
  lifecycleLoading.value = true
  lifecycleSource.value = null
  error.value = ''
  try {
    const source = await getPaidLifecycleSource(lifecycleKind.value, lifecycleSourceID.value.trim())
    if (shownContext.value && source.customer_id !== shownContext.value.customer_id) throw new Error('来源许可证与新订单客户不一致')
    lifecycleSource.value = source
  } catch (value) { error.value = message(value, '读取当前许可证来源失败') }
  finally { lifecycleLoading.value = false }
}
function changeLifecycleKind() {
  lifecycleSourceID.value = ''
  lifecycleSource.value = null
}
function clearRequest() {
  requestReadGeneration++
  activeRequestFile = null
  requestReading.value = false
  requestFile.value = null
  requestText.value = ''
}
function promptApproval() {
  if (approving.value || loading.value || journalError.value) return
  if (!pendingApproval.value && (!context.value || !requestText.value || !reason.value.trim() || !checked.value)) {
    error.value = '请完整核对订单、到账、安装请求和授权权益'
    return
  }
  error.value = ''
  confirmationAction.value = 'approval'
}
function promptIssue() {
  if (issuing.value || loading.value || journalError.value || issueConflict.value) return
  if (!pendingIssue.value && !selectedKey.value && !record.value?.claims?.key_id) { error.value = '请选择签发密钥'; return }
  error.value = ''
  confirmationAction.value = 'issue'
}
function promptRedelivery() {
  if (redelivering.value || loading.value || journalError.value || issueConflict.value) return
  if (!pendingRedelivery.value && (!redeliveryReason.value.trim() || !redeliveryChecked.value)) { error.value = '请填写补发原因并确认收件对象'; return }
  error.value = ''
  confirmationAction.value = 'redelivery'
}
async function confirmAction(password: string) {
  if (confirmationAction.value === 'approval') { approvalPassword.value = password; await approve() }
  else if (confirmationAction.value === 'issue') { issuePassword.value = password; await issue() }
  else if (confirmationAction.value === 'redelivery') { redeliveryPassword.value = password; await redeliver() }
  if (!error.value) confirmationAction.value = null
}
async function approve() {
  if (approving.value || loading.value || journalError.value || !approvalPassword.value) return
  error.value = ''
  if (!pendingApproval.value) {
    const source = context.value
    if (!source || source.status !== 'fulfillment_pending' || !requestText.value || !reason.value.trim() || !checked.value
      || lifecycleKind.value !== 'initial' && (!lifecycleSource.value || lifecycleSource.value.source_id !== lifecycleSourceID.value.trim() || lifecycleSource.value.kind !== lifecycleKind.value)) {
      error.value = '请完整核对订单、到账、安装请求和授权权益'
      return
    }
    try {
      const lifecycle = lifecycleKind.value === 'initial' ? undefined : {
        kind: lifecycleKind.value,
        source_id: lifecycleSource.value!.source_id,
        expected_document_sha256: lifecycleSource.value!.document_sha256,
      }
      approvalJournal.prepare({
        operation_id: `fulfillment_${crypto.randomUUID()}`,
        order_id: source.order_id,
        expected_order_sha256: source.order_sha256,
        expected_payment_sha256: source.payment_sha256,
        license_request_json: requestText.value,
        reason: reason.value.trim(),
        ...(lifecycle ? { lifecycle } : {}),
        context: source,
      })
    } catch (value) { error.value = message(value, '保存待确认批准失败'); return }
  }
  approving.value = true
  let attempt: ReturnType<typeof approvalJournal.begin> | undefined
  try {
    attempt = approvalJournal.begin()
    const saved = pendingApproval.value!
    const input: ApprovePaidFulfillmentInput = {
      operation_id: saved.operation_id,
      expected_order_sha256: saved.expected_order_sha256,
      expected_payment_sha256: saved.expected_payment_sha256,
      license_request_json: saved.license_request_json,
      reason: saved.reason,
      ...(saved.lifecycle ? { lifecycle: saved.lifecycle } : {}),
    }
    const result = await approvePaidFulfillment(saved.order_id, input, approvalPassword.value)
    approvalJournal.clear()
    acceptRecord(result)
    checked.value = false
    toast.success('付费交付已批准')
  } catch (value) {
    const rejected = !!attempt && value instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(value.status) && approvalJournal.reject(attempt)
    error.value = `${message(value, '批准付费交付失败')}。${rejected ? '本次请求已明确拒绝，请重新读取订单核对' : '结果尚未确认，请保留原请求并重试'}`
  } finally { approving.value = false; approvalPassword.value = '' }
}
async function loadProfiles() {
  if (profilesLoading.value || !record.value || record.value.status !== 'approved') return
  profilesLoading.value = true
  profilesError.value = ''
  try {
    profiles.value = await listV2IssuerProfiles() || []
    if (keyOptions.value.length === 1) selectedKey.value = keyOptions.value[0]!.value
    if (!keyOptions.value.length) profilesError.value = '没有允许商业订单、安装绑定和固定期限的签发配置'
  } catch (value) { profilesError.value = message(value, '读取签发配置失败') }
  finally { profilesLoading.value = false }
}
async function issue() {
  const current = record.value
  if (!current || current.status === 'issued' || issueConflict.value || issuing.value || loading.value || journalError.value || !issuePassword.value) return
  error.value = ''
  if (!pendingIssue.value) {
    const key = current.claims?.key_id ?? selectedKey.value
    if (!key) { error.value = '请选择受限付费签发密钥'; return }
    try { issueJournal.prepare({ operation_id: `issue_${current.snapshot.id}`, fulfillment_id: current.snapshot.id, key_id: key }) }
    catch (value) { error.value = message(value, '保存待确认签发失败'); return }
  }
  issuing.value = true
  let attempt: ReturnType<typeof issueJournal.begin> | undefined
  try {
    attempt = issueJournal.begin()
    const saved = pendingIssue.value!
    const result = await issuePaidFulfillment(saved.fulfillment_id, saved.key_id, issuePassword.value)
    issueJournal.clear()
    acceptRecord(result)
    emit('completed')
    toast.success('付费授权已签发')
  } catch (value) {
    const rejected = !!attempt && value instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(value.status) && issueJournal.reject(attempt)
    error.value = `${message(value, '签发付费授权失败')}。${rejected ? '本次请求已明确拒绝，请重新读取履约记录' : '结果尚未确认，请保留原签发身份并重试'}`
  } finally { issuing.value = false; issuePassword.value = '' }
}
async function download() {
  if (!record.value || record.value.status !== 'issued' || issueConflict.value || downloading.value) return
  downloading.value = true
  error.value = ''
  try { await downloadPaidFulfillment(record.value); toast.success('付费授权文件摘要已核对') }
  catch (value) { error.value = message(value, '下载付费授权失败') }
  finally { downloading.value = false }
}
async function redeliver() {
  const current = record.value
  if (!current || current.status !== 'issued' || !current.document_sha256 || issueConflict.value || redelivering.value || loading.value || journalError.value || !redeliveryPassword.value) return
  error.value = ''
  if (!pendingRedelivery.value) {
    if (!redeliveryReason.value.trim() || !redeliveryChecked.value) {
      error.value = '请填写补发原因并确认收件对象'
      return
    }
    try {
      redeliveryJournal.prepare({
        operation_id: `redelivery_${crypto.randomUUID()}`,
        fulfillment_id: current.snapshot.id,
        expected_document_sha256: current.document_sha256,
        reason: redeliveryReason.value.trim(),
      })
    } catch (value) { error.value = message(value, '保存待确认补发失败'); return }
  }
  redelivering.value = true
  let attempt: ReturnType<typeof redeliveryJournal.begin> | undefined
  let recorded = false
  try {
    attempt = redeliveryJournal.begin()
    const saved = pendingRedelivery.value!
    if (saved.fulfillment_id !== current.snapshot.id || saved.expected_document_sha256 !== current.document_sha256) throw new Error('待补发记录与当前授权文件不一致，请保留浏览器记录并人工核对')
    const input: RecordPaidRedeliveryInput = { operation_id: saved.operation_id, expected_document_sha256: saved.expected_document_sha256, reason: saved.reason }
    const result = await recordPaidRedelivery(saved.fulfillment_id, input, redeliveryPassword.value)
    if (result.snapshot.fulfillment_id !== current.snapshot.id || result.snapshot.document_sha256 !== current.document_sha256) throw new Error('补发回执与当前授权文件不一致，请停止交付并核对')
    redeliveryJournal.clear()
    recorded = true
    redeliveryChecked.value = false
    redeliveryReason.value = ''
    redeliveryOpen.value = false
    emit('completed')
    await downloadPaidFulfillment(current)
    toast.success('补发记录已留痕，原授权文件摘要已核对')
  } catch (value) {
    if (recorded) {
      error.value = `${message(value, '下载原授权文件失败')}。补发记录已经完成，可直接重新下载原授权文件`
      return
    }
    const rejected = !!attempt && value instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(value.status) && redeliveryJournal.reject(attempt)
    error.value = `${message(value, '补发付费授权失败')}。${rejected ? '本次请求已明确拒绝，请重新核对授权文件' : '结果尚未确认，请保留原补发请求并重试'}`
  } finally { redelivering.value = false; redeliveryPassword.value = '' }
}
async function copy(value: string) {
  try { await navigator.clipboard.writeText(value); toast.success('已复制') }
  catch { toast.error('复制失败，请检查剪贴板权限') }
}
function acceptServerIssueKey() {
  const conflict = issueConflict.value
  if (!conflict || !record.value?.claims?.key_id) return
  issueJournal.clear()
  selectedKey.value = record.value.claims.key_id
  error.value = ''
  toast.info('已结束本标签页的旧密钥请求，并采用服务器冻结记录')
}
onUnmounted(() => { requestReadGeneration++; activeRequestFile = null })
</script>

<template>
  <AButton v-if="props.launcher" variant="secondary" @click="start">{{ actionLabel }}</AButton>
  <component :is="props.embedded ? 'section' : AModal" v-bind="surfaceProps" :class="{ 'fulfillment-workspace': props.embedded }" @close="close">
    <div class="fulfillment-flow">
      <p v-if="journalError || error" class="fulfillment-error" role="alert">{{ journalError || error }}</p>
      <ol v-if="!props.embedded" class="fulfillment-steps" aria-label="付费授权办理进度">
        <li :class="{ active: flowStep === 1, complete: flowStep > 1 }"><span>1</span><strong>导入与批准</strong></li>
        <li :class="{ active: flowStep === 2, complete: flowStep > 2 }"><span>2</span><strong>签发授权</strong></li>
        <li :class="{ active: flowStep === 3 }"><span>3</span><strong>下载交付</strong></li>
      </ol>
      <fieldset v-if="!props.embedded && !pendingApproval && !pendingIssue && !pendingRedelivery && !context && !record" class="fulfillment-lookup" :disabled="loading">
        <label class="field"><span>订单编号</span><input v-model="orderID" aria-label="付费交付订单编号" required @input="resetLookup"></label>
        <AButton variant="secondary" :disabled="!orderID.trim()" @click="loadOrder">读取交付状态</AButton>
      </fieldset>
      <ALoadingState v-if="loading" label="正在核对原订单与交付记录" />

      <template v-if="shownContext && !record">
        <p class="fulfillment-stage">等待批准</p>
        <dl class="fulfillment-info">
          <dt>客户</dt><dd>{{ shownContext.customer_id }}</dd>
          <dt>固定套餐</dt><dd>{{ shownContext.plan.definition.name }} v{{ shownContext.plan.version }}</dd>
          <dt>成交金额</dt><dd>{{ formatAmount(shownContext.amount_minor, shownContext.currency) }}</dd>
          <dt>合同期限</dt><dd>{{ shownContext.starts_at }} 至 {{ shownContext.ends_at }}</dd>
          <dt>订单来源</dt><dd>{{ shownContext.source === 'manual' ? '手工固定订单' : shownContext.source === 'local' ? '本地核对官网' : '生产官网' }}</dd>
          <dt>履约环境</dt><dd>{{ shownContext.environment || '未配置' }}</dd>
          <dt>付款记录</dt><dd>{{ shownContext.payment_id }}</dd>
        </dl>
        <details class="fulfillment-rights" :open="!props.embedded"><summary>订单权益</summary><CommercialPlanSummary :definition="shownContext.plan.definition" /></details>
        <form class="fulfillment-form" @submit.prevent="promptApproval">
          <template v-if="pendingApproval && (!props.embedded || props.viewStep >= 5)">
            <p class="fulfillment-note">结果尚未确认，以下是原请求。重新输入当前密码后只重试同一批准。</p>
            <dl class="fulfillment-info"><dt>批准说明</dt><dd>{{ pendingApproval.reason }}</dd><dt>操作编号</dt><dd>{{ pendingApproval.operation_id }}</dd></dl>
          </template>
          <template v-else-if="!pendingApproval && (!props.embedded || props.viewStep === 4)">
            <label class="field"><span>本次履约类型</span><ASelect v-model="lifecycleKind" aria-label="付费履约类型" :options="lifecycleOptions" @update:model-value="changeLifecycleKind" /></label>
            <fieldset v-if="lifecycleKind !== 'initial'" class="fulfillment-lookup" :disabled="lifecycleLoading">
              <label class="field"><span>上一份履约编号</span><input v-model="lifecycleSourceID" aria-label="付费履约来源编号" required @input="lifecycleSource = null"></label>
              <AButton type="button" variant="secondary" :loading="lifecycleLoading" :disabled="!lifecycleSourceID.trim()" @click="loadLifecycle">核对当前来源</AButton>
            </fieldset>
            <AFilePicker v-model="requestFile" label="选择 v2 安装请求" hint="JSON 最大 16 KiB，也可选择 CLI 生成的二维码图片" accept="application/json,.json,image/png,image/jpeg,image/webp,.png,.jpg,.jpeg,.webp" required :loading="requestReading" @select="selectRequest" @clear="clearRequest" />
          </template>
          <dl v-if="lifecycleSource" class="fulfillment-info"><dt>当前许可证</dt><dd>{{ lifecycleSource.license_id }}</dd><dt>当前安装</dt><dd>{{ lifecycleSource.binding.installation_id }}</dd><dt>当前有效期</dt><dd>{{ lifecycleSource.valid_from }} 至 {{ lifecycleSource.valid_until }}</dd><dt>当前文件摘要</dt><dd>{{ lifecycleSource.document_sha256 }}</dd></dl>
          <template v-if="!props.embedded || props.viewStep >= 5">
            <p v-if="requestText && !pendingApproval" class="fulfillment-note">安装请求 {{ requestFile?.name || '已读取' }}</p>
            <label v-if="!pendingApproval" class="field"><span>批准说明</span><textarea v-model="reason" aria-label="付费交付批准说明" maxlength="2000" rows="3" required /></label>
            <details class="fulfillment-technical"><summary>技术核验信息</summary><ACopyCode v-if="requestText" :value="requestText" layout="block" label="复制" copied-label="已复制" @copy="copy(requestText)" /><ACopyCode :value="shownContext.order_sha256" label="复制" copied-label="已复制" @copy="copy(shownContext.order_sha256)" /><ACopyCode :value="shownContext.payment_sha256" label="复制" copied-label="已复制" @copy="copy(shownContext.payment_sha256)" /></details>
            <div class="fulfillment-submit-row"><label v-if="!pendingApproval" class="fulfillment-check"><input v-model="checked" type="checkbox" required><span>已核对原订单、全额到账、目标安装、履约来源和完整权益</span></label><AButton type="submit" :loading="approving" :disabled="!!journalError || !shownContext.environment">{{ pendingApproval ? '重试原批准' : '确认批准交付' }}</AButton></div>
          </template>
        </form>
      </template>

      <template v-if="record">
        <div class="fulfillment-record-head"><strong>履约记录</strong><span class="fulfillment-stage">{{ statusName(record.status) }}</span></div>
        <dl v-if="props.embedded && props.viewStep === 4" class="fulfillment-info fulfillment-node-summary">
          <dt>安装编号</dt><dd>{{ record.snapshot.installation_request.installation_id }}</dd>
          <dt>请求编号</dt><dd>{{ record.snapshot.installation_request.request_id }}</dd>
          <dt>请求摘要</dt><dd class="code">{{ record.snapshot.request_sha256 }}</dd>
        </dl>
        <dl v-else-if="props.embedded && props.viewStep === 5" class="fulfillment-info fulfillment-node-summary">
          <dt>批准说明</dt><dd>{{ record.snapshot.request.reason }}</dd>
          <dt>批准人</dt><dd>{{ record.snapshot.approved_by }}</dd>
          <dt>批准时间</dt><dd>{{ record.snapshot.approved_at }}</dd>
          <dt>履约环境</dt><dd>{{ record.snapshot.environment === 'local' ? '本地验证' : '生产' }}</dd>
        </dl>
        <dl v-else-if="props.embedded && props.viewStep === 6 && record.status !== 'approved'" class="fulfillment-info fulfillment-node-summary">
          <dt>签发密钥</dt><dd>{{ record.claims?.key_id || '尚未冻结' }}</dd>
          <dt>履约编号</dt><dd class="code">{{ record.snapshot.id }}</dd>
        </dl>
        <dl v-else-if="props.embedded && props.viewStep === 7 && record.status === 'issued'" class="fulfillment-info fulfillment-node-summary">
          <dt>签发密钥</dt><dd>{{ record.claims?.key_id }}</dd>
          <dt>授权文件摘要</dt><dd class="code">{{ record.document_sha256 }}</dd>
        </dl>
        <section v-if="!props.embedded || props.viewStep === 8" class="fulfillment-record-details">
          <dl class="fulfillment-info">
          <dt>履约编号</dt><dd class="code">{{ record.snapshot.id }}</dd>
          <dt>客户</dt><dd>{{ record.snapshot.payment.snapshot.order.customer_id }}</dd>
          <dt>固定套餐</dt><dd>{{ record.snapshot.payment.snapshot.order.plan.definition.name }} v{{ record.snapshot.payment.snapshot.order.plan.version }}</dd>
          <dt>合同期限</dt><dd>{{ record.snapshot.payment.snapshot.order.starts_at }} 至 {{ record.snapshot.payment.snapshot.order.ends_at }}</dd>
          <dt>履约环境</dt><dd>{{ record.snapshot.environment === 'local' ? '本地验证' : '生产' }}</dd>
          <dt>批准人</dt><dd>{{ record.snapshot.approved_by }}</dd>
          <dt>批准时间</dt><dd>{{ record.snapshot.approved_at }}</dd>
          <dt>安装编号</dt><dd>{{ record.snapshot.installation_request.installation_id }}</dd>
          <dt>请求编号</dt><dd>{{ record.snapshot.installation_request.request_id }}</dd>
          <dt>签发密钥</dt><dd>{{ record.claims?.key_id || '尚未冻结' }}</dd>
          <template v-if="recordLifecycle">
            <dt>履约类型</dt><dd>{{ lifecycleOptions.find(item => item.value === recordLifecycle?.kind)?.label }}</dd>
            <dt>来源编号</dt><dd>{{ recordLifecycle.source_id }}</dd>
            <dt>来源文件摘要</dt><dd>{{ recordLifecycle.document_sha256 }}</dd>
          </template>
          </dl>
        </section>
        <section v-if="!props.embedded || props.viewStep === 8" class="fulfillment-rights"><h3>订单权益</h3><CommercialPlanSummary v-if="planDefinition" :definition="planDefinition" /></section>
        <section v-if="!props.embedded || props.viewStep === 8" class="fulfillment-technical-info"><h3>技术核验</h3><div class="fulfillment-hashes"><label><span>安装请求摘要</span><ACopyCode :value="record.snapshot.request_sha256" label="复制" copied-label="已复制" @copy="copy(record.snapshot.request_sha256)" /></label><label><span>履约记录摘要</span><ACopyCode :value="record.sha256" label="复制" copied-label="已复制" @copy="copy(record.sha256)" /></label></div></section>

        <div v-if="issueConflict" class="fulfillment-conflict" role="alert">
          <strong>服务器已经固定另一签发密钥</strong>
          <p>本标签页保留 {{ issueConflict.local }}，可信履约记录固定为 {{ issueConflict.server }}。原请求不能继续，核对后采用服务器记录。</p>
          <AButton variant="secondary" @click="acceptServerIssueKey">确认采用服务器固定密钥</AButton>
        </div>

        <div v-if="props.embedded && props.viewStep === 6 && record.status === 'approved'" class="fulfillment-key-choice">
          <label v-if="profiles.length" class="field"><span>付费签发密钥</span><ASelect v-model="selectedKey" aria-label="付费签发密钥" :options="keyOptions" required /></label>
          <ALoadingState v-else-if="profilesLoading" label="正在读取签发配置" />
          <AButton v-else variant="secondary" :loading="profilesLoading" @click="loadProfiles">读取签发配置</AButton>
          <p v-if="profilesError" class="fulfillment-error" role="alert">{{ profilesError }}</p>
          <AButton :disabled="!selectedKey || !!journalError || !!issueConflict" @click="emit('ready-to-issue')">下一步：签发授权</AButton>
        </div>

        <form v-if="record.status !== 'issued' && (!props.embedded || props.viewStep === 7)" class="fulfillment-form fulfillment-issue" @submit.prevent="promptIssue">
          <template v-if="record.status === 'approved' && !pendingIssue">
            <AButton v-if="!profiles.length && (!props.embedded || profilesError)" type="button" variant="secondary" :loading="profilesLoading" @click="loadProfiles">重新读取签发配置</AButton>
            <label v-else-if="profiles.length" class="field"><span>付费签发密钥</span><ASelect v-model="selectedKey" aria-label="付费签发密钥" :options="keyOptions" required /></label>
            <ALoadingState v-else label="正在读取签发配置" />
          </template>
          <dl v-else class="fulfillment-info"><dt>固定签发密钥</dt><dd>{{ pendingIssue?.key_id || record.claims?.key_id }}</dd></dl>
          <p v-if="profilesError" class="fulfillment-error" role="alert">{{ profilesError }}</p>
          <AButton type="submit" :loading="issuing" :disabled="!!journalError || !!issueConflict || (record.status === 'approved' && !pendingIssue && !selectedKey)">{{ pendingIssue || record.status === 'prepared' ? '重试原签发' : '签发付费授权' }}</AButton>
        </form>

        <div v-if="record.status === 'issued' && (!props.embedded || props.viewStep === 8)" class="fulfillment-download">
          <label class="fulfillment-document"><span>授权文件摘要</span><ACopyCode :value="record.document_sha256 || ''" label="复制" copied-label="已复制" @copy="copy(record.document_sha256 || '')" /></label>
          <AButton icon="download" :loading="downloading" :disabled="!!issueConflict || redelivering" @click="download">下载授权文件</AButton>
        </div>
        <div v-if="record.status === 'issued' && (!props.embedded || props.viewStep === 8)" class="fulfillment-secondary">
          <strong>后续办理</strong>
          <div class="fulfillment-secondary-actions"><PaidTransfer :fulfillment="record" @completed="emit('completed')" /><AButton variant="secondary" @click="redeliveryOpen = true">补发授权文件</AButton></div>
        </div>
        <AModal :open="redeliveryOpen && record.status === 'issued'" title="补发授权文件" @close="redeliveryOpen = false">
          <form class="fulfillment-form fulfillment-redelivery" @submit.prevent="promptRedelivery">
            <template v-if="pendingRedelivery">
              <p class="fulfillment-note">上次补发结果尚未确认。重新输入当前密码后只重试同一操作，不会重新签发授权。</p>
              <dl class="fulfillment-info"><dt>补发原因</dt><dd>{{ pendingRedelivery.reason }}</dd><dt>操作编号</dt><dd>{{ pendingRedelivery.operation_id }}</dd></dl>
            </template>
            <template v-else>
              <label class="field"><span>补发原因</span><textarea v-model="redeliveryReason" aria-label="付费授权补发原因" maxlength="2000" rows="2" required /></label>
              <label class="fulfillment-check"><input v-model="redeliveryChecked" aria-label="确认补发原授权文件" type="checkbox" required><span>已核对收件对象，只补发上述摘要对应的原授权文件</span></label>
            </template>
            <AButton type="submit" :loading="redelivering" :disabled="!!journalError || !!issueConflict || (!pendingRedelivery && (!redeliveryReason.trim() || !redeliveryChecked))">{{ pendingRedelivery ? '重试原补发' : '登记并补发原授权' }}</AButton>
          </form>
        </AModal>
      </template>
      <AButton v-if="!props.embedded" variant="secondary" :disabled="loading || approving || issuing || downloading || redelivering" @click="close">关闭</AButton>
    </div>
  </component>
  <ReauthActionModal :open="!!confirmationAction" :title="confirmationTitle" :busy="confirmationBusy" :error="error" @close="confirmationAction = null" @submit="confirmAction" />
</template>

<style scoped>
.fulfillment-flow, .fulfillment-form, .fulfillment-download { display: grid; gap: 14px; min-width: 0; }
.fulfillment-steps { display:grid;grid-template-columns:repeat(3,1fr);gap:8px;margin:0;padding:0;list-style:none; }
.fulfillment-steps li { display:flex;align-items:center;gap:8px;min-width:0;padding:10px;border:1px solid var(--line);border-radius:12px;color:var(--muted);font-size:var(--font-size-body); }
.fulfillment-steps li span { display:grid;place-items:center;width:22px;height:22px;flex:0 0 22px;border-radius:999px;background:var(--surface-3);font-variant-numeric:tabular-nums; }
.fulfillment-steps li.active { border-color:color-mix(in srgb,var(--accent) 45%,var(--line));color:var(--accent);background:var(--accent-soft); }
.fulfillment-steps li.complete { color:var(--positive); }
.fulfillment-steps li.complete span { background:var(--positive-soft); }
.fulfillment-lookup { display: grid; grid-template-columns: minmax(0, 1fr) max-content; align-items: end; gap: 12px; border: 0; padding: 0; margin: 0; min-width: 0; }
.fulfillment-info { display: grid; grid-template-columns: 108px minmax(0, 1fr); gap: 10px 16px; margin: 4px 0; font-size:var(--font-size-body); }
.fulfillment-info dt, .fulfillment-note { color: var(--muted); }
.fulfillment-info dd { min-width: 0; margin: 0; overflow-wrap: anywhere; }
.fulfillment-stage { width: max-content; margin: 0; border-radius: 999px; padding: 5px 10px; background: var(--accent-soft); color: var(--accent); font-size:var(--font-size-body); font-weight: 650; }
.fulfillment-error { margin: 0; border: 1px solid var(--line); border-radius: 10px; padding: 12px; font-size:var(--font-size-body); line-height: 1.6; }
.fulfillment-conflict { display: grid; gap: 8px; border: 1px solid color-mix(in srgb,var(--warning) 42%,var(--line)); border-radius: 12px; padding: 13px; background: var(--warning-soft); font-size:var(--font-size-body); line-height: 1.6; }
.fulfillment-conflict p { margin: 0; overflow-wrap: anywhere; }
.fulfillment-note, .fulfillment-check { margin: 0; font-size:var(--font-size-body); line-height: 1.7; }
.fulfillment-check { display: flex; align-items: flex-start; gap: 10px; }
.fulfillment-check input { flex: 0 0 auto; width: 16px; height: 16px; margin-top: 3px; }
.fulfillment-issue { margin-top: 4px; border-top: 1px solid var(--line); padding-top: 16px; }
.fulfillment-workspace { min-width:0; }
.fulfillment-workspace details { max-width:100%; }
.fulfillment-rights summary,.fulfillment-technical summary { cursor:pointer;font-weight:650; }
.fulfillment-technical[open] { display:grid;gap:12px; }
.fulfillment-submit-row { display:flex;align-items:center;justify-content:space-between;gap:18px; }
.fulfillment-submit-row > .a-button { margin-left:auto; }
.fulfillment-workspace .fulfillment-form { max-width:1100px; }
.fulfillment-node-summary { border:1px solid var(--line);border-radius:12px;padding:18px; }
.fulfillment-key-choice { display:grid;grid-template-columns:minmax(0,1fr) max-content;align-items:end;gap:14px; }
.fulfillment-key-choice > .fulfillment-error { grid-column:1/-1; }
.fulfillment-key-choice > .a-button:only-child { justify-self:start; }
.fulfillment-workspace .fulfillment-issue { max-width:none;grid-template-columns:minmax(0,1fr) max-content;align-items:end; }
.fulfillment-workspace .fulfillment-issue > .fulfillment-error { grid-column:1/-1; }
.fulfillment-workspace .fulfillment-download { grid-template-columns:minmax(0,1fr) max-content;align-items:center; }
.fulfillment-workspace .fulfillment-download > .a-button { justify-self:end; }
.fulfillment-record-head { display:flex;align-items:center;gap:10px;min-height:28px; }
.fulfillment-record-head strong { font-size:var(--font-size-body); }
.fulfillment-record-details { border:1px solid var(--line);border-radius:10px;padding:10px 14px; }
.fulfillment-record-details .fulfillment-info { gap:5px 12px;margin:0;font-size:var(--font-size-body); }
.fulfillment-rights,.fulfillment-technical-info { min-width:0; }
.fulfillment-rights h3,.fulfillment-technical-info h3 { margin:0 0 6px;font-size:var(--font-size-body); }
.fulfillment-rights :deep(.commercial-summary dl) { grid-template-columns:repeat(2,minmax(84px,max-content) minmax(0,1fr));gap:5px 10px;margin:0;font-size:var(--font-size-body);line-height:1.45; }
.fulfillment-rights :deep(.commercial-summary p) { margin:0 0 6px; }
.fulfillment-hashes { display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:10px; }
.fulfillment-hashes label,.fulfillment-document { display:grid;min-width:0;gap:5px;color:var(--muted);font-size:var(--font-size-body); }
.fulfillment-workspace .fulfillment-flow { gap:9px; }
.fulfillment-workspace .fulfillment-info { grid-template-columns:105px minmax(0,1fr) 105px minmax(0,1fr); }
.fulfillment-workspace .fulfillment-download { grid-template-columns:minmax(0,1fr) max-content;gap:10px; }
.fulfillment-workspace .fulfillment-download > .fulfillment-document { grid-column:1; }
.fulfillment-secondary { display:flex;align-items:center;justify-content:space-between;gap:14px;padding:10px 0 0;border:0;border-top:1px solid var(--line);border-radius:0; }
.fulfillment-secondary strong { font-size:var(--font-size-body); }
.fulfillment-secondary-actions { display:flex;align-items:center;gap:8px; }
.fulfillment-redelivery { margin:0;padding:0;border:0; }
.fulfillment-redelivery > .a-button { justify-self:end; }
details { min-width: 0; }
summary { cursor: pointer; font-size:var(--font-size-body); font-weight: 600; }
@media (max-width: 560px) {
  .fulfillment-lookup { grid-template-columns: 1fr; }
  .fulfillment-key-choice { grid-template-columns: 1fr; }
  .fulfillment-info { grid-template-columns: 88px minmax(0, 1fr); }
  .fulfillment-hashes,.fulfillment-rights :deep(.commercial-summary dl) { grid-template-columns:1fr; }
  .fulfillment-secondary { align-items:flex-start;flex-direction:column; }
}
@media (max-width: 900px) { .fulfillment-workspace .fulfillment-info { grid-template-columns:88px minmax(0,1fr); } }
</style>
