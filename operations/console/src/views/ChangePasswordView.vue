<script setup lang="ts">
import { reactive, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, APasswordInput, useToast } from '@aster/ui'
import { changePassword } from '../api/client'
const router = useRouter(); const saving = ref(false); const toast = useToast()
const form = reactive({ current: '', next: '', confirm: '' })
async function submit() { if (form.next !== form.confirm) { toast.warning('两次输入的新密码不一致'); return }; saving.value = true; try { await changePassword(form.current, form.next); form.current = ''; form.next = ''; form.confirm = ''; toast.success('密码修改成功'); await router.push('/overview') } catch (value) { toast.error(value instanceof Error ? value.message : '修改密码失败') } finally { saving.value = false } }
</script>
<template><section class="content narrow"><div class="page-head"><div><h1>修改初始密码</h1><p>首次登录必须替换 env 中的一次性 bootstrap 密码；新密码只保存 bcrypt 哈希，并撤销其他会话。</p></div></div><form class="card form" @submit.prevent="submit"><div class="field"><span>当前密码</span><APasswordInput v-model="form.current" aria-label="当前密码" autocomplete="current-password" required /></div><div class="field"><span>新密码（12—72 字节）</span><APasswordInput v-model="form.next" aria-label="新密码" autocomplete="new-password" minlength="12" maxlength="72" required /></div><div class="field"><span>确认新密码</span><APasswordInput v-model="form.confirm" aria-label="确认新密码" autocomplete="new-password" minlength="12" maxlength="72" required /></div><div class="form-actions"><AButton type="submit" :loading="saving">修改密码</AButton></div></form></section></template>
