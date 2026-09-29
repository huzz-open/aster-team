<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRouter } from 'vue-router'
import { AButton, AEmpty, AIcon, ALoadingState, useToast } from '@aster/ui'
import { downloadOperationsExport, getOverview, listBackupHistory, type BackupRecord, type Overview } from '../api/client'
import ReauthActionModal from '../components/ReauthActionModal.vue'
import { presentationLabel } from '../presentation'

const data = ref<Overview | null>(null)
const router = useRouter()
const toast = useToast()
const backups = ref<BackupRecord[]>([])
const exportOpen = ref(false); const exporting = ref(false)
const loading = ref(true)
const pipeline = computed(() => [
  { label: '客户建档', detail: '联系人与开票资料', value: data.value?.customers_total ?? 0, to: '/customers', icon: 'customer' },
  { label: '订单收款', detail: '合同与线下到账', value: data.value?.orders_pending ?? 0, to: '/commercial/orders', icon: 'order', pending: (data.value?.orders_pending ?? 0) > 0 },
  { label: '许可证签发', detail: '机器申请与许可证文件', value: data.value?.paid_licenses_issued ?? 0, to: '/commercial/fulfillments', icon: 'shield' },
  { label: '发布交付', detail: '统一安装包与签名摘要', value: data.value?.release_artifacts_total ?? 0, to: '/release-artifacts', icon: 'package' },
])
onMounted(async () => {
  try { const [overview, history] = await Promise.all([getOverview(), listBackupHistory(10)]); data.value = overview; backups.value = history }
  catch (value) { toast.error(value instanceof Error ? value.message : '读取概览失败') }
  finally { loading.value = false }
})
async function exportData(password: string) { exporting.value = true; try { await downloadOperationsExport(password); exportOpen.value = false; toast.success('运营数据已导出') } catch (value) { toast.error(value instanceof Error ? value.message : '导出失败') } finally { exporting.value = false } }
</script>

<template>
  <section class="content">
    <div class="page-head"><div><h1>运营工作台</h1></div><div class="inline-actions"><AButton icon="order" variant="secondary" @click="router.push('/commercial/orders')">处理订单</AButton><AButton icon="download" variant="secondary" @click="exportOpen = true">导出运营数据</AButton></div></div>
    <ALoadingState v-if="loading" label="正在读取运营概览…" />
    <div v-else class="grid stats">
      <article class="card"><span class="stat-label">客户与线索总数</span><strong class="stat-value">{{ data?.customers_total ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">正式客户</span><strong class="stat-value">{{ data?.customers_active ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">待跟进线索</span><strong class="stat-value">{{ data?.customers_leads ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">套餐总数</span><strong class="stat-value">{{ data?.plans_total ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">待收款 / 待履约</span><strong class="stat-value">{{ data?.orders_pending ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">已签发免费分发</span><strong class="stat-value">{{ data?.free_distributions_issued ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">待签发付费授权</span><strong class="stat-value">{{ data?.paid_fulfillments_pending ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">已签发付费授权</span><strong class="stat-value">{{ data?.paid_licenses_issued ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">已入库安装包</span><strong class="stat-value">{{ data?.release_artifacts_total ?? '—' }}</strong></article>
      <article class="card"><span class="stat-label">失败备份记录</span><strong class="stat-value" :class="{ 'danger-text': (data?.backups_failed ?? 0) > 0 }">{{ data?.backups_failed ?? '—' }}</strong></article>
    </div>
    <section v-if="!loading" class="card operations-pipeline">
      <div class="section-head"><h2>客户履约主链</h2><span class="status">Offline-first</span></div>
      <div class="pipeline-grid">
        <button v-for="(item,index) in pipeline" :key="item.label" type="button" class="pipeline-step" :class="{ attention:item.pending }" @click="router.push(item.to)">
          <span class="pipeline-index">{{ index + 1 }}</span><span class="pipeline-icon"><AIcon :name="item.icon" /></span><span class="pipeline-copy"><strong>{{ item.label }}</strong><small>{{ item.detail }}</small></span><b>{{ item.value }}</b><AIcon name="chevron" :size="14" />
        </button>
      </div>
    </section>
    <article v-if="!loading" class="card backup-card"><h2>备份与校验记录</h2><div class="table-wrap"><table v-if="backups.length"><thead><tr><th>类型</th><th>对象</th><th>大小</th><th>状态</th><th>校验时间</th></tr></thead><tbody><tr v-for="item in backups" :key="item.id"><td>{{ presentationLabel('backupKind', item.kind) }}</td><td class="muted">{{ item.object_ref }}</td><td>{{ item.size_bytes.toLocaleString() }} B</td><td><span class="status" :class="{ off: item.status === 'failed' || item.status === 'consistency_failed', warning: item.status === 'created' }">{{ presentationLabel('backupStatus', item.status) }}</span></td><td>{{ item.verified_at ? new Date(item.verified_at).toLocaleString('zh-CN') : '未校验' }}</td></tr></tbody></table><AEmpty v-else title="尚无备份记录" /></div></article>
    <ReauthActionModal :open="exportOpen" title="导出运营数据" description="导出包含客户、联系人、订单、收款、授权索引和审计台账，不包含完整 License、密码、会话或签名密钥。此报告用于核对，数据库恢复请使用备份。" :busy="exporting" @close="exportOpen = false" @submit="exportData" />
  </section>
</template>

<style scoped>
.stats{grid-template-columns:repeat(5,minmax(0,1fr));gap:10px}.stats .card{min-height:78px;display:flex;align-items:center;justify-content:space-between;gap:10px;padding:13px}.stats .stat-value{margin:0;font-size:var(--font-size-title)}.operations-pipeline{margin-top:14px}.pipeline-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin-top:14px}.pipeline-step{display:grid;grid-template-columns:auto auto minmax(0,1fr) auto auto;align-items:center;gap:10px;padding:13px;color:var(--text);border:1px solid var(--line);border-radius:12px;background:linear-gradient(180deg,var(--surface),var(--surface-2));box-shadow:var(--shadow-sm);text-align:left}.pipeline-step:hover{background:color-mix(in srgb,var(--accent-soft) 40%,var(--surface))}.pipeline-step.attention{border-color:color-mix(in srgb,var(--warning) 48%,var(--line));background:linear-gradient(180deg,color-mix(in srgb,var(--warning-soft) 48%,var(--surface)),var(--surface))}.pipeline-index{font-size:var(--font-size-caption);color:var(--muted)}.pipeline-icon{display:grid;place-items:center;width:32px;height:32px;color:var(--accent);background:var(--accent-soft);border-radius:9px}.pipeline-step.attention .pipeline-icon{color:var(--warning);background:var(--warning-soft)}.pipeline-copy{display:flex;flex-direction:column;gap:3px;min-width:0}.pipeline-copy strong{font-size:var(--font-size-body)}.pipeline-copy small{overflow:hidden;color:var(--muted);font-size:var(--font-size-caption);text-overflow:ellipsis;white-space:nowrap}.pipeline-step b{font-size:var(--font-size-title)}.pipeline-step>.a-icon:last-child{color:var(--muted)}.pipeline-step:hover>.a-icon:last-child{color:var(--accent)}.backup-card{margin-top:14px}.backup-card h2{margin-bottom:12px}@media(max-width:1200px){.stats{grid-template-columns:repeat(3,minmax(0,1fr))}}@media(max-width:1100px){.pipeline-grid{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:650px){.stats{grid-template-columns:repeat(2,minmax(0,1fr))}.pipeline-grid{grid-template-columns:1fr}}@media(max-width:420px){.stats{grid-template-columns:1fr}}
</style>
