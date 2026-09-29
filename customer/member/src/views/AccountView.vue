<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { ALoadingState, useToast } from '@aster/ui'
import { formatDate, formatTokens, request, type User } from '@aster/sdk'
import { locale, t } from '../i18n'

const user = ref<User | null>(null)
const loading = ref(true)
const toast = useToast()
onMounted(async () => {
  try { user.value = await request<User>('/api/member/me') }
  catch (value) { toast.error(value instanceof Error ? value.message : t('overviewLoadFailed')) }
  finally { loading.value = false }
})
</script>

<template>
  <div class="content">
    <header class="page-head"><div><h1>{{ t('personalSettingsTitle') }}</h1></div></header>
    <ALoadingState v-if="loading" :label="t('loadingOverview')" />
    <section v-else-if="user" class="profile-card card">
      <span class="profile-avatar-large">{{ user.display_name.slice(0,2) }}</span>
      <div class="profile-identity"><h2>{{ user.display_name }}</h2><span>{{ user.email }}</span></div>
      <dl>
        <div><dt>{{ t('status') }}</dt><dd><span class="status">{{ user.status==='active'?t('active'):user.status }}</span></dd></div>
        <div><dt>{{ t('memberSince') }}</dt><dd>{{ formatDate(user.created_at,locale) }}</dd></div>
        <div><dt>{{ t('currentBalance') }}</dt><dd>{{ formatTokens(user.balance_tokens||0,locale) }}</dd></div>
        <div><dt>{{ t('settledTokens') }}</dt><dd>{{ formatTokens(user.billed_tokens||0,locale) }}</dd></div>
      </dl>
    </section>
  </div>
</template>

<style scoped>
.profile-card{display:grid;grid-template-columns:auto minmax(180px,1fr) minmax(520px,2fr);align-items:center;gap:16px}.profile-avatar-large{width:48px;height:48px;display:grid;place-items:center;border-radius:14px;color:#fff;background:linear-gradient(145deg,var(--accent),var(--accent-2));font-size:var(--font-size-body);font-weight:850}.profile-identity{min-width:0}.profile-identity h2,.profile-identity span{overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.profile-identity span{color:var(--muted);font-size:var(--font-size-body)}.profile-card dl{display:grid;grid-template-columns:repeat(4,minmax(100px,1fr));gap:10px;margin:0}.profile-card dl>div{display:grid;gap:5px;padding-left:12px;border-left:1px solid var(--line)}dt{color:var(--muted);font-size:var(--font-size-caption)}dd{margin:0;color:var(--text-soft);font-size:var(--font-size-body);font-weight:650}@media(max-width:1000px){.profile-card{grid-template-columns:auto 1fr}.profile-card dl{grid-column:1/-1}}@media(max-width:620px){.profile-card dl{grid-template-columns:repeat(2,1fr)}}
</style>
