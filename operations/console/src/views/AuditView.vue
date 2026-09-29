<script setup lang="ts">
import { onMounted, ref } from 'vue'
import { AEmpty, ALoadingState, useToast } from '@aster/ui'
import { listAuditEvents, type AuditEvent } from '../api/client'
import { presentationLabel } from '../presentation'
const items = ref<AuditEvent[]>([]); const loading = ref(true); const toast = useToast()
onMounted(async () => { try { items.value = await listAuditEvents() } catch (value) { toast.error(value instanceof Error ? value.message : '读取审计事件失败') } finally { loading.value = false } })
</script>
<template><section class="content"><div class="page-head"><h1>审计记录</h1></div><div class="table-wrap"><ALoadingState v-if="loading" label="正在读取审计事件…" /><table v-else-if="items.length" class="flat-data-table"><thead><tr><th>时间</th><th>动作</th><th>动作代码</th><th>资源类型</th><th>资源 ID</th><th>操作员</th></tr></thead><tbody><tr v-for="item in items" :key="item.id"><td>{{ new Date(item.created_at).toLocaleString('zh-CN') }}</td><td><strong>{{ presentationLabel('auditAction', item.action) }}</strong></td><td class="code">{{ item.action }}</td><td>{{ presentationLabel('resourceType', item.resource_type) }}</td><td class="code">{{ item.resource_id }}</td><td>{{ item.operator_id }}</td></tr></tbody></table><AEmpty v-else title="暂无审计记录" /></div></section></template>
