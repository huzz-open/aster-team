<script setup lang="ts">
import { reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, APasswordInput, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { t } from '../i18n'

const router = useRouter()
const toast = useToast()
const saving = ref(false)
const form = reactive({ current_password: '', new_password: '', confirm_password: '' })
async function submit() {
  if (form.new_password !== form.confirm_password) { toast.error(t('mismatch')); return }
  saving.value = true
  try {
    await request('/api/member/auth/password', { method: 'POST', body: JSON.stringify({ current_password: form.current_password, new_password: form.new_password }) })
    await router.push('/login')
  } catch (value) { toast.error(value instanceof Error ? value.message : t('changeFailed')) }
  finally { saving.value = false }
}
</script>

<template>
  <div class="content narrow-page">
    <header class="page-head"><div><h1>{{ t('changePasswordSection') }}</h1></div></header>
    <form class="card form password-form" @submit.prevent="submit">
      <div class="field"><span>{{ t('currentPassword') }}</span><APasswordInput v-model="form.current_password" :aria-label="t('currentPassword')" autocomplete="current-password" required /></div>
      <div class="field"><span>{{ t('newPassword') }}</span><APasswordInput v-model="form.new_password" :aria-label="t('newPassword')" minlength="12" autocomplete="new-password" required /></div>
      <div class="field"><span>{{ t('confirmPassword') }}</span><APasswordInput v-model="form.confirm_password" :aria-label="t('confirmPassword')" minlength="12" autocomplete="new-password" required /></div>
      <div class="form-actions"><AButton type="submit" :loading="saving">{{ t('changePassword') }}</AButton></div>
    </form>
  </div>
</template>

<style scoped>.narrow-page{max-width:var(--page-max-width)}.password-form{max-width:620px}</style>
