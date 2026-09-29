<script setup lang="ts">
import { reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, APasswordInput, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale } from '../i18n'

const router = useRouter()
const toast = useToast()
const saving = ref(false)
const form = reactive({ current_password: '', new_password: '', confirm_password: '' })
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh

async function submit() {
  if (form.new_password !== form.confirm_password) {
    toast.error(tx('两次输入的新密码不一致', 'The new passwords do not match'))
    return
  }
  saving.value = true
  try {
    await request('/api/admin/auth/password', { method: 'POST', body: JSON.stringify({ current_password: form.current_password, new_password: form.new_password }) })
    await router.push({ path: '/login', query: { password_changed: '1' } })
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('密码修改失败', 'Could not change password')) }
  finally { saving.value = false }
}
</script>

<template>
  <div class="content narrow-page">
    <header class="page-head"><div><h1>{{ tx('修改管理员密码', 'Change administrator password') }}</h1></div></header>
    <form class="card form password-form" @submit.prevent="submit">
      <div class="field"><span>{{ tx('当前密码', 'Current password') }}</span><APasswordInput v-model="form.current_password" :aria-label="tx('当前密码', 'Current password')" autocomplete="current-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
      <div class="field"><span>{{ tx('新密码', 'New password') }}</span><APasswordInput v-model="form.new_password" :aria-label="tx('新密码', 'New password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
      <div class="field"><span>{{ tx('确认新密码', 'Confirm new password') }}</span><APasswordInput v-model="form.confirm_password" :aria-label="tx('确认新密码', 'Confirm new password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
      <div class="form-actions"><AButton type="submit" :loading="saving">{{ tx('修改密码', 'Change password') }}</AButton></div>
    </form>
  </div>
</template>

<style scoped>.narrow-page{max-width:var(--page-max-width)}.password-form{max-width:620px}</style>
