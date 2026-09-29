<script setup lang="ts">
import { computed, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ABrandMark, AButton, AIcon, useToast } from '@aster/ui'
import { licenseFeatureAvailable, request, type PublicLicenseState } from '@aster/sdk'
import { locale, setLocale } from '../i18n'

const router = useRouter()
const route = useRoute()
const toast = useToast()
const checking = ref(false)
const message = ref('')
const missingFeature = computed(() => route.query.reason === 'feature')

async function checkAgain() {
  checking.value = true
  message.value = ''
  try {
    const license = await request<PublicLicenseState>('/api/public/license-state')
    if (licenseFeatureAvailable(license, 'member')) {
      await router.push('/home')
      return
    }
    const reason = license.state === 'active' && license.available ? 'feature' : undefined
    if (route.query.reason !== reason) await router.replace({ path: '/license-required', query: { reason } })
    message.value = reason === 'feature'
      ? locale.value === 'zh-CN' ? '当前许可证未包含成员功能，请联系系统管理员。' : 'This license does not include member features. Contact your administrator.'
      : locale.value === 'zh-CN' ? '授权仍未生效，请联系系统管理员。' : 'The license is still unavailable. Contact your administrator.'
  } catch (value) {
    toast.error(value instanceof Error ? value.message : locale.value === 'zh-CN' ? '暂时无法连接控制服务，请稍后重试。' : 'Control is temporarily unreachable. Try again shortly.')
  } finally {
    checking.value = false
  }
}
</script>

<template>
  <main class="license-page">
    <section class="license-card">
      <header><ABrandMark class="brand-mark" :size="44" /><button type="button" class="locale-control" :aria-label="locale === 'zh-CN' ? 'Switch to English' : '切换到中文'" @click="setLocale(locale === 'zh-CN' ? 'en-US' : 'zh-CN')"><AIcon name="globe" :size="15" />{{ locale === 'zh-CN' ? 'EN' : '中' }}</button></header>
      <div class="license-mark"><AIcon name="lock" :size="28" /></div>
      <p class="eyebrow">Aster Team License</p>
      <h1>{{ missingFeature ? (locale === 'zh-CN' ? '当前授权未包含成员功能' : 'Member features are not licensed') : (locale === 'zh-CN' ? '系统尚未获得有效授权' : 'A valid license is required') }}</h1>
      <p>{{ missingFeature ? (locale === 'zh-CN' ? '系统许可证有效，但未授予成员功能。请联系本系统管理员核对所需权益，并在管理端更新许可证。' : 'The system license is valid but does not include member features. Ask your administrator to review the required capabilities and update the license.') : (locale === 'zh-CN' ? '成员业务功能暂时不可用。请联系本系统管理员，在管理端“产品授权”页面导入或更新许可证。' : 'Member features are unavailable. Ask your administrator to import or renew the license in Customer Admin.') }}</p>
      <div class="license-actions"><AButton icon="sync" :loading="checking" @click="checkAgain">{{ locale === 'zh-CN' ? '重新检查授权' : 'Check again' }}</AButton></div>
      <p v-if="message" class="check-message" role="status">{{ message }}</p>
    </section>
  </main>
</template>

<style scoped>
.license-page { min-height: 100vh; display: grid; place-items: center; padding: 24px; background: radial-gradient(circle at 50% 35%, var(--accent-soft), transparent 32%), var(--bg); }
.license-card { width: min(520px, 100%); padding: 36px; border: 1px solid var(--line); border-radius: 20px; background: var(--surface); box-shadow: var(--shadow-lg); }
.license-card header { display: flex; align-items: center; justify-content: space-between; margin-bottom: 34px; }
.brand-mark { width: 44px; height: 44px; display: block; border-radius: 12px; }
.locale-control { min-width: 38px; height: 34px; display: inline-flex; align-items: center; justify-content: center; gap: 5px; padding: 0 9px; color: var(--accent); border: 1px solid var(--line); border-radius: 9px; background: var(--surface-2); font-size:var(--font-size-caption); font-weight: 750; }
.locale-control:hover { border-color: var(--accent); }
.license-mark { width: 58px; height: 58px; display: grid; place-items: center; color: var(--accent); border: 1px solid color-mix(in srgb, var(--accent) 25%, var(--line)); border-radius: 17px; background: var(--accent-soft); }
.eyebrow { margin: 20px 0 8px; color: var(--accent); font-size:var(--font-size-caption); font-weight: 800; letter-spacing: .08em; text-transform: uppercase; }
h1 { margin: 0 0 12px; font-size: var(--font-size-display); line-height: var(--page-title-line-height); letter-spacing: var(--page-title-letter-spacing); }
.license-card > p { margin: 0; color: var(--text-soft); line-height: 1.7; }
.license-actions { margin-top: 24px; }
.check-message { margin-top: 14px !important; padding: 10px 12px; border-radius: 10px; background: var(--surface-2); font-size:var(--font-size-body); }
@media (max-width: 520px) { .license-page { padding: 16px; } .license-card { padding: 26px 22px; } }
</style>
