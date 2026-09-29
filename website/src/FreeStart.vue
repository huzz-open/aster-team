<script setup lang="ts">
import { computed } from 'vue'
import catalog from 'virtual:aster-public-catalog'
import release from 'virtual:aster-product-release'
const props = defineProps<{ locale: 'zh' | 'en' }>()
defineEmits<{ explore: [] }>()
const plan = catalog?.plans.find(plan => plan.offer.kind === 'free' && (!release || (plan.plan_id === release.free_plan.plan_id && plan.version === release.free_plan.plan_version)))
// Until a catalog is supplied, describe the documented bundled free offer.
// Published catalog rights always take precedence; these values do not enforce entitlements.
const documented = { member_seats: 3, runners: 1, upstream_accounts: 1, api_keys_per_member: 1 }
const quotas = computed(() => [
  ['member_seats', props.locale === 'zh' ? '成员席位' : 'Member seats'],
  ['runners', 'Runner'],
  ['upstream_accounts', props.locale === 'zh' ? '订阅/账号' : 'Subscriptions & accounts'],
  ['api_keys_per_member', props.locale === 'zh' ? '有效 Key / 成员' : 'Active key / member'],
].map(([id,label]) => {
  const limit = plan?.entitlements.quotas.find(quota => quota.id === id)?.limit
  const value = catalog ? (limit?.mode === 'unlimited' ? (props.locale === 'zh' ? '不限' : 'Unlimited') : limit?.mode === 'limited' ? limit.value : '—') : documented[id as keyof typeof documented]
  return { id,label,value }
}))
</script>
<template>
  <section class="hero-free-start" :aria-label="locale==='zh'?'免费开始':'Start free'">
    <div class="free-start-copy"><strong>{{ locale==='zh'?'免费开始，按需升级':'Start free. Grow when ready.' }}</strong><p>{{ locale==='zh'?'内置免费授权，升级无需重装。':'Free license included. Upgrade without reinstalling.' }}</p><button type="button" @click="$emit('explore')">{{ locale==='zh'?'查看下载与授权':'View downloads & licensing' }}</button></div>
    <dl v-if="!catalog || plan"><div v-for="quota in quotas" :key="quota.id"><dd>{{ quota.value }}</dd><dt>{{ quota.label }}</dt></div></dl>
    <p v-else>{{ locale==='zh'?'免费授权详情请查看下载页。':'See the download section for free license details.' }}</p>
  </section>
</template>
<style scoped>
.hero-free-start{--free-start-padding:18px 24px}
.hero-free-start{width:min(100%,1320px);margin:0 auto;display:grid;grid-template-columns:minmax(260px,.9fr) minmax(0,1.6fr);gap:32px;padding:var(--free-start-padding);border:1px solid #ded9e5;border-radius:14px;background:#ffffff85}.free-start-copy strong{font-size:var(--font-size-body);font-weight:650}.free-start-copy p,.hero-free-start>p{font-size:var(--font-size-body);line-height:1.6;color:var(--muted);margin:7px 0}.free-start-copy button{border:0;background:transparent;padding:3px 0;color:var(--accent-dark);font-size:var(--font-size-body);text-decoration:underline;text-underline-offset:4px;cursor:pointer}dl{margin:0;display:grid;grid-template-columns:repeat(4,minmax(0,1fr));align-items:center;gap:18px}dl>div{min-width:0;text-align:center}dd{margin:0;font-size:var(--font-size-title);font-weight:650;color:var(--ink)}dt{margin-top:9px;font-size:var(--font-size-body);line-height:1.5;color:var(--muted)}@media(max-width:980px){.hero-free-start{grid-template-columns:1fr;padding:22px;gap:20px}}@media(max-width:500px){dl{grid-template-columns:repeat(2,minmax(0,1fr));gap:24px 16px}.free-start-copy strong{font-size:var(--font-size-body)}}
</style>
