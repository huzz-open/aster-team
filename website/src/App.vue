<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { useLenis } from 'lenis/vue'
import SiteIcon from './SiteIcon.vue'
import ContactLinks from './ContactLinks.vue'
import TurnstileWidget from './TurnstileWidget.vue'
import ProductPlans from './ProductPlans.vue'
import ProductDemo from './ProductDemo.vue'
import HeroGateway from './HeroGateway.vue'
import FreeStart from './FreeStart.vue'
import ClientConnections from './ClientConnections.vue'
import PrivateDeploymentIllustration from './PrivateDeploymentIllustration.vue'

type Locale = 'zh' | 'en'
type UsageEvidence = 'tokens' | 'time'
type DailyAiTime = 'under1h' | '1h-4h' | '4h-8h' | 'over8h'
type LeadSubmitState = 'idle' | 'saving' | 'saved' | 'sending' | 'sent' | 'failed' | 'not-configured' | 'verification-required'
type LeadForm = {
  contact: string
  company: string
  teamSize: number
  activeUsers: number
  evidence: UsageEvidence
  weeklyTokens: number
  dailyTime: DailyAiTime
}

const email = String(import.meta.env.VITE_ASTER_CONTACT_EMAIL || '').trim()
const publicRepositoryUrl = 'https://github.com/huzz-open/aster-team'
const leadEndpoint = String(import.meta.env.VITE_ASTER_LEAD_ENDPOINT || '').trim()
const turnstileSiteKey = String(import.meta.env.VITE_ASTER_TURNSTILE_SITE_KEY || (import.meta.env.DEV ? '1x00000000000000000000AA' : '')).trim()
const locale = ref<Locale>('en')
const mobileNavOpen = ref(false)
const activeSection = ref(0)
const leadSubmitState = ref<LeadSubmitState>('idle')
const leadSuccessOpen = ref(false)
const leadSuccessId = ref('')
const leadSuccessCloseButton = ref<HTMLButtonElement | null>(null)
const leadFallbackOpen = ref(false)
const leadFallbackCloseButton = ref<HTMLButtonElement | null>(null)
const leadFallbackCopyState = ref<'idle' | 'email' | 'content'>('idle')
const leadSubmitButton = ref<HTMLButtonElement | null>(null)
const turnstileDialogOpen = ref(false)
const turnstileDialogPanel = ref<HTMLElement | null>(null)
const leadUsageOpen = ref(false)
const leadUsageActiveIndex = ref(0)
const leadUsageRoot = ref<HTMLElement | null>(null)
const leadUsageButton = ref<HTMLButtonElement | null>(null)
const turnstileWidget = ref<InstanceType<typeof TurnstileWidget> | null>(null)
const turnstileToken = ref('')
const leadHoneypot = ref('')
const leadForm = ref<LeadForm>({
  contact: '',
  company: '',
  teamSize: 10,
  activeUsers: 4,
  evidence: 'time',
  weeklyTokens: 1,
  dailyTime: '1h-4h',
})
const leadDraftCacheKey = 'aster-team:trial-lead-draft:v1'

const copy = {
  zh: {
    private: {
      trust: ['订阅账号由企业持有', '成员权限由企业控制', '用量记录在内部留存', '不经公共中转或第三方托管'],
      environment: '企业环境', user: '用户端', admin: '管理端', control: 'Control 控制层', data: '私有数据', runner: 'Runner 集群', upstream: 'ChatGPT 订阅', upstreamAuth: '登录授权', inference: '模型调用', license: '离线\n许可证',
    },
    lead: {
      title: '填写团队情况', subtitle: '分享团队规模和日常使用情况，方便我们了解你的部署需求。',
      contact: '联系方式', contactPlaceholder: '微信、手机号或邮箱', company: '团队 / 企业名称（选填）', companyPlaceholder: '例如：星河工作室',
      teamSize: '团队成员', activeUsers: '经常使用 AI', people: '人', evidence: '你们有近 7 天 Token 使用数据吗？',
      hasTokens: '有，填写 Token 用量', noTokens: '没有，填写使用时长', weeklyTokens: '团队近 7 天 Token 用量', tokenUnit: '亿 Token', dailyTime: '平均每天使用 AI 的时间',
      timeOptions: [['under1h', '小于 1 小时'], ['1h-4h', '1–4 小时'], ['4h-8h', '4–8 小时'], ['over8h', '8 小时以上']],
      autoSaved: '填写内容会自动保存在当前设备',
      save: '保存草稿', saved: '草稿已保存', submit: '提交试用申请', sending: '正在提交', sent: '申请已提交，我们会尽快联系你。',
      failed: '提交失败，请稍后再试。', notConfigured: '草稿已保存；申请接口尚未配置。', verificationRequired: '请完成安全验证，我们会在验证通过后自动提交。', privacy: '提交的信息仅用于团队方案评估和联系，不会公开。',
      verification: { title: '完成安全验证', body: '验证通过后，试用申请将自动提交。' },
      success: { title: '试用申请已提交', body: '我们已经收到你的团队信息，会尽快通过你填写的联系方式与你联系。', applicationId: '申请编号', close: '完成' },
      fallback: {
        title: '申请暂未提交成功', intro: '表单内容已经保留。请将下面的申请信息发送到我们的联系邮箱，我们会继续为你安排试用。',
        recipient: '收件邮箱', subject: '邮件主题', body: '邮件正文', copyEmail: '复制邮箱', copiedEmail: '邮箱已复制',
        copyContent: '复制完整邮件', copiedContent: '邮件内容已复制', openMail: '打开邮件客户端', close: '关闭',
      },
    },
  },
  en: {
    private: {
      trust: ['Company-owned subscriptions', 'Company-controlled member access', 'Usage records retained internally', 'No public relay or third-party custody'],
      environment: 'Customer environment', user: 'Member portal', admin: 'Admin console', control: 'Control plane', data: 'Private data', runner: 'Runner pool', upstream: 'ChatGPT subscription', upstreamAuth: 'OAuth\nsign-in', inference: 'Model\nrequests', license: 'Offline\nlicense',
    },
    lead: {
      title: 'Tell us about your team', subtitle: 'Share your team size and everyday usage to help us understand your deployment needs.',
      contact: 'Contact details', contactPlaceholder: 'Email, phone, Telegram or WhatsApp', company: 'Team / company (optional)', companyPlaceholder: 'Example: Northstar Studio',
      teamSize: 'Team members', activeUsers: 'Regular AI users', people: 'people', evidence: 'Do you have token data from the last 7 days?',
      hasTokens: 'Yes, use token data', noTokens: 'No, share daily usage time', weeklyTokens: 'Team tokens in the last 7 days', tokenUnit: '100M tokens', dailyTime: 'Average daily AI usage time',
      timeOptions: [['under1h', 'Under 1 hour'], ['1h-4h', '1–4 hours'], ['4h-8h', '4–8 hours'], ['over8h', 'Over 8 hours']],
      autoSaved: 'Your draft is saved on this device automatically',
      save: 'Save draft', saved: 'Draft saved', submit: 'Submit trial request', sending: 'Submitting', sent: 'Request submitted. We will contact you soon.',
      failed: 'Submission failed. Please try again later.', notConfigured: 'Draft saved; the submission endpoint is not configured.', verificationRequired: 'Complete the security check and your request will submit automatically.', privacy: 'Submitted information is used only for team assessment and follow-up.',
      verification: { title: 'Complete security check', body: 'Your trial request will submit automatically after verification.' },
      success: { title: 'Trial request submitted', body: 'We have received your team details and will contact you shortly using the contact information provided.', applicationId: 'Request ID', close: 'Done' },
      fallback: {
        title: 'Your request was not submitted', intro: 'Your form details are still saved. Please email the request below to us and we will continue arranging your trial.',
        recipient: 'Send to', subject: 'Subject', body: 'Email body', copyEmail: 'Copy email', copiedEmail: 'Email copied',
        copyContent: 'Copy full email', copiedContent: 'Email copied', openMail: 'Open email app', close: 'Close',
      },
    },
  },
} as const

const t = computed(() => copy[locale.value])
const pageCopy = computed(() => locale.value === 'zh' ? {
  nav: ['产品体验', '核心能力', '私有部署', '下载与价格'],
  hero: {
    eyebrow: '面向企业与团队的私有部署 AI 网关',
    title: ['让团队用好 AI，', '让每一份投入可控。'],
    body: '统一接入订阅/账号，按成员分配 Key 与额度，记录每次调用的用量。',
    benefit: '让资源利用更充分，让团队成本更清晰。',
    primary: '免费下载', secondary: '体验产品',
  },
  demo: {
    eyebrow: '直接体验产品', title: '看看团队如何使用 Aster',
    body: '从管理员分配资源，到成员查看额度和使用记录，体验团队日常使用 Aster 的工作流程。',
  },
  capabilities: {
    eyebrow: 'ASTER 提供什么', title: '团队 AI 接入与管理，在一处完成',
    body: '连接自有且已合法授权的 AI 账号，让成员独立使用，让管理员看清并控制团队用量。',
    items: [
      ['私有化团队网关', '在客户管理的基础设施中部署，掌控团队接入与业务数据。', 'shield'],
      ['自有账号集中管理', '连接和管理客户自己拥有、已合法授权的订阅与账号。', 'model'],
      ['成员独立 API Key', '为成员签发独立密钥，无需分发共享账号的凭据。', 'key'],
      ['模型、额度与审计', '控制模型开放范围，分配额度，统计并审计团队用量。', 'chart'],
      ['客户控制的 Runner', '通过客户控制的执行节点，转发受支持的模型请求。', 'server'],
      ['兼容接口与工具接入', '提供 OpenAI、Anthropic 兼容接口，支持 Codex、Claude Code 接入。', 'gateway'],
    ],
    roles: [
      { title: '管理员统一管理', body: '维护订阅/账号与 Runner，控制模型开放范围，为成员分配额度，查看团队请求、消耗与操作记录。' },
      { title: '成员独立使用', body: '在自己的工作台查看可用额度、创建并管理 API Key、获取接入地址与使用说明，查看个人用量，按需申请更多额度。' },
    ],
  },
  deployment: {
    eyebrow: '私有化交付', title: '软件和数据留在自己的环境',
    body: 'Aster 以部署包和 License 交付。成员、Key、额度和使用记录保存在客户环境中，模型请求仍会按配置发送到对应上游服务。',
    nodes: ['用户入口', 'Aster Control', 'Runner', '上游服务'],
    cards: [
      ['部署包', '提供适用于约定环境的软件包'],
      ['离线 License', '授权校验不依赖持续连接运营平台'],
      ['安装资料', '提供安装、配置和常见问题说明'],
      ['支持服务', '按约定处理配置问题和产品缺陷'],
    ],
    steps: ['确认环境', '交付部署包', '安装授权', '开始使用'],
  },
  trial: {
    eyebrow: '企业咨询', title: '为团队部署 Aster，需要帮助？',
    body: '分享团队规模和使用情况，与我们讨论部署条件、产品接入和适合的授权方案。',
  },
  footer: { owner: 'HUZZ · 弧之舟（HuZhiZhou）旗下产品', back: '回到顶部' },
} : {
  nav: ['Product demo', 'Capabilities', 'Private deployment', 'Download & pricing'],
  hero: {
    eyebrow: 'A privately deployed AI gateway for teams',
    title: ['Empower your team with AI.', 'Keep every investment in control.'],
    body: 'Connect subscription accounts, assign member keys and quotas, and track usage for every request.',
    benefit: 'Make better use of resources. Understand your team’s AI costs.',
    primary: 'Download free', secondary: 'Explore the product',
  },
  demo: {
    eyebrow: 'INTERACTIVE PRODUCT TOUR', title: 'See how teams use Aster',
    body: 'Explore the daily workflow: administrators allocate resources, while members manage their keys and review usage.',
  },
  capabilities: {
    eyebrow: 'WHAT ASTER PROVIDES', title: 'One place for team AI access and control',
    body: 'Connect accounts your organization owns and is authorized to use. Give members independent access and administrators visibility and control.',
    items: [
      ['A privately deployed team gateway', 'Run on infrastructure you manage, with control over team access and business data.', 'shield'],
      ['Your accounts, centrally managed', 'Connect and manage subscriptions and accounts your organization owns and is authorized to use.', 'model'],
      ['Independent member API keys', 'Issue individual keys without distributing shared account credentials.', 'key'],
      ['Models, quotas and usage audit', 'Control model availability, allocate quota, and track and audit team usage.', 'chart'],
      ['Runners under your control', 'Forward supported model requests through execution nodes you control.', 'server'],
      ['Compatible APIs and familiar tools', 'Use OpenAI- and Anthropic-compatible APIs, with Codex and Claude Code integrations.', 'gateway'],
    ],
    roles: [
      { title: 'Central control for administrators', body: 'Manage subscription accounts and Runners, control model availability, allocate member quotas, and review team requests, consumption and activity records.' },
      { title: 'Independent access for members', body: 'Check available quota, create and manage API keys, find connection details and guides, review personal usage, and request more quota from your own workspace.' },
    ],
  },
  deployment: {
    eyebrow: 'PRIVATE DELIVERY', title: 'Keep the software and records in your environment',
    body: 'Aster is delivered as a deployment package and license. Members, keys, quota and usage records stay in the customer environment while model requests go to the configured upstream service.',
    nodes: ['User access', 'Aster Control', 'Runner', 'Upstream service'],
    cards: [
      ['Deployment package', 'Software for the agreed environment'],
      ['Offline license', 'Authorization does not require a continuous Operations connection'],
      ['Installation guide', 'Installation, configuration and common issue guidance'],
      ['Support', 'Configuration and product defects handled as agreed'],
    ],
    steps: ['Confirm environment', 'Receive package', 'Install license', 'Start using'],
  },
  trial: {
    eyebrow: 'ENTERPRISE SUPPORT', title: 'Need help deploying Aster for your team?',
    body: 'Share your team size and usage to discuss deployment, product integration and licensing.',
  },
  footer: { owner: 'A product by HUZZ (HuZhiZhou)', back: 'Back to top' },
})
const normalizedLeadTeamSize = computed(() => Math.min(500, Math.max(1, Number(leadForm.value.teamSize) || 1)))
const normalizedLeadActiveUsers = computed(() => Math.min(normalizedLeadTeamSize.value, Math.max(1, Number(leadForm.value.activeUsers) || 1)))
const normalizedWeeklyTokens = computed(() => Math.min(100000, Math.max(0.1, Number(leadForm.value.weeklyTokens) || 0.1)))
const leadUsageOptions = computed<readonly (readonly [string, string])[]>(() => t.value.lead.timeOptions)
const selectedLeadUsageValue = computed(() => leadForm.value.dailyTime)
const selectedLeadUsageLabel = computed(() => leadUsageOptions.value.find(option => option[0] === selectedLeadUsageValue.value)?.[1] ?? '')
const leadFallbackSubject = computed(() => {
  const organization = leadForm.value.company.trim() || (locale.value === 'zh' ? '团队' : 'Team')
  return locale.value === 'zh'
    ? `Aster Team 企业试用申请｜${organization}｜${normalizedLeadTeamSize.value} 人`
    : `Aster Team trial request | ${organization} | ${normalizedLeadTeamSize.value} members`
})
const leadFallbackBody = computed(() => {
  const organization = leadForm.value.company.trim() || (locale.value === 'zh' ? '未填写' : 'Not provided')
  const evidenceDetail = leadForm.value.evidence === 'tokens'
    ? (locale.value === 'zh' ? `${normalizedWeeklyTokens.value} 亿 Token／近 7 天` : `${normalizedWeeklyTokens.value} × 100M tokens in the last 7 days`)
    : selectedLeadUsageLabel.value
  if (locale.value === 'zh') {
    return [
      '您好，Aster Team 团队：',
      '',
      '官网试用申请暂时未能提交，以下是我们的团队情况，希望申请 Aster Team 产品试用：',
      '',
      `团队 / 企业名称：${organization}`,
      `联系方式：${leadForm.value.contact.trim()}`,
      `团队成员：${normalizedLeadTeamSize.value} 人`,
      `经常使用 AI：${normalizedLeadActiveUsers.value} 人`,
      `评估依据：${leadForm.value.evidence === 'tokens' ? '近 7 天 Token 用量' : '平均每天使用 AI 的时间'}`,
      `使用情况：${evidenceDetail}`,
      '',
      '希望进一步了解产品试用安排、部署方式，以及适合团队的授权方案。',
      '',
      '谢谢。',
    ].join('\n')
  }
  return [
    'Hello Aster Team,',
    '',
    'The website could not submit our request. Here are our team details for an Aster Team product trial:',
    '',
    `Team / company: ${organization}`,
    `Contact details: ${leadForm.value.contact.trim()}`,
    `Team members: ${normalizedLeadTeamSize.value}`,
    `Regular AI users: ${normalizedLeadActiveUsers.value}`,
    `Assessment basis: ${leadForm.value.evidence === 'tokens' ? 'Tokens in the last 7 days' : 'Average daily AI usage time'}`,
    `Usage: ${evidenceDetail}`,
    '',
    'We would like to learn about the product trial, deployment options, and suitable licensing for our team.',
    '',
    'Thank you.',
  ].join('\n')
})
const leadFallbackCompleteEmail = computed(() => locale.value === 'zh'
  ? `收件人：${email}\n主题：${leadFallbackSubject.value}\n\n${leadFallbackBody.value}`
  : `To: ${email}\nSubject: ${leadFallbackSubject.value}\n\n${leadFallbackBody.value}`)

function setLocale(value: Locale) {
  locale.value = value
  mobileNavOpen.value = false
}

const sectionScrollEasing = (progress: number) => 1 - Math.pow(1 - progress, 4)

function scrollToSection(index: number) {
  const section = document.getElementById(`section-${index + 1}`)
  if (section) {
    // Align visible content, not the section's decorative top padding.
    const content = section.querySelector<HTMLElement>('.section-heading, .deployment-copy, .catalog-heading') ?? section.querySelector<HTMLElement>('.section-inner') ?? section
    const headerHeight = document.querySelector('.site-header')?.getBoundingClientRect().height ?? 0
    const top = index === 0 ? 0 : Math.max(0, window.scrollY + content.getBoundingClientRect().top - headerHeight - 24)
    if (lenis.value) {
      // The first click can precede Lenis's debounced ResizeObserver update.
      // Refresh dimensions before clamping a target in the long pricing section.
      lenis.value.resize()
      lenis.value.scrollTo(top, {
        duration: 0.72,
        easing: sectionScrollEasing,
      })
    } else window.scrollTo({ top, behavior: window.matchMedia('(prefers-reduced-motion: reduce)').matches ? 'instant' : 'smooth' })
  }
  mobileNavOpen.value = false
}

function syncActiveSection() {
  const marker = window.innerHeight * 0.22
  let current = 0
  document.querySelectorAll<HTMLElement>('.snap-section, .catalog-section').forEach(section => {
    const rect = section.getBoundingClientRect()
    // Keep navigation tied to stable section IDs when optional sections are removed.
    if (rect.top <= marker) current = Number(section.id.replace('section-', '')) - 1
  })
  activeSection.value = Math.min(8, Math.max(0, current))
}

const lenis = useLenis(syncActiveSection)
let leadFallbackCopyTimer = 0

function syncOverlayBodyLock() {
  document.body.classList.toggle('lightbox-open', Boolean(leadSuccessOpen.value || leadFallbackOpen.value || turnstileDialogOpen.value))
}

function openLeadSuccess(id: string) {
  closeLeadUsage()
  leadSuccessId.value = id
  leadSuccessOpen.value = true
  syncOverlayBodyLock()
  void nextTick(() => leadSuccessCloseButton.value?.focus())
}

function closeLeadSuccess() {
  if (!leadSuccessOpen.value) return
  leadSuccessOpen.value = false
  leadSubmitState.value = 'idle'
  syncOverlayBodyLock()
  void nextTick(() => leadSubmitButton.value?.focus())
}

function openLeadFallback() {
  closeLeadUsage()
  leadFallbackCopyState.value = 'idle'
  leadFallbackOpen.value = true
  syncOverlayBodyLock()
  void nextTick(() => leadFallbackCloseButton.value?.focus())
}

function closeLeadFallback() {
  if (!leadFallbackOpen.value) return
  leadFallbackOpen.value = false
  leadFallbackCopyState.value = 'idle'
  if (leadSubmitState.value === 'failed' || leadSubmitState.value === 'not-configured') leadSubmitState.value = 'idle'
  syncOverlayBodyLock()
  void nextTick(() => leadSubmitButton.value?.focus())
}

function openTurnstileDialog() {
  if (!turnstileSiteKey) {
    leadSubmitState.value = 'verification-required'
    return
  }
  turnstileToken.value = ''
  leadSubmitState.value = 'verification-required'
  turnstileDialogOpen.value = true
  syncOverlayBodyLock()
  void nextTick(() => turnstileDialogPanel.value?.focus())
}

function handleTurnstileError() {
  turnstileToken.value = ''
  leadSubmitState.value = 'verification-required'
}

function handleTurnstileToken(token: string) {
  turnstileToken.value = token
  if (!token || !turnstileDialogOpen.value) return
  turnstileDialogOpen.value = false
  syncOverlayBodyLock()
  void nextTick(() => submitLead())
}

async function copyLeadFallback(kind: 'email' | 'content') {
  const value = kind === 'email' ? email : leadFallbackCompleteEmail.value
  try {
    await navigator.clipboard.writeText(value)
  } catch {
    const field = document.createElement('textarea')
    field.value = value
    field.style.position = 'fixed'
    field.style.opacity = '0'
    document.body.appendChild(field)
    field.select()
    document.execCommand('copy')
    field.remove()
  }
  leadFallbackCopyState.value = kind
  window.clearTimeout(leadFallbackCopyTimer)
  leadFallbackCopyTimer = window.setTimeout(() => { leadFallbackCopyState.value = 'idle' }, 2200)
}

function openLeadFallbackMail() {
  window.location.href = `mailto:${email}?subject=${encodeURIComponent(leadFallbackSubject.value)}&body=${encodeURIComponent(leadFallbackBody.value)}`
}

function closeLeadUsage(restoreFocus = false) {
  leadUsageOpen.value = false
  if (restoreFocus) void nextTick(() => leadUsageButton.value?.focus())
}

function openLeadUsage() {
  const selectedIndex = leadUsageOptions.value.findIndex(option => option[0] === selectedLeadUsageValue.value)
  leadUsageActiveIndex.value = Math.max(0, selectedIndex)
  leadUsageOpen.value = true
}

function toggleLeadUsage() {
  if (leadUsageOpen.value) closeLeadUsage()
  else openLeadUsage()
}

function selectLeadUsage(value: string) {
  leadForm.value.dailyTime = value as DailyAiTime
  closeLeadUsage(true)
}

function handleLeadUsageKeydown(event: KeyboardEvent) {
  const lastIndex = leadUsageOptions.value.length - 1
  if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
    event.preventDefault()
    if (!leadUsageOpen.value) openLeadUsage()
    else {
      const direction = event.key === 'ArrowDown' ? 1 : -1
      leadUsageActiveIndex.value = (leadUsageActiveIndex.value + direction + leadUsageOptions.value.length) % leadUsageOptions.value.length
    }
  } else if (event.key === 'Home' || event.key === 'End') {
    event.preventDefault()
    if (!leadUsageOpen.value) openLeadUsage()
    leadUsageActiveIndex.value = event.key === 'Home' ? 0 : lastIndex
  } else if (event.key === 'Enter' || event.key === ' ') {
    event.preventDefault()
    if (!leadUsageOpen.value) openLeadUsage()
    else selectLeadUsage(leadUsageOptions.value[leadUsageActiveIndex.value][0])
  } else if (event.key === 'Escape' && leadUsageOpen.value) {
    event.preventDefault()
    closeLeadUsage(true)
  } else if (event.key === 'Tab') {
    closeLeadUsage()
  }
}

function handleLeadUsagePointerDown(event: PointerEvent) {
  if (leadUsageOpen.value && event.target instanceof Node && !leadUsageRoot.value?.contains(event.target)) closeLeadUsage()
}

function handleGlobalKeydown(event: KeyboardEvent) {
  if (event.key === 'Escape' && turnstileDialogOpen.value) {
    event.preventDefault()
    event.stopImmediatePropagation()
    return
  }
  if (event.key === 'Escape' && leadSuccessOpen.value) {
    event.preventDefault()
    closeLeadSuccess()
    return
  }
  if (event.key === 'Escape' && leadFallbackOpen.value) {
    event.preventDefault()
    closeLeadFallback()
    return
  }
  if (event.key === 'Escape' && leadUsageOpen.value) {
    event.preventDefault()
    closeLeadUsage(true)
    return
  }
}

function saveLeadDraft(showFeedback = false) {
  try {
    localStorage.setItem(leadDraftCacheKey, JSON.stringify(leadForm.value))
    if (showFeedback) {
      leadSubmitState.value = 'saved'
      window.setTimeout(() => {
        if (leadSubmitState.value === 'saved') leadSubmitState.value = 'idle'
      }, 2200)
    }
  } catch {
    if (showFeedback) leadSubmitState.value = 'failed'
  }
}

function restoreLeadDraft() {
  try {
    const saved = JSON.parse(localStorage.getItem(leadDraftCacheKey) || 'null') as Partial<LeadForm> | null
    if (!saved) return
    leadForm.value = { ...leadForm.value, ...saved }
    if (!['under1h', '1h-4h', '4h-8h', 'over8h'].includes(String(leadForm.value.dailyTime))) leadForm.value.dailyTime = '1h-4h'
    if (!Number.isFinite(Number(leadForm.value.weeklyTokens)) || Number(leadForm.value.weeklyTokens) <= 0) leadForm.value.weeklyTokens = 1
  } catch { /* ignore invalid local drafts */ }
}

async function submitLead() {
  closeLeadUsage()
  leadForm.value.teamSize = normalizedLeadTeamSize.value
  leadForm.value.activeUsers = normalizedLeadActiveUsers.value
  leadForm.value.weeklyTokens = normalizedWeeklyTokens.value
  saveLeadDraft()
  if (!leadEndpoint) {
    leadSubmitState.value = 'not-configured'
    openLeadFallback()
    return
  }
  if (!turnstileToken.value) {
    openTurnstileDialog()
    return
  }
  leadSubmitState.value = 'sending'
  try {
    const response = await fetch(leadEndpoint, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        ...leadForm.value,
        locale: locale.value,
        source: 'aster-team-website',
        submittedAt: new Date().toISOString(),
        turnstileToken: turnstileToken.value,
        website: leadHoneypot.value,
      }),
    })
    if (!response.ok) throw new Error(`lead submission failed: ${response.status}`)
    const result = await response.json() as { id?: unknown }
    if (typeof result.id !== 'string' || result.id.length === 0 || result.id.length > 100) throw new Error('lead submission returned an invalid id')
    leadSubmitState.value = 'sent'
    localStorage.removeItem(leadDraftCacheKey)
    openLeadSuccess(result.id)
  } catch {
    leadSubmitState.value = 'failed'
    openLeadFallback()
  } finally {
    turnstileWidget.value?.reset()
    turnstileToken.value = ''
  }
}

const leadStatusMessage = computed(() => {
  if (leadSubmitState.value === 'verification-required') return t.value.lead.verificationRequired
  return ''
})

watch(locale, value => { document.documentElement.lang = value === 'zh' ? 'zh-CN' : 'en' }, { immediate: true })
watch(() => leadForm.value.evidence, () => closeLeadUsage())
watch(leadForm, () => {
  if (leadForm.value.activeUsers > normalizedLeadTeamSize.value) leadForm.value.activeUsers = normalizedLeadTeamSize.value
  saveLeadDraft()
}, { deep: true })
onMounted(() => {
  restoreLeadDraft()
  syncActiveSection()
  window.addEventListener('scroll', syncActiveSection, { passive: true })
  window.addEventListener('keydown', handleGlobalKeydown)
  window.addEventListener('pointerdown', handleLeadUsagePointerDown)
})
onUnmounted(() => {
  window.removeEventListener('scroll', syncActiveSection)
  window.removeEventListener('keydown', handleGlobalKeydown)
  window.removeEventListener('pointerdown', handleLeadUsagePointerDown)
  document.body.classList.remove('lightbox-open')
  window.clearTimeout(leadFallbackCopyTimer)
})
</script>

<template>
  <header class="site-header" @keydown.esc="mobileNavOpen = false">
    <button class="brand" type="button" aria-label="Aster Team" @click="scrollToSection(0)"><img src="/logo-mark.svg" width="31" height="31" alt=""><strong>Aster</strong><span>Team</span></button>
    <nav :class="{ open: mobileNavOpen }" :aria-label="locale === 'zh' ? '页面导航' : 'Page navigation'">
      <button type="button" :class="{ active: activeSection === 1 }" @click="scrollToSection(1)">{{ pageCopy.nav[0] }}</button>
      <button type="button" :class="{ active: activeSection === 2 }" @click="scrollToSection(2)">{{ pageCopy.nav[1] }}</button>
      <button type="button" :class="{ active: activeSection === 3 }" @click="scrollToSection(3)">{{ pageCopy.nav[2] }}</button>
      <button type="button" :class="{ active: activeSection === 8 }" @click="scrollToSection(8)">{{ pageCopy.nav[3] }}</button>
    </nav>
    <div class="header-tools">
      <a class="header-docs" :href="locale === 'zh' ? '/docs/zh-cn/' : '/docs/en/'" target="_blank" rel="noopener noreferrer">{{ locale === 'zh' ? '文档' : 'Documentation' }}</a>
      <a class="header-github" :href="publicRepositoryUrl" target="_blank" rel="noopener noreferrer" :aria-label="locale === 'zh' ? '打开 Aster Team GitHub 仓库' : 'Open the Aster Team GitHub repository'" :title="locale === 'zh' ? 'GitHub 仓库' : 'GitHub repository'"><SiteIcon name="github" :size="24" /></a>
      <div class="language-switch" role="group" :aria-label="locale === 'zh' ? '选择语言' : 'Choose language'">
        <button type="button" lang="zh-CN" :class="{ active: locale === 'zh' }" :aria-pressed="locale === 'zh'" aria-label="切换为简体中文" @click="setLocale('zh')">中文</button>
        <button type="button" lang="en" :class="{ active: locale === 'en' }" :aria-pressed="locale === 'en'" aria-label="Switch to English" @click="setLocale('en')">EN</button>
      </div>
      <button class="primary-button header-download" type="button" @click="scrollToSection(8)">{{ pageCopy.hero.primary }}</button>
      <button class="menu-toggle" type="button" :aria-expanded="mobileNavOpen" :aria-label="locale === 'zh' ? '打开导航' : 'Open navigation'" @click="mobileNavOpen = !mobileNavOpen"><span></span><span></span></button>
    </div>
  </header>

  <main class="site-scroll">
    <section id="section-1" class="snap-section hero-section" :class="{ 'is-revealed': activeSection === 0 }">
      <div class="section-inner hero-layout">
        <div class="hero-copy">
          <span class="section-kicker">{{ pageCopy.hero.eyebrow }}</span>
          <h1><span>{{ pageCopy.hero.title[0] }}</span><em>{{ pageCopy.hero.title[1] }}</em></h1>
          <p>{{ pageCopy.hero.body }}<span class="hero-benefit">{{ pageCopy.hero.benefit }}</span></p>
          <div class="hero-actions"><button class="primary-button" type="button" @click="scrollToSection(8)">{{ pageCopy.hero.primary }}<SiteIcon name="download" :size="18" /></button><button class="secondary-button" type="button" @click="scrollToSection(1)">{{ pageCopy.hero.secondary }}<SiteIcon name="chevron-right" :size="18" /></button></div>
        </div>
        <div class="hero-system"><HeroGateway :locale="locale" /></div>
        <FreeStart :locale="locale" @explore="scrollToSection(8)" />
      </div>
    </section>

    <section id="section-2" class="snap-section demo-section" :class="{ 'is-revealed': activeSection === 1 }">
      <div class="section-inner">
        <header class="section-heading demo-heading"><div><span class="section-kicker">{{ pageCopy.demo.eyebrow }}</span><h2>{{ pageCopy.demo.title }}</h2></div><p>{{ pageCopy.demo.body }}</p></header>
        <div class="demo-frame"><ProductDemo :locale="locale" /></div>
        <ClientConnections :locale="locale" />
      </div>
    </section>

    <section id="section-3" class="snap-section capability-section" :class="{ 'is-revealed': activeSection === 2 }">
      <div class="section-inner">
        <header class="section-heading"><span class="section-kicker">{{ pageCopy.capabilities.eyebrow }}</span><h2>{{ pageCopy.capabilities.title }}</h2><p>{{ pageCopy.capabilities.body }}</p></header>
        <div class="capability-list"><article v-for="(item,index) in pageCopy.capabilities.items" :key="item[0]"><span>0{{ index + 1 }}</span><i><SiteIcon :name="index === 0 ? 'shield' : index === 1 ? 'model' : index === 2 ? 'key' : index === 3 ? 'chart' : index === 4 ? 'server' : 'gateway'" :size="25" /></i><div><h3>{{ item[0] }}</h3><p>{{ item[1] }}</p></div></article></div>
        <div class="capability-roles">
          <article v-for="role in pageCopy.capabilities.roles" :key="role.title"><h3>{{ role.title }}</h3><p>{{ role.body }}</p></article>
        </div>
      </div>
    </section>

    <section id="section-4" class="snap-section deployment-section" :class="{ 'is-revealed': activeSection === 3 }">
      <div class="section-inner deployment-layout">
        <div class="deployment-copy"><span class="section-kicker">{{ pageCopy.deployment.eyebrow }}</span><h2>{{ pageCopy.deployment.title }}</h2><p>{{ pageCopy.deployment.body }}</p><div class="delivery-cards"><article v-for="(card,index) in pageCopy.deployment.cards" :key="card[0]"><i><SiteIcon :name="index === 0 ? 'file' : index === 1 ? 'shield' : index === 2 ? 'copy' : 'settings'" :size="21" /></i><div><h3>{{ card[0] }}</h3><p>{{ card[1] }}</p></div></article></div></div>
        <PrivateDeploymentIllustration :labels="t.private" />
      </div>
      <div class="section-inner delivery-steps"><article v-for="(step,index) in pageCopy.deployment.steps" :key="step"><b>0{{ index + 1 }}</b><strong>{{ step }}</strong><i v-if="index < pageCopy.deployment.steps.length - 1"><SiteIcon name="arrow" :size="17" /></i></article></div>
    </section>

    <section id="section-6" class="snap-section trial-section" :class="{ 'is-revealed': activeSection === 5 }">
      <div class="section-inner trial-layout">
        <div class="trial-copy"><span class="section-kicker">{{ pageCopy.trial.eyebrow }}</span><h2>{{ pageCopy.trial.title }}</h2><p>{{ pageCopy.trial.body }}</p><ContactLinks :locale="locale" :email="email" /><div class="trial-assurance"><SiteIcon name="shield" :size="18" /><span>{{ t.lead.privacy }}</span></div></div>
        <form class="trial-panel lead-form" @submit.prevent="submitLead">
          <label class="lead-honeypot" aria-hidden="true">Website<input v-model="leadHoneypot" type="text" tabindex="-1" autocomplete="off"></label>
          <div class="trial-panel-heading"><div><h3>{{ t.lead.title }}</h3><p>{{ t.lead.subtitle }}</p></div><span class="lead-draft-state"><SiteIcon name="check" :size="14" />{{ t.lead.autoSaved }}</span></div>
          <div class="lead-form-grid"><label class="lead-field"><span>{{ t.lead.contact }} <b>*</b></span><input v-model.trim="leadForm.contact" required autocomplete="email tel" :placeholder="t.lead.contactPlaceholder"></label><label class="lead-field"><span>{{ t.lead.company }}</span><input v-model.trim="leadForm.company" autocomplete="organization" :placeholder="t.lead.companyPlaceholder"></label><label class="lead-field lead-number-field"><span>{{ t.lead.teamSize }}</span><div><input v-model.number="leadForm.teamSize" type="number" min="1" max="500" inputmode="numeric"><em>{{ t.lead.people }}</em></div></label><label class="lead-field lead-number-field"><span>{{ t.lead.activeUsers }}</span><div><input v-model.number="leadForm.activeUsers" type="number" min="1" :max="normalizedLeadTeamSize" inputmode="numeric"><em>{{ t.lead.people }}</em></div></label></div>
          <fieldset class="usage-evidence"><legend>{{ t.lead.evidence }}</legend><div class="lead-segmented"><label :class="{ active: leadForm.evidence === 'tokens' }"><input v-model="leadForm.evidence" type="radio" value="tokens"><SiteIcon name="database" :size="17" />{{ t.lead.hasTokens }}</label><label :class="{ active: leadForm.evidence === 'time' }"><input v-model="leadForm.evidence" type="radio" value="time"><SiteIcon name="clock" :size="17" />{{ t.lead.noTokens }}</label></div></fieldset>
          <label v-if="leadForm.evidence === 'tokens'" class="lead-field lead-usage-field"><span>{{ t.lead.weeklyTokens }}</span><div class="lead-token-field"><input v-model.number="leadForm.weeklyTokens" type="number" min="0.1" max="100000" step="0.1" inputmode="decimal"><em>{{ t.lead.tokenUnit }}</em></div></label>
          <div v-else class="lead-field lead-usage-field"><span id="lead-usage-label">{{ t.lead.dailyTime }}</span><div ref="leadUsageRoot" class="lead-select" :class="{ 'is-open': leadUsageOpen }"><button ref="leadUsageButton" class="lead-select__trigger" type="button" role="combobox" aria-haspopup="listbox" aria-labelledby="lead-usage-label" :aria-expanded="leadUsageOpen" aria-controls="lead-usage-options" :aria-activedescendant="leadUsageOpen ? `lead-usage-option-${leadUsageActiveIndex}` : undefined" @click="toggleLeadUsage" @keydown="handleLeadUsageKeydown"><span>{{ selectedLeadUsageLabel }}</span><SiteIcon name="chevron-down" :size="17" /></button><Transition name="lead-select"><div v-if="leadUsageOpen" id="lead-usage-options" class="lead-select__menu" role="listbox" aria-labelledby="lead-usage-label"><div v-for="(option,index) in leadUsageOptions" :id="`lead-usage-option-${index}`" :key="option[0]" class="lead-select__option" :class="{ 'is-active': index === leadUsageActiveIndex, 'is-selected': option[0] === selectedLeadUsageValue }" role="option" :aria-selected="option[0] === selectedLeadUsageValue" @mouseenter="leadUsageActiveIndex = index" @click="selectLeadUsage(option[0])"><span>{{ option[1] }}</span><SiteIcon v-if="option[0] === selectedLeadUsageValue" name="check" :size="17" /></div></div></Transition></div></div>
          <div class="trial-actions"><button class="secondary-button" type="button" @click="saveLeadDraft(true)"><SiteIcon :name="leadSubmitState === 'saved' ? 'check' : 'file'" :size="18" />{{ leadSubmitState === 'saved' ? t.lead.saved : t.lead.save }}</button><button ref="leadSubmitButton" class="primary-button" type="submit" :disabled="leadSubmitState === 'sending'"><span>{{ leadSubmitState === 'sending' ? t.lead.sending : t.lead.submit }}</span><SiteIcon name="send" :size="18" /></button></div>
          <p v-if="leadStatusMessage" class="lead-status" :class="`is-${leadSubmitState}`" role="status">{{ leadStatusMessage }}</p>
        </form>
      </div>
    </section>

    <ProductPlans :locale="locale" :sitekey="turnstileSiteKey" :email="email" />
  </main>

  <footer class="site-footer"><div class="site-footer__inner"><span class="footer-signature"><img src="/logo-mark.svg" width="28" height="28" alt=""><strong>Aster Team</strong><small>{{ pageCopy.footer.owner }}</small></span><button class="back-to-top" type="button" @click="scrollToSection(0)">{{ pageCopy.footer.back }}<SiteIcon name="arrow" :size="16" /></button></div></footer>

  <Teleport to="body">
    <Transition name="lead-fallback-dialog"><div v-if="turnstileDialogOpen" class="lead-fallback-dialog turnstile-dialog" role="dialog" aria-modal="true" aria-labelledby="turnstile-dialog-title" @pointerdown.self.prevent @click.self.prevent @keydown.esc.prevent.stop><div ref="turnstileDialogPanel" class="lead-fallback-dialog__panel turnstile-dialog__panel" tabindex="-1"><header class="lead-fallback-dialog__header turnstile-dialog__header"><i><SiteIcon name="shield" :size="27" /></i><div><span>Aster Team</span><h2 id="turnstile-dialog-title">{{ t.lead.verification.title }}</h2><p>{{ t.lead.verification.body }}</p></div></header><TurnstileWidget ref="turnstileWidget" :sitekey="turnstileSiteKey" @token="handleTurnstileToken" @error="handleTurnstileError" /><p class="turnstile-dialog__status" role="status">{{ leadStatusMessage }}</p></div></div></Transition>
    <Transition name="lead-fallback-dialog"><div v-if="leadSuccessOpen" class="lead-fallback-dialog lead-success-dialog" role="dialog" aria-modal="true" aria-labelledby="lead-success-title" @pointerdown.self.prevent @click.self.prevent><div class="lead-fallback-dialog__panel lead-success-dialog__panel"><button class="lead-fallback-dialog__close" type="button" :aria-label="t.lead.success.close" :title="t.lead.success.close" @click="closeLeadSuccess"><SiteIcon name="close" :size="17" /></button><header class="lead-fallback-dialog__header lead-success-dialog__header"><i><SiteIcon name="check" :size="28" /></i><div><span>Aster Team</span><h2 id="lead-success-title">{{ t.lead.success.title }}</h2><p>{{ t.lead.success.body }}</p></div></header><section class="lead-success-dialog__id"><small>{{ t.lead.success.applicationId }}</small><strong>{{ leadSuccessId }}</strong></section><footer class="lead-success-dialog__actions"><button ref="leadSuccessCloseButton" class="primary-button" type="button" @click="closeLeadSuccess"><SiteIcon name="check" :size="19" />{{ t.lead.success.close }}</button></footer></div></div></Transition>
    <Transition name="lead-fallback-dialog"><div v-if="leadFallbackOpen" class="lead-fallback-dialog" role="dialog" aria-modal="true" aria-labelledby="lead-fallback-title" @pointerdown.self.prevent @click.self.prevent><div class="lead-fallback-dialog__panel"><button ref="leadFallbackCloseButton" class="lead-fallback-dialog__close" type="button" :aria-label="t.lead.fallback.close" :title="t.lead.fallback.close" @click="closeLeadFallback"><SiteIcon name="close" :size="17" /></button><header class="lead-fallback-dialog__header"><i><SiteIcon name="send" :size="25" /></i><div><span>Aster Team</span><h2 id="lead-fallback-title">{{ t.lead.fallback.title }}</h2><p>{{ t.lead.fallback.intro }}</p></div></header><div class="lead-fallback-dialog__email"><section class="lead-fallback-dialog__recipient"><div><small>{{ t.lead.fallback.recipient }}</small><strong>{{ email }}</strong></div><button type="button" @click="copyLeadFallback('email')"><SiteIcon :name="leadFallbackCopyState === 'email' ? 'check' : 'copy'" :size="18" /><span>{{ leadFallbackCopyState === 'email' ? t.lead.fallback.copiedEmail : t.lead.fallback.copyEmail }}</span></button></section><section><small>{{ t.lead.fallback.subject }}</small><strong>{{ leadFallbackSubject }}</strong></section><section class="lead-fallback-dialog__body"><small>{{ t.lead.fallback.body }}</small><pre tabindex="0">{{ leadFallbackBody }}</pre></section></div><footer class="lead-fallback-dialog__actions"><button class="secondary-button" type="button" @click="copyLeadFallback('content')"><SiteIcon :name="leadFallbackCopyState === 'content' ? 'check' : 'copy'" :size="19" />{{ leadFallbackCopyState === 'content' ? t.lead.fallback.copiedContent : t.lead.fallback.copyContent }}</button><button class="primary-button" type="button" @click="openLeadFallbackMail"><SiteIcon name="send" :size="19" />{{ t.lead.fallback.openMail }}</button></footer></div></div></Transition>
  </Teleport>
</template>
