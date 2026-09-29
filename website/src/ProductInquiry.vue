<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import TurnstileWidget from './TurnstileWidget.vue'
import validate from '../shared/generated/inquiry-validator.js'
import type { InquiryReference, ProductInquiry } from '../shared/generated/contracts'
import { readPendingInquiry, savePendingInquiry, clearPendingInquiry, type PendingInquiry } from './inquiry-journal'

const props = defineProps<{ locale: 'zh' | 'en'; sitekey: string; email: string }>()
const emit = defineEmits<{ busy: [value: boolean] }>()
const contact = ref('')
const message = ref('')
const website = ref('')
const selected = ref<{ label: string; reference: InquiryReference } | null>(null)
const state = ref<'idle' | 'verifying' | 'sending' | 'success' | 'error'>('idle')
const error = ref('')
const contactInput = ref<HTMLInputElement | null>(null)
const root = ref<HTMLElement | null>(null)
const verificationRequested = ref(false)
const unconfirmed = ref<PendingInquiry | null>(null)
const recoveryError = ref(false)
let pending: Omit<ProductInquiry, 'turnstile_token'> | null = null
const busy = computed(() => state.value === 'verifying' || state.value === 'sending')
const locked = computed(() => busy.value || unconfirmed.value !== null || recoveryError.value)
const t = computed(() => props.locale === 'zh' ? {
  title: '咨询采购', intro: '留下联系方式和需求，我们会与你确认适用版本、部署条件与交付安排。',
  contact: '联系方式', contactHint: '邮箱、微信或电话', message: '需求说明', messageHint: '想解决什么问题 预计如何使用',
  selected: '意向版本', clear: '清除选择', submit: '发送咨询', sending: '正在提交', verifying: '完成安全验证后提交',
  fallback: '也可以通过邮箱联系', unavailable: '在线咨询暂不可用 请通过邮箱联系',
  received: '咨询已收到', receivedDetail: '我们会通过你留下的方式联系你。', again: '提交新的咨询',
  invalid: '请填写 2–200 字的联系方式和 2–4000 字的需求说明', verification: '安全验证未完成 请重试或通过邮箱联系',
  failed: '提交结果暂时无法确认 输入已保留 再次提交会核对原请求', rate: '提交较频繁 请稍后重试或通过邮箱联系',
  conflict: '该请求的内容不一致 请通过邮箱联系并说明情况',
  recover: '核对原咨询', pending: '上次提交的结果尚未确认。重试会核对原咨询，内容与意向版本暂时保持不变。',
  newWarning: '原咨询可能已经收到。开始新的咨询可能产生另一条记录。', storage: '无法保存或读取本次提交记录，请通过邮箱联系。', cancel: '取消验证',
} : {
  title: 'Talk about Aster', intro: 'Leave your contact details and requirements to discuss plans, deployment and delivery.',
  contact: 'Contact details', contactHint: 'Email, phone, Telegram or WhatsApp', message: 'Requirements', messageHint: 'What would you like to do with Aster',
  selected: 'Selected plan', clear: 'Clear selection', submit: 'Send inquiry', sending: 'Submitting', verifying: 'Complete verification to submit',
  fallback: 'You can also contact us by email', unavailable: 'Online inquiries are unavailable Please contact us by email',
  received: 'Inquiry received', receivedDetail: 'We will contact you using the details you provided.', again: 'Send another inquiry',
  invalid: 'Enter 2–200 characters for contact details and 2–4000 for requirements', verification: 'Verification was not completed Please retry or contact us by email',
  failed: 'The result could not be confirmed Your input is retained Retrying checks the original request', rate: 'Too many submissions Please retry later or contact us by email',
  conflict: 'This request has different content Please contact us by email',
  recover: 'Check original inquiry', pending: 'The previous result is unconfirmed. Retrying checks the original inquiry with its original content and plan.',
  newWarning: 'The original inquiry may have been received. Starting another may create a second record.', storage: 'The submission record cannot be saved or read. Please contact us by email.', cancel: 'Cancel verification',
})

function setState(value: typeof state.value) {
  state.value = value
  emit('busy', value === 'verifying' || value === 'sending' || unconfirmed.value !== null || recoveryError.value)
}

async function choose(value: { label: string; reference: InquiryReference } | null) {
  if (locked.value) return
  selected.value = value ? { label: value.label, reference: { ...value.reference } } : null
  if (state.value === 'success') startAgain()
  await nextTick()
  root.value?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  contactInput.value?.focus({ preventScroll: true })
}
defineExpose({ choose })

function startAgain() {
  try { clearPendingInquiry(sessionStorage) } catch { error.value = t.value.storage; return }
  contact.value = ''
  message.value = ''
  unconfirmed.value = null
  recoveryError.value = false
  pending = null
  error.value = ''
  verificationRequested.value = false
  setState('idle')
}

async function submit() {
  if (busy.value || !props.sitekey || recoveryError.value) return
  const body = {
    contact: contact.value.trim(), message: message.value.trim(), locale: props.locale, website: website.value,
    ...(selected.value ? { reference: { ...selected.value.reference } } : {}),
  }
  pending = unconfirmed.value ? {
    ...unconfirmed.value.payload,
    ...(unconfirmed.value.payload.reference ? { reference: { ...unconfirmed.value.payload.reference } } : {}),
  } : { request_id: crypto.randomUUID(), ...body }
  if (!validate({ ...pending, turnstile_token: 'pending' })) { error.value = t.value.invalid; setState('error'); return }
  error.value = ''
  setState('verifying')
  // Remount even after a failed script load; reset alone cannot create a widget
  // that never mounted. Shared script loading retries failures without a reload.
  verificationRequested.value = false
  await nextTick()
  verificationRequested.value = true
}

function verificationFailed() {
  if (state.value !== 'verifying') return
  error.value = t.value.verification
  setState('error')
}

async function verified(token: string) {
  if (!token || state.value !== 'verifying' || !pending) return
  const payload: ProductInquiry = { ...pending, turnstile_token: token }
  if (!unconfirmed.value) {
    const record = { payload: pending, label: selected.value?.label ?? '' }
    try { savePendingInquiry(sessionStorage, record) }
    catch { error.value = t.value.storage; setState('error'); return }
    unconfirmed.value = record
  }
  setState('sending')
  try {
    const response = await fetch('/api/inquiries', {
      method: 'POST', headers: { 'Content-Type': 'application/json' }, credentials: 'omit',
      body: JSON.stringify(payload), signal: AbortSignal.timeout(20_000),
    })
    const result: unknown = await response.json()
    if (typeof result !== 'object' || result === null) throw new Error('invalid response')
    const record = result as Record<string, unknown>
    if (response.status === 202 && record.ok === true && record.id === payload.request_id) {
      clearPendingInquiry(sessionStorage, payload.request_id)
      unconfirmed.value = null
      setState('success')
      verificationRequested.value = false
      return
    }
    error.value = record.error === 'rate_limited' ? t.value.rate
      : record.error === 'request_conflict' ? t.value.conflict
        : record.error === 'verification_failed' ? t.value.verification
          : record.error === 'invalid_request' ? t.value.invalid : t.value.failed
  } catch { error.value = t.value.failed }
  setState('error')
}

function cancelVerification() { verificationRequested.value = false; setState('idle') }

onMounted(() => {
  try {
    const record = readPendingInquiry(sessionStorage)
    if (!record) return
    unconfirmed.value = record
    contact.value = record.payload.contact
    message.value = record.payload.message
    selected.value = record.payload.reference ? { label: record.label, reference: record.payload.reference } : null
    setState('error')
  } catch { recoveryError.value = true; error.value = t.value.storage; setState('error') }
})
</script>

<template>
  <div ref="root" class="product-inquiry" data-lenis-prevent>
    <div class="inquiry-intro"><h3>{{ t.title }}</h3><p>{{ t.intro }}</p><p>{{ t.fallback }}<br><a :href="`mailto:${email}`">{{ email }}</a></p><div class="inquiry-community"><h4>{{ locale === 'zh' ? '问题反馈与功能建议' : 'Questions, bugs & feature requests' }}</h4><p>{{ locale === 'zh' ? '欢迎到公开仓库 huzz-open/aster-team 提交 Issue，与我们交流。采购或私密问题请使用表单或邮箱。' : 'Open an issue in huzz-open/aster-team to share feedback with us. Use the form or email for purchasing and private questions.' }}</p><a href="https://github.com/huzz-open/aster-team/issues/new/choose" target="_blank" rel="noopener noreferrer">{{ locale === 'zh' ? '在 GitHub 提交 Issue' : 'Open a GitHub issue' }}</a><p>{{ locale === 'zh' ? 'Issue 内容公开，请勿粘贴 API Key、账号凭据或客户数据。' : 'Issues are public. Never include API keys, account credentials or customer data.' }}</p></div></div>
    <div v-if="state === 'success'" class="inquiry-success" role="status"><h4>{{ t.received }}</h4><p>{{ t.receivedDetail }}</p><button type="button" @click="startAgain">{{ t.again }}</button></div>
    <form v-else @submit.prevent="submit">
      <p v-if="unconfirmed" role="status">{{ t.pending }}</p>
      <p v-if="selected" class="inquiry-selection"><span>{{ t.selected }} · {{ selected.label }}</span><button type="button" :disabled="locked" @click="choose(null)">{{ t.clear }}</button></p>
      <label>{{ t.contact }}<input ref="contactInput" v-model="contact" name="contact" type="text" autocomplete="email" :placeholder="t.contactHint" required maxlength="400" :disabled="locked"></label>
      <label>{{ t.message }}<textarea v-model="message" name="message" :placeholder="t.messageHint" required maxlength="8000" rows="4" :disabled="locked"></textarea></label>
      <input v-model="website" class="inquiry-honeypot" name="website" tabindex="-1" autocomplete="off" aria-hidden="true">
      <TurnstileWidget v-if="verificationRequested" :sitekey="sitekey" @token="verified" @error="verificationFailed" />
      <button v-if="state === 'verifying'" type="button" @click="cancelVerification">{{ t.cancel }}</button>
      <p v-if="error" class="inquiry-error" role="alert">{{ error }}</p>
      <p v-if="!sitekey" role="status">{{ t.unavailable }}</p>
      <div class="inquiry-actions"><button type="submit" :disabled="busy || !sitekey || recoveryError">{{ state === 'sending' ? t.sending : state === 'verifying' ? t.verifying : unconfirmed ? t.recover : t.submit }}</button></div>
      <div v-if="(unconfirmed || recoveryError) && !busy" class="inquiry-new"><p>{{ t.newWarning }}</p><button type="button" @click="startAgain">{{ t.again }}</button></div>
    </form>
  </div>
</template>

<style scoped>
.inquiry-community{margin-top:28px;padding-top:24px;border-top:1px solid var(--line)}.inquiry-community h4{margin:0;font-size:var(--font-size-body);line-height:1.5}.inquiry-community a{display:inline-block;font-size:var(--font-size-body);margin:8px 0}
.product-inquiry{display:grid;grid-template-columns:.82fr 1.18fr;gap:70px;padding-top:66px;border-top:1px solid #d5d1ca;scroll-margin-top:calc(var(--header-height) + 24px)}.inquiry-intro h3{margin:0 0 18px;font-size:var(--font-size-display);letter-spacing:-.04em}.inquiry-intro p{max-width:430px;color:#716e76;font-size:var(--font-size-body);line-height:1.8}.inquiry-intro a{color:#5147ad;text-decoration:underline;text-underline-offset:5px}form{position:relative;display:grid;gap:18px;min-width:0;padding:26px;border:1px solid #d7d3cb;border-radius:14px;background:#fffefa}label{display:grid;gap:8px;font-size:var(--font-size-body);color:#55525b}input,textarea{width:100%;min-width:0;padding:13px 14px;color:#27252d;background:#f8f6f1;border:1px solid #d7d3cc;border-radius:8px;resize:vertical}input:focus,textarea:focus{border-color:#a59fd2;background:#fff}.inquiry-actions{display:flex;align-items:center;gap:14px;flex-wrap:wrap}.inquiry-actions button{min-height:44px;padding:0 18px;border:1px solid #29272f;border-radius:8px;background:#29272f;color:#fff;cursor:pointer;font-size:var(--font-size-body);font-weight:700}.inquiry-actions span{color:#88858d;font-size:var(--font-size-body)}.inquiry-selection{display:flex;align-items:center;flex-wrap:wrap;gap:10px;margin:0;color:#615d69;overflow-wrap:anywhere}.inquiry-selection button{min-height:auto;padding:6px 9px;border:1px solid #d0ccc5;border-radius:6px;background:transparent;color:#605c66;font-size:var(--font-size-body)}.inquiry-honeypot{position:absolute;width:1px;height:1px;opacity:0;pointer-events:none;padding:0}.inquiry-error{color:#a54550;margin:0;line-height:1.7}.inquiry-success h4{margin:0;font-size:var(--font-size-title)}.inquiry-success p{color:#716e76;line-height:1.8}button:disabled{opacity:.55;cursor:default}input:focus-visible,textarea:focus-visible,button:focus-visible,a:focus-visible{outline:3px solid #6558db50;outline-offset:3px}.inquiry-new{padding-top:14px;border-top:1px solid #e3e0da}.inquiry-new p{color:#7c7881;font-size:var(--font-size-body)}.inquiry-new button{padding:8px 10px;border:1px solid #d0ccc5;border-radius:6px;background:transparent;cursor:pointer;font-size:var(--font-size-body)}@media(max-width:760px){.product-inquiry{grid-template-columns:1fr;gap:28px;padding-top:42px}.product-inquiry form{padding:20px 15px}.inquiry-actions{align-items:flex-start;flex-direction:column}.inquiry-actions button{width:100%}}
</style>
