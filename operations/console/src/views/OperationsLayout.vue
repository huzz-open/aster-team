<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AConsoleShell, type ConsoleNavSection, useToast } from '@aster/ui'
import { getSession, logout } from '../api/client'

const router = useRouter()
const route = useRoute()
const toast = useToast()
const operator = ref<{ email: string } | null>(null)
const baseSections: ConsoleNavSection[] = [
  { label: '流程中心', items: [
    { to: '/workflows/business', label: '业务办理', icon: 'order' },
    { to: '/workflows/release', label: '发布交付', icon: 'globe' },
  ] },
  { label: '基础数据', items: [
    { to: '/base/customers', label: '客户资料', icon: 'customer' },
    { to: '/base/plans', label: '产品套餐', icon: 'model' },
    { to: '/base/signing', label: '签名与环境', icon: 'shield' },
  ] },
  { label: '系统', items: [
    { to: '/system/audit', label: '审计记录', icon: 'audit' },
  ] },
]
const sections = computed<ConsoleNavSection[]>(() => baseSections.map(section => ({
  ...section,
  items: section.items.map(item => route.path === '/change-password'
    ? { ...item, disabled: true, disabledLabel: '请先完成初始密码修改' }
    : item),
})))
const labels = { notice: '系统通知', theme: '切换主题', profile: '运营账户', logout: '退出系统', collapse: '收起导航', allClear: '当前没有需要处理的系统通知。' }
onMounted(async () => { try { operator.value = (await getSession()).operator } catch (value) { toast.error(value instanceof Error ? value.message : '读取运营账户失败') } })
async function signOut() { try { await logout(); await router.push('/login'); toast.success('已安全退出') } catch (value) { toast.error(value instanceof Error ? value.message : '退出失败') } }
</script>

<template>
  <AConsoleShell class="operations-shell" subtitle="" :sections="sections" :user-name="operator?.email || ''" user-meta="Aster Team" :labels="labels" @logout="signOut" />
</template>
