<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import catalog from 'virtual:aster-public-catalog'
import release, { downloadConfig } from 'virtual:aster-product-release'
import { parseReleaseCatalog, type ReleaseCatalog } from '../shared/release-catalog'
import type { PublicPlan, InquiryReference } from '../shared/generated/contracts'
import { CAPABILITIES, effectiveFeatures, QUOTAS } from './generated/product-capabilities'
import ProductInquiry from './ProductInquiry.vue'
import LatestDownload from './LatestDownload.vue'
import { fallbackReleaseCatalog } from './release-fallback'

const props = defineProps<{ locale: 'zh' | 'en'; sitekey: string; email: string }>()
const yearsByPlan = ref<Record<string, number>>({})
const inquiry = ref<InstanceType<typeof ProductInquiry> | null>(null)
const sending = ref(false)
const plans = catalog?.plans ?? []
const releaseCatalog = ref<ReleaseCatalog>(fallbackReleaseCatalog)
const catalogLive = ref(false)
const latestDownload = computed(() => downloadConfig ?? releaseCatalog.value.releases[0])
onMounted(async () => {
  try {
    const response = await fetch('/api/releases', { headers: { Accept: 'application/json' } })
    if (!response.ok) throw new Error('Release metadata unavailable')
    const parsed = parseReleaseCatalog(await response.json())
    if (!parsed) throw new Error('Invalid release metadata')
    releaseCatalog.value = parsed
    catalogLive.value = true
  } catch { /* Keep the bundled release when the catalog endpoint is unavailable. */ }
})
const t = computed(() => props.locale === 'zh' ? {
  eyebrow: '下载与安装', title: '一行命令安装 Aster Team',
  free: '免费', contact: '联系报价', year: '年', total: '合计', term: '授权期限', includedTax: '含税', noTax: '税费另行确认',
  inquire: '咨询采购', learn: '了解部署', compare: '完整权益对比', feature: '功能与额度', included: '包含', excluded: '不包含',
  quoted: '按报价确定', unlimited: '不限量', empty: '当前套餐请联系咨询', emptyDetail: '说明你的使用场景与部署需求，我们会与你确认适用方案。',
  minimum: '最低产品版本', noExpiry: '不设到期', expiry: '免费授权到期', swipe: '左右滚动查看全部版本',
  releaseEyebrow: '为你的团队而建', releaseTitle: '从免费开始，随团队成长',
  releaseBody: '用免费版连接订阅与账号，为成员分配独立密钥，掌握团队用量。需要更多容量时，直接升级，无需重新部署。',
  releaseLocal: '本地验证包', releaseProduction: '正式发行', releaseVersion: '版本', releasePlatform: '平台', releaseSize: '大小', releaseDigest: 'SHA-256',
  download: '下载安装包', manual: '查看用户手册', linux: 'Linux 安装说明', checksums: '校验文件',
} : {
  eyebrow: 'DOWNLOAD & INSTALL', title: 'Install Aster Team in one command',
  free: 'Free', contact: 'Contact for pricing', year: 'year', total: 'Total', term: 'License term', includedTax: 'Tax included', noTax: 'Confirm taxes with us',
  inquire: 'Discuss this plan', learn: 'Explore deployment', compare: 'Compare all entitlements', feature: 'Features and limits', included: 'Included', excluded: 'Not included',
  quoted: 'Agreed in quotation', unlimited: 'Unlimited', empty: 'Contact us about available plans', emptyDetail: 'Tell us about your use case and deployment needs to discuss a suitable plan.',
  minimum: 'Minimum product version', noExpiry: 'No expiry', expiry: 'Free license expires', swipe: 'Scroll horizontally to view every plan',
  releaseEyebrow: 'BUILT FOR YOUR TEAM', releaseTitle: 'Start free. Grow with your team.',
  releaseBody: 'Connect subscriptions and accounts, give members their own keys, and understand team usage with the free plan. Upgrade when you need more capacity, without redeploying.',
  releaseLocal: 'Local validation build', releaseProduction: 'Production release', releaseVersion: 'Version', releasePlatform: 'Platform', releaseSize: 'Size', releaseDigest: 'SHA-256',
  download: 'Download package', manual: 'User manual', linux: 'Linux installation', checksums: 'Checksums',
})
const english: Record<string, string> = { gateway: 'Model access', member: 'Member collaboration', runner: 'Runner execution', member_seats: 'Member seats', runners: 'Runners', upstream_accounts: 'Subscriptions & accounts', api_keys_per_member: 'Keys per member' }
function label(entry: { id: string; label: string }) { return props.locale === 'en' ? english[entry.id] ?? entry.label : entry.label }
function money(amount: number, currency: string) { return new Intl.NumberFormat(props.locale === 'zh' ? 'zh-CN' : 'en-US', { style: 'currency', currency, minimumFractionDigits: 0, maximumFractionDigits: 2 }).format(amount / 100) }
function termLabel(years: number) { return props.locale === 'zh' ? `${years} 年` : `${years} ${years === 1 ? 'year' : 'years'}` }
function selectedTerm(plan: PublicPlan) {
  if (plan.offer.kind !== 'fixed_price') return null
  return plan.offer.terms.find(term => term.years === yearsByPlan.value[plan.plan_id]) ?? plan.offer.terms[0]
}
function price(plan: PublicPlan) {
  if (plan.offer.kind === 'free') return t.value.free
  if (plan.offer.kind === 'contact') return t.value.contact
  return money(selectedTerm(plan)!.total_amount_minor, plan.offer.currency)
}
function quota(plan: PublicPlan, id: string) {
  if (plan.offer.kind === 'contact' && id === 'member_seats') return t.value.quoted
  const grant = plan.entitlements.quotas.find(item => item.id === id)!
  return grant.limit.mode === 'unlimited' ? t.value.unlimited : new Intl.NumberFormat(props.locale).format(grant.limit.value)
}
function expiry(plan: PublicPlan) {
  if (plan.offer.kind !== 'free') return ''
  if (plan.offer.expiry.mode === 'none') return t.value.noExpiry
  const date = new Intl.DateTimeFormat(props.locale === 'zh' ? 'zh-CN' : 'en-US', { dateStyle: 'medium', timeStyle: 'short', timeZone: 'UTC' }).format(new Date(plan.offer.expiry.expires_at))
  return `${t.value.expiry} ${date} UTC`
}
function isDownloadPlan(plan: PublicPlan) {
  return plan.offer.kind === 'free' && release?.free_plan.plan_id === plan.plan_id && release.free_plan.plan_version === plan.version
}
function fileSize(bytes: number) {
  return `${new Intl.NumberFormat(props.locale === 'zh' ? 'zh-CN' : 'en-US', { maximumFractionDigits: 1 }).format(bytes / 1024 / 1024)} MiB`
}
function choose(plan: PublicPlan) {
  if (!catalog || sending.value) return
  const reference: InquiryReference = { catalog_revision: catalog.revision, plan_id: plan.plan_id, plan_version: plan.version }
  const term = selectedTerm(plan)
  if (term) reference.years = term.years
  void inquiry.value?.choose({ label: term ? `${plan.name} · ${termLabel(term.years)}` : plan.name, reference })
}
</script>

<template>
  <section id="section-9" class="catalog-section" :data-catalog-revision="catalog?.revision">
    <div class="catalog-inner">
      <header class="catalog-heading"><span>{{ t.eyebrow }}</span><h2>{{ t.title }}</h2></header>
      <LatestDownload :locale="locale" :catalog="releaseCatalog" :catalog-live="catalogLive" />
      <section v-if="!release" class="release-pending" aria-labelledby="release-pending-title">
        <div><span>{{ t.releaseEyebrow }}</span><h3 id="release-pending-title">{{ t.releaseTitle }}</h3><p>{{ t.releaseBody }}</p></div>
        <div class="release-actions"><a v-if="latestDownload" class="release-primary latest-download-button" :href="latestDownload.url" :download="downloadConfig?.filename ?? undefined">{{ locale === 'zh' ? '下载 Linux 安装包' : 'Download Linux package' }}</a><a href="https://github.com/huzz-open/aster-team/releases" target="_blank" rel="noopener noreferrer">{{ locale === 'zh' ? '查看 GitHub 发布页' : 'View GitHub releases' }}</a><a href="https://github.com/huzz-open/aster-team/blob/main/docs/guides/install-aster-team-linux.md" target="_blank" rel="noopener noreferrer">{{ locale === 'zh' ? 'Linux 安装文档' : 'Linux installation guide' }}</a></div>
      </section>
      <section v-if="release" class="release-card" :data-release-environment="release.environment" :data-release-version="release.version">
        <div class="release-copy"><span>{{ t.releaseEyebrow }}</span><h3>{{ t.releaseTitle }}</h3><p>{{ t.releaseBody }}</p><b>{{ release.environment === 'local' ? t.releaseLocal : t.releaseProduction }}</b></div>
        <dl class="release-facts"><div><dt>{{ t.releaseVersion }}</dt><dd>{{ release.version }}</dd></div><div><dt>{{ t.releasePlatform }}</dt><dd>Linux amd64</dd></div><div><dt>{{ t.releaseSize }}</dt><dd>{{ fileSize(release.artifact.size_bytes) }}</dd></div><div class="release-digest"><dt>{{ t.releaseDigest }}</dt><dd>{{ release.artifact.sha256 }}</dd></div></dl>
        <div class="release-actions"><a class="release-primary" :href="release.artifact.url" :download="release.environment === 'local' ? release.artifact.name : undefined">{{ t.download }}</a><a :href="release.documents.user_manual">{{ t.manual }}</a><a :href="release.documents.linux_guide">{{ t.linux }}</a><a :href="release.documents.checksums">{{ t.checksums }}</a></div>
      </section>
      <section v-if="release?.windows" class="release-card windows-release-card" data-release-channel="experimental" :data-release-version="release.windows.version">
        <div class="release-copy"><span>Windows amd64</span><h3>{{ locale === 'zh' ? 'Windows 实验版' : 'Windows experimental' }}</h3><p>{{ locale === 'zh' ? '不承诺稳定 缺陷修复可能较慢 正式使用建议选择 Linux' : 'Stability is not guaranteed and fixes may take longer. Linux is recommended for production.' }}</p></div>
        <dl class="release-facts"><div><dt>{{ t.releaseVersion }}</dt><dd>{{ release.windows.version }}</dd></div><div><dt>{{ t.releaseSize }}</dt><dd>{{ fileSize(release.windows.artifact.size_bytes) }}</dd></div><div class="release-digest"><dt>{{ t.releaseDigest }}</dt><dd>{{ release.windows.artifact.sha256 }}</dd></div></dl>
        <div class="release-actions"><a class="release-primary" :href="release.windows.artifact.url" :download="release.windows.environment === 'local' ? release.windows.artifact.name : undefined">{{ locale === 'zh' ? '下载 Windows 实验版' : 'Download Windows experimental' }}</a><a :href="release.windows.documents.windows_guide">{{ locale === 'zh' ? 'Windows 安装说明' : 'Windows installation guide' }}</a><a :href="release.windows.documents.checksums">{{ t.checksums }}</a></div>
      </section>
      <div v-if="plans.length" class="plan-grid">
        <article v-for="plan in plans" :key="plan.plan_id" class="plan-card" :class="`is-${plan.offer.kind}`" :data-plan-id="plan.plan_id" :data-plan-version="plan.version">
          <h3>{{ plan.name }}</h3><p v-if="plan.entitlements.feature_sets?.includes('standard')" class="plan-detail">{{ locale === 'zh' ? '订阅期间包含标准功能更新与升级' : 'Standard feature updates included during your subscription' }}</p><p class="plan-description">{{ plan.description }}</p>
          <div class="plan-price"><strong>{{ price(plan) }}</strong><span v-if="selectedTerm(plan)">{{ termLabel(selectedTerm(plan)!.years) }} · {{ t.total }}</span></div>
          <fieldset v-if="plan.offer.kind === 'fixed_price'" class="plan-terms" :disabled="sending"><legend>{{ t.term }}</legend><button v-for="term in plan.offer.terms" :key="term.years" type="button" :aria-pressed="selectedTerm(plan)?.years === term.years" @click="yearsByPlan[plan.plan_id] = term.years">{{ termLabel(term.years) }}</button></fieldset>
          <p v-if="plan.offer.kind === 'fixed_price'" class="plan-detail">{{ plan.offer.tax_mode === 'inclusive' ? t.includedTax : t.noTax }}</p>
          <p v-else-if="plan.offer.kind === 'free'" class="plan-detail">{{ expiry(plan) }}</p>
          <dl class="plan-quotas"><div v-for="entry in QUOTAS" :key="entry.id"><dt>{{ label(entry) }}</dt><dd>{{ quota(plan, entry.id) }}</dd></div></dl>
          <a v-if="release && isDownloadPlan(plan)" class="plan-action" :href="release.artifact.url" :download="release.environment === 'local' ? release.artifact.name : undefined">{{ t.download }}</a>
          <button v-else type="button" class="plan-action" :disabled="sending" @click="choose(plan)">{{ plan.offer.kind === 'free' ? t.learn : t.inquire }}</button>
        </article>
      </div>
      <div v-else class="plan-empty"><h3>{{ locale === 'zh' ? '企业授权与部署支持' : 'Enterprise licensing & deployment support' }}</h3><p>{{ locale === 'zh' ? '需要更多成员、账号容量或部署支持？告诉我们团队规模与使用场景，我们会确认适用权益、授权期限及交付报价。' : 'Need more members, account capacity or deployment support? Share your team size and use case so we can confirm entitlements, license terms and a delivery quote.' }}</p><p>{{ locale === 'zh' ? '当前未发布固定价格表，具体价格以确认的报价为准。' : 'A fixed price list has not been published here. Pricing is subject to a confirmed quote.' }}</p><button type="button" class="plan-action" @click="inquiry?.choose(null)">{{ t.contact }}</button></div>
      <details v-if="plans.length" class="plan-comparison"><summary>{{ t.compare }}</summary><p>{{ t.swipe }}</p><div class="comparison-scroll" tabindex="0" :aria-label="t.compare" data-lenis-prevent><table><thead><tr><th scope="col">{{ t.feature }}</th><th v-for="plan in plans" :key="plan.plan_id" scope="col">{{ plan.name }}</th></tr></thead><tbody><tr v-for="entry in CAPABILITIES" :key="entry.id"><th scope="row">{{ label(entry) }}</th><td v-for="plan in plans" :key="plan.plan_id">{{ effectiveFeatures(plan.entitlements).includes(entry.id) ? t.included : t.excluded }}</td></tr><tr v-for="entry in QUOTAS" :key="entry.id"><th scope="row">{{ label(entry) }}</th><td v-for="plan in plans" :key="plan.plan_id">{{ quota(plan, entry.id) }}</td></tr><tr><th scope="row">{{ t.minimum }}</th><td v-for="plan in plans" :key="plan.plan_id">{{ plan.minimum_version }}</td></tr></tbody></table></div></details>
      <ProductInquiry ref="inquiry" :locale="locale" :sitekey="sitekey" :email="email" @busy="sending = $event" />
    </div>
  </section>
</template>

<style scoped>
.release-pending{padding:28px;margin-bottom:32px;border:1px solid var(--line-dark);border-radius:16px;background:var(--paper-strong)}.release-pending h3{font-size:var(--font-size-title);margin:14px 0}.release-pending>div>span{font-size:var(--font-size-body);color:var(--muted)}.release-pending p{max-width:960px;font-size:var(--font-size-body);line-height:1.75;color:var(--muted)}.release-pending .release-availability{font-size:var(--font-size-body);color:var(--ink)}.release-pending .release-actions{margin-top:24px}.plan-empty .plan-action{width:fit-content;padding:12px 24px}
.catalog-section{padding:clamp(94px,9vw,144px) 32px 110px;color:#25242c;background:#f1eee7;scroll-margin-top:var(--header-height)}.catalog-inner{max-width:1440px;margin:0 auto}.catalog-heading{max-width:820px;margin-bottom:48px}.catalog-heading>span,.release-copy>span{color:#5e596d;font-size:var(--font-size-body);font-weight:750;letter-spacing:.08em;text-transform:uppercase}.catalog-heading h2{margin:18px 0;font-size:var(--font-size-display);letter-spacing:-.055em;line-height:1.12}.catalog-heading p,.catalog-note,.plan-empty p,.release-copy p{color:#706d75;line-height:1.8}.catalog-heading p{font-size:var(--font-size-body)}.release-card{margin:-12px 0 48px;padding:28px;display:grid;grid-template-columns:minmax(280px,.8fr) minmax(360px,1fr);gap:24px 48px;border:1px solid #cbc6bc;border-radius:16px;background:#e6e2da}.release-copy h3{margin:13px 0 9px;font-size:var(--font-size-title);letter-spacing:-.035em}.release-copy p{max-width:600px;margin:0;font-size:var(--font-size-body)}.release-copy b{width:fit-content;margin-top:15px;padding:6px 9px;display:block;border-radius:6px;color:#514a9b;background:#d7d1f1;font-size:var(--font-size-body)}.release-facts{margin:0;display:grid;grid-template-columns:repeat(3,1fr);align-content:start;gap:16px}.release-facts div{min-width:0;padding-bottom:13px;border-bottom:1px solid #cbc6bc}.release-facts dt{color:#817d84;font-size:var(--font-size-body)}.release-facts dd{margin:6px 0 0;font-size:var(--font-size-body);font-weight:650}.release-facts .release-digest{grid-column:1/-1}.release-digest dd{font:var(--font-size-body)/1.6 var(--font-ui);overflow-wrap:anywhere}.release-actions{grid-column:1/-1;display:flex;flex-wrap:wrap;align-items:center;gap:9px}.release-actions a,.plan-action{min-height:43px;padding:0 15px;display:inline-flex;align-items:center;justify-content:center;border:1px solid #bbb6ad;border-radius:8px;color:#36333b;background:#f5f2ec;font-size:var(--font-size-body);font-weight:700;text-decoration:none}.release-actions .release-primary{border-color:#2b2931;color:#fff;background:#2b2931}.release-actions a:hover{border-color:#6558db}.plan-grid{display:grid;grid-template-columns:repeat(auto-fit,minmax(min(100%,280px),1fr));gap:14px}.plan-card{position:relative;display:flex;flex-direction:column;min-width:0;min-height:520px;padding:27px;border:1px solid #d7d3cb;border-radius:14px;background:#fffefa;box-shadow:0 18px 50px #302b3610}.plan-card.is-fixed_price{border-color:#8078c3;box-shadow:0 20px 55px #5a50bd18}.plan-card.is-fixed_price::before{position:absolute;inset:0 0 auto;height:4px;border-radius:14px 14px 0 0;background:#6558db;content:""}.plan-card h3{margin:0 0 12px;font-size:var(--font-size-title);overflow-wrap:anywhere}.plan-description{min-height:70px;margin:0 0 25px;color:#77747c;font-size:var(--font-size-body);line-height:1.7;white-space:pre-wrap;overflow-wrap:anywhere}.plan-price{display:grid;gap:7px;margin-top:auto}.plan-price strong{font-size:var(--font-size-display);font-weight:700;overflow-wrap:anywhere;letter-spacing:-.045em}.plan-price span,.plan-detail{font-size:var(--font-size-body);color:#89868e}.plan-detail{min-height:17px}.plan-terms{display:flex;flex-wrap:wrap;gap:6px;padding:0;margin:22px 0 0;border:0}.plan-terms legend{margin-bottom:9px;font-size:var(--font-size-body);color:#8d8991}.plan-terms button{border:1px solid #d3d0c9;border-radius:6px;padding:6px 9px;background:#f8f6f1;cursor:pointer;font-size:var(--font-size-body)}.plan-terms button[aria-pressed=true]{color:#fff;background:#29272f;border-color:#29272f}.plan-quotas{display:grid;gap:12px;margin:23px 0;padding-top:21px;border-top:1px solid #e5e2dc;font-size:var(--font-size-body)}.plan-quotas div{display:flex;justify-content:space-between;gap:20px}.plan-quotas dt{color:#858189}.plan-quotas dd{margin:0;text-align:right;font-weight:600}.plan-action{width:100%;border-color:#2b2931;background:#2b2931;color:#fff;cursor:pointer;font-size:var(--font-size-body)}.plan-card.is-free .plan-action,.plan-card.is-contact .plan-action{color:#29272f;background:transparent}.plan-action:hover{background:#4941a5;border-color:#4941a5;color:#fff}.plan-action:disabled,button:disabled{opacity:.55;cursor:default}.catalog-note{max-width:880px;margin:22px 0 40px;font-size:var(--font-size-body)}.plan-empty{padding:28px;border:1px solid #d7d3cb;border-radius:12px;background:#fffefa}.plan-empty h3{margin-top:0;font-size:var(--font-size-title)}.plan-comparison{margin-bottom:66px}.plan-comparison summary{width:fit-content;padding:12px 0;cursor:pointer;font-size:var(--font-size-body);font-weight:700}.plan-comparison>p{color:#8a8790;font-size:var(--font-size-body)}.comparison-scroll{overflow:auto;max-width:100%;border:1px solid #d7d3cb;border-radius:10px;background:#fffefa}.comparison-scroll:focus-visible,button:focus-visible,summary:focus-visible,a:focus-visible{outline:3px solid #6558db50;outline-offset:3px}table{width:100%;border-collapse:collapse;font-size:var(--font-size-body)}th,td{min-width:170px;max-width:320px;padding:16px 18px;text-align:left;vertical-align:top;border-bottom:1px solid #e3e0da;overflow-wrap:anywhere}th{font-weight:650}td{color:#77747c}thead{background:#e9e6df}tbody tr:last-child>*{border-bottom:0}@media(max-width:760px){.catalog-section{padding:78px 18px}.catalog-heading{margin-bottom:28px}.catalog-heading h2{font-size:var(--font-size-display)}.catalog-heading p{font-size:var(--font-size-body)}.release-card{margin-top:0;padding:22px 17px;grid-template-columns:1fr}.release-facts{grid-template-columns:1fr 1fr}.release-facts .release-digest{grid-column:1/-1}.release-actions{grid-column:1}.release-actions a{width:100%}.plan-card{min-height:0;padding:22px}.plan-description{min-height:0}.plan-comparison{margin-bottom:42px}}
</style>
