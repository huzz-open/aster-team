<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { AButton, AFilePicker, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale } from '../i18n'

type MaintenanceStatus = 'queued' | 'verifying' | 'staging' | 'stopping_services' | 'restoring_previous' | 'migrating' | 'starting_candidate' | 'switching_traffic' | 'draining_previous' | 'succeeded' | 'failed'
type MaintenanceJob = {
  id: string
  operation: { type: 'upgrade'; archive: string; archive_sha256: string } | { type: 'delete_version'; version: string }
  status: MaintenanceStatus
  upgrade_mode?: 'maintenance' | 'blue_green'
  current_version: string
  target_version?: string
  message: string
  created_at: string
  updated_at: string
}
type InstalledVersion = { version: string; current: boolean }
type MaintenanceState = {
  jobs: MaintenanceJob[]; versions: InstalledVersion[]; current_version: string; busy?: boolean
  upgrade_capabilities?: { database_driver: string; supported_modes: ('maintenance' | 'blue_green')[]; unavailable_reason: string }
}

const toast = useToast()
const state = ref<MaintenanceState>({ jobs: [], versions: [], current_version: '' })
const loading = ref(true)
const uploading = ref(false)
const connectionLost = ref(false)
const selectedFile = ref<File | null>(null)
let pollTimer: ReturnType<typeof setTimeout> | undefined
let disposed = false

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const activeJob = computed(() => state.value.jobs.find(job => !['succeeded', 'failed'].includes(job.status)))
const busy = computed(() => state.value.busy === true || Boolean(activeJob.value))
const latestJobs = computed(() => state.value.jobs.slice(0, 10))
const canUpgrade = computed(() => state.value.upgrade_capabilities?.supported_modes.includes('maintenance') === true && !connectionLost.value)
const upgradeNotice = computed(() => {
  const capabilities = state.value.upgrade_capabilities
  if (!capabilities) return tx('正在确认环境升级能力；旧版本未提供能力信息时，请使用官方 CLI 维护升级流程。', 'Checking upgrade capabilities. For older versions without capability information, use the official CLI maintenance upgrade procedure.')
  if (capabilities.database_driver === 'sqlcipher') return tx('SQLite／SQLCipher 部署不支持不停服升级。维护期间管理端、用户端和 API 可能短暂不可用，正在进行的流式调用可能中断。', 'SQLite/SQLCipher deployments do not support zero-downtime upgrades. Admin, member pages and APIs may be temporarily unavailable, and active streams may be interrupted.')
  if (capabilities.database_driver === 'mariadb' && capabilities.supported_modes.includes('maintenance')) return tx('当前外部数据库支持维护升级，期间页面、API 和流式调用可能中断。完整蓝绿切换与排空能力尚未开放。', 'This external database supports maintenance upgrades. Pages, APIs and active streams may be interrupted. Blue-green switching and draining are not available yet.')
  return tx('当前数据库的升级能力尚未开放，暂不能从此页面执行升级。', 'Upgrades for this database are not available yet.')
})
const statusLabels: Record<MaintenanceStatus, [string, string]> = {
  queued: ['等待执行', 'Queued'],
  verifying: ['校验安装包', 'Verifying package'],
  staging: ['准备新版本', 'Staging release'],
  stopping_services: ['停止旧服务', 'Stopping services'],
  restoring_previous: ['恢复原版本', 'Restoring previous release'],
  starting_candidate: ['启动候选版本', 'Starting candidate'],
  migrating: ['自动迁移并健康检查', 'Migrating and checking health'],
  switching_traffic: ['切换访问流量', 'Switching traffic'],
  draining_previous: ['结束旧版本连接', 'Draining previous release'],
  succeeded: ['已完成', 'Succeeded'],
  failed: ['失败', 'Failed'],
}

function statusLabel(status: MaintenanceStatus) {
  const label = statusLabels[status]
  return label ? tx(label[0], label[1]) : status
}

function operationLabel(job: MaintenanceJob) {
  return job.operation.type === 'upgrade'
    ? tx(`升级至 ${job.target_version || '待识别版本'}`, `Upgrade to ${job.target_version || 'pending package verification'}`)
    : tx(`删除 ${job.operation.version}`, `Delete ${job.operation.version}`)
}

function phasesFor(job: MaintenanceJob): MaintenanceStatus[] {
  if (job.operation.type === 'upgrade' && job.upgrade_mode === 'maintenance') {
    if (job.status === 'restoring_previous') return ['queued', 'verifying', 'staging', 'stopping_services', 'restoring_previous']
    return ['queued', 'verifying', 'staging', 'stopping_services', 'starting_candidate', 'migrating', 'switching_traffic', 'succeeded']
  }
  return job.operation.type === 'upgrade'
    ? ['queued', 'verifying', 'staging', 'starting_candidate', 'migrating', 'switching_traffic', 'draining_previous', 'succeeded']
    : ['queued', 'verifying', 'succeeded']
}

function formatTime(value: string) {
  return new Intl.DateTimeFormat(locale.value, { dateStyle: 'medium', timeStyle: 'short' }).format(new Date(value))
}

async function loadState(quiet = false) {
  if (pollTimer) clearTimeout(pollTimer)
  try {
    state.value = await request<MaintenanceState>('/api/admin/maintenance')
    connectionLost.value = false
  } catch (value) {
    connectionLost.value = true
    if (!quiet) toast.error(value instanceof Error ? value.message : tx('升级状态加载失败', 'Could not load upgrade status'))
  } finally {
    loading.value = false
    if (!disposed) pollTimer = setTimeout(() => void loadState(true), activeJob.value ? 1500 : 5000)
  }
}

async function uploadPackage() {
  if (!selectedFile.value || !canUpgrade.value || busy.value || uploading.value) return
  uploading.value = true
  try {
    const body = new FormData()
    body.append('package', selectedFile.value)
    await request('/api/admin/maintenance/upgrade?mode=maintenance', { method: 'POST', body })
    toast.success(tx('安装包已上传，系统开始在后台校验并升级。', 'Package uploaded. Verification and upgrade have started in the background.'))
    selectedFile.value = null
    await loadState(true)
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('无法确认上传结果，请查看任务状态', 'Upload result could not be confirmed; check job status'))
    await loadState(true)
  } finally {
    uploading.value = false
  }
}

async function deleteVersion(version: string) {
  if (!window.confirm(tx(`确认删除版本 ${version} 及对应升级快照？`, `Delete version ${version} and its upgrade snapshot?`))) return
  try {
    await request(`/api/admin/maintenance/versions/${encodeURIComponent(version)}`, { method: 'DELETE' })
    toast.success(tx('清理任务已提交。', 'Cleanup job queued.'))
    await loadState(true)
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('版本清理失败', 'Could not queue version cleanup'))
  }
}

onMounted(() => void loadState())
onBeforeUnmount(() => { disposed = true; if (pollTimer) clearTimeout(pollTimer) })
</script>

<template>
  <div class="content maintenance-page">
    <header class="page-head">
      <div>
        <h1>{{ tx('系统升级', 'System upgrade') }}</h1>
      </div>
      <span class="current-version">{{ tx('当前版本', 'Current version') }} · {{ state.current_version || '—' }}</span>
    </header>

    <p v-if="connectionLost" class="maintenance-notice" role="status">{{ tx('暂时无法读取升级状态，页面会继续重试。已提交的升级由后台继续执行，请勿重复上传；恢复连接后再确认最终结果。', 'Upgrade status is temporarily unavailable. This page will retry. Submitted upgrades continue in the background; do not upload again. Confirm the final result after reconnecting.') }}</p>

    <section class="card upgrade-card">
      <div class="section-head">
        <div>
          <h2>{{ tx('上传安装包', 'Upload package') }}</h2>
          <p>{{ tx('仅接受当前系统和架构对应的 .tar.gz 签名发布包。数据库迁移随程序内置，不需要单独上传 SQL。', 'Only signed .tar.gz releases for this platform and architecture are accepted. Database migrations are embedded; no SQL upload is needed.') }}</p>
        </div>
      </div>
      <p class="maintenance-notice" role="note"><strong>{{ state.upgrade_capabilities?.supported_modes.includes('maintenance') ? tx('升级方式：维护升级', 'Upgrade mode: maintenance') : tx('升级暂不可用', 'Upgrade unavailable') }}</strong> {{ upgradeNotice }}</p>
      <div class="file-upload-row">
        <AFilePicker
          v-model="selectedFile"
          :label="tx('选择签名安装包', 'Select signed package')"
          :empty-label="tx('点击选择或拖入 .tar.gz 安装包', 'Choose or drop a .tar.gz package')"
          :hint="tx('仅支持 .tar.gz / .tgz，最大 1 GiB', '.tar.gz / .tgz only, up to 1 GiB')"
          accept=".tar.gz,.tgz,application/gzip,application/x-gzip"
          :disabled="busy || !canUpgrade"
          :loading="uploading"
        />
        <AButton icon="upload" :loading="uploading" :disabled="!selectedFile || busy || !canUpgrade" @click="uploadPackage">{{ busy ? tx('维护任务尚未结束', 'Maintenance is still pending') : tx('校验并维护升级', 'Verify and upgrade') }}</AButton>
      </div>
    </section>

    <section v-if="activeJob" class="card active-card">
      <div class="section-head"><div><h2>{{ operationLabel(activeJob) }}</h2><p>{{ activeJob.message }}</p></div><span class="status-pill is-active">{{ statusLabel(activeJob.status) }}</span></div>
      <ol class="phase-list">
        <li v-for="status in phasesFor(activeJob)" :key="status" :class="{ current: activeJob.status === status }">{{ statusLabel(status) }}</li>
      </ol>
    </section>

    <section class="card">
      <div class="section-head"><div><h2>{{ tx('已安装版本', 'Installed versions') }}</h2><p>{{ tx('当前版本不可删除，也不能手动切回旧版本。确认新版本正常后，可以清理历史版本和对应快照。', 'The current release cannot be deleted or manually rolled back. Older releases and snapshots can be removed after the upgrade is confirmed healthy.') }}</p></div></div>
      <div v-if="loading" class="empty-state">{{ tx('正在读取版本…', 'Loading versions…') }}</div>
      <div v-else class="version-list">
        <div v-for="version in state.versions" :key="version.version" class="version-row">
          <div><strong>{{ version.version }}</strong><span v-if="version.current" class="status-pill">{{ tx('当前运行', 'Current') }}</span></div>
          <AButton v-if="!version.current" variant="secondary" :disabled="busy" @click="deleteVersion(version.version)">{{ tx('删除版本', 'Delete') }}</AButton>
        </div>
      </div>
    </section>

    <section class="card">
      <div class="section-head"><div><h2>{{ tx('最近任务', 'Recent jobs') }}</h2></div></div>
      <div v-if="!latestJobs.length" class="empty-state">{{ tx('还没有升级或清理记录。', 'No upgrade or cleanup jobs yet.') }}</div>
      <div v-else class="job-list">
        <div v-for="job in latestJobs" :key="job.id" class="job-row">
          <div><strong>{{ operationLabel(job) }}</strong><small>{{ formatTime(job.updated_at) }} · {{ job.message }}</small></div>
          <span class="status-pill" :class="{ 'is-error': job.status === 'failed', 'is-success': job.status === 'succeeded' }">{{ statusLabel(job.status) }}</span>
        </div>
      </div>
    </section>
  </div>
</template>

<style scoped>
.maintenance-page{display:grid;gap:24px}.page-head p,.section-head p{max-width:820px;margin:6px 0 0;color:var(--muted)}.current-version{align-self:flex-start;padding:8px 12px;border:1px solid var(--line);border-radius:999px;color:var(--muted);white-space:nowrap}.upgrade-card,.active-card{display:grid;gap:18px}.status-pill{display:inline-flex;align-items:center;margin-left:10px;padding:4px 9px;border-radius:999px;background:var(--surface-2);color:var(--muted);font-size:var(--font-size-body)}.status-pill.is-active{color:var(--accent);background:color-mix(in srgb,var(--accent) 12%,transparent)}.status-pill.is-success{color:var(--positive)}.status-pill.is-error{color:var(--danger)}.phase-list{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:10px;margin:0;padding:0;list-style:none;counter-reset:phase}.phase-list li{padding:12px;border:1px solid var(--line);border-radius:10px;color:var(--muted);font-size:var(--font-size-body)}.phase-list li.current{border-color:var(--accent);color:var(--accent);background:color-mix(in srgb,var(--accent) 8%,transparent)}.version-list,.job-list{display:grid}.version-row,.job-row{display:flex;align-items:center;justify-content:space-between;gap:20px;padding:15px 0;border-top:1px solid var(--line)}.version-row:first-child,.job-row:first-child{border-top:0}.job-row small{display:block;margin-top:5px;color:var(--muted)}.empty-state{padding:18px 0;color:var(--muted)}@media(max-width:900px){.phase-list{grid-template-columns:repeat(2,minmax(0,1fr))}}@media(max-width:600px){.phase-list{grid-template-columns:1fr}.page-head{align-items:flex-start;flex-direction:column}}
.maintenance-page{align-content:start;gap:12px}.maintenance-page>.page-head{margin-bottom:4px}
.maintenance-notice{margin:0;padding:12px 16px;border:1px solid var(--line);border-radius:10px;background:var(--surface-2);color:var(--muted);line-height:1.6}.maintenance-notice strong{color:var(--text)}
</style>
