<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('runner')
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { AButton, AConfirmModal, ACopyCode, AIconButton, AModal, ASegmentedControl, ASelect, useToast } from '@aster/ui'
import { APIError, copyText, formatDate, request, type Runner, type RunnerConnectionInfo } from '@aster/sdk'
import { locale } from '../i18n'
import { buildRunnerEnrollmentCommand, type RunnerPlatform } from '../runner-enrollment-command'

type Enrollment = { enrollment_id: string; runner_name: string; token: string; expires_at: string; notice: string }

const items = ref<Runner[]>([])
const keyword = ref('')
const statusFilter = ref('')
const platformFilter = ref('')
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'online', label: tx('在线', 'Online') },
  { value: 'offline', label: tx('离线', 'Offline') },
  { value: 'disabled', label: tx('已停用', 'Disabled') },
])
const platformOptions = computed(() => [
  { value: '', label: tx('全部平台', 'All platforms') },
  ...Array.from(new Set(items.value.map(runner => runner.platform).filter(Boolean)))
    .sort((a, b) => a.localeCompare(b))
    .map(platform => ({ value: platform, label: platform })),
])
const filteredItems = computed(() => {
  const needle = keyword.value.trim().toLocaleLowerCase()
  return items.value.filter(runner => {
    const state = !runner.enabled ? 'disabled' : runner.online ? 'online' : 'offline'
    return (!needle || `${runner.name} ${runner.id} ${runner.platform} ${runner.architecture} ${runner.version}`.toLocaleLowerCase().includes(needle))
      && (!statusFilter.value || state === statusFilter.value)
      && (!platformFilter.value || runner.platform === platformFilter.value)
  })
})
function resetFilters() {
  keyword.value = ''
  statusFilter.value = ''
  platformFilter.value = ''
}
const toast = useToast()
const addOpen = ref(false)
const newName = ref('')
const enrollment = ref<Enrollment | null>(null)
const loading = ref(false)
const pendingRemoval = ref<Runner | null>(null)
const operatingID = ref('')
const localDevelopmentControlURL = String(import.meta.env.VITE_ASTER_LOCAL_CONTROL_URL || '').trim()
const publicControlURL = ref(localDevelopmentControlURL)
const runnerPlatform = ref<RunnerPlatform>('linux')
const transportSecurity = computed(() => publicControlURL.value.startsWith('https:') ? 'https' : 'insecure_http')
const runnerPlatformChoices = computed(() => [
  { value: 'linux', label: 'Linux' },
  { value: 'windows', label: 'Windows' },
])
const enrollmentCommand = computed(() => {
  if (!enrollment.value) return ''
  return buildRunnerEnrollmentCommand({
    platform: runnerPlatform.value,
    installerBaseURL: window.location.origin,
    controlURL: publicControlURL.value,
    token: enrollment.value.token,
    allowInsecureHTTP: transportSecurity.value === 'insecure_http',
  })
})
const date = (value?: string) => formatDate(value, locale.value)
let timer = 0

async function load(silent = false) {
  if (!silent) loading.value = true
  try { items.value = (await request<{ items: Runner[] }>('/api/admin/runners')).items }
  catch (value) { if (!silent) toast.error(value instanceof Error ? value.message : tx('Runner 列表加载失败', 'Could not load runners')) }
  finally { if (!silent) loading.value = false }
}

function openAdd() {
  newName.value = ''
  enrollment.value = null
  addOpen.value = true
}

async function loadRunnerInstallationSettings() {
  await Promise.all([
    (async () => {
      if (localDevelopmentControlURL) {
        publicControlURL.value = localDevelopmentControlURL
        return
      }
      try {
        const settings = await request<RunnerConnectionInfo>('/api/admin/runners/connection')
        publicControlURL.value = settings.public_api_base_url.trim()
      } catch {
        publicControlURL.value = ''
      }
    })(),
    (async () => {
      try {
        const maintenance = await request<{ platform: string }>('/api/admin/maintenance')
        if (maintenance.platform === 'linux' || maintenance.platform === 'windows') runnerPlatform.value = maintenance.platform
      } catch { /* Keep the portable Linux default when platform discovery is unavailable. */ }
    })(),
  ])
}

async function createEnrollment() {
  if (!publicControlURL.value) {
    toast.error(tx('请先在设置中填写公共 API 基础地址。', 'Configure the public API base URL in Settings first.'))
    return
  }
  loading.value = true
  try {
    enrollment.value = await request('/api/admin/runners/enrollments', { method: 'POST', body: JSON.stringify({ name: newName.value }) })
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('无法生成安装命令', 'Could not generate the installation command')) }
  finally { loading.value = false }
}

async function copy(value: string) {
  try { await copyText(value); toast.success(tx('命令已复制到剪贴板。', 'Command copied to the clipboard.')) }
  catch (copyError) { toast.error(copyError instanceof Error ? copyError.message : tx('复制失败，请手动选择命令。', 'Copy failed. Select the command manually.')) }
}

function reportRunnerError(value: unknown, fallback: string) {
  if (value instanceof APIError && value.code === 'RUNNER_NOT_READY') {
    void load(true)
    return
  }
  toast.error(value instanceof Error ? value.message : fallback)
}

async function test(runner: Runner) {
  operatingID.value = runner.id
  try {
    await request(`/api/admin/runners/${runner.id}/test`, { method: 'POST' })
    toast.success(tx(`${runner.name} 控制通道与后台进程运行正常。`, `${runner.name} control channel and background process are healthy.`))
    await load(true)
  } catch (value) { reportRunnerError(value, tx('连通性测试失败', 'Connectivity test failed')) }
  finally { operatingID.value = '' }
}

async function toggle(runner: Runner) {
  operatingID.value = runner.id
  try {
    await request(`/api/admin/runners/${runner.id}`, { method: 'PATCH', body: JSON.stringify({ enabled: !runner.enabled }) })
    await load(true)
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('Runner 状态更新失败', 'Could not update runner status')) }
  finally { operatingID.value = '' }
}

async function remove(runner: Runner) {
  operatingID.value = runner.id
  try { await request(`/api/admin/runners/${runner.id}`, { method: 'DELETE' }); pendingRemoval.value = null; await load(true) }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('Runner 删除失败', 'Could not delete runner')) }
  finally { operatingID.value = '' }
}

onMounted(async () => { await Promise.all([load(), loadRunnerInstallationSettings()]); timer = window.setInterval(() => void load(true), 10_000) })
onBeforeUnmount(() => window.clearInterval(timer))
</script>

<template>
  <div class="content">
    <header class="page-head">
      <div><h1>{{ tx('Runner 节点', 'Runner nodes') }}</h1></div>
      <AButton icon="plus" @click="openAdd" :disabled="!canWrite">{{ tx('新增 Runner', 'Add runner') }}</AButton>
    </header>
    <div class="filter-bar runner-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :aria-label="tx('搜索 Runner 节点', 'Search Runner nodes')" :placeholder="tx('节点名称、Runner ID 或版本', 'Node name, Runner ID, or version')"></label>
      <label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="statusFilter" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label>
      <label class="field"><span>{{ tx('平台', 'Platform') }}</span><ASelect v-model="platformFilter" :options="platformOptions" :aria-label="tx('平台', 'Platform')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>
    <div class="table-wrap section">
      <table class="flat-data-table"><thead><tr><th>{{ tx('节点', 'Node') }}</th><th>Runner ID</th><th>{{ tx('节点状态', 'Status') }}</th><th>{{ tx('平台', 'Platform') }}</th><th>{{ tx('架构', 'Architecture') }}</th><th>{{ tx('版本', 'Version') }}</th><th>{{ tx('最后心跳', 'Last heartbeat') }}</th><th class="table-action-cell table-action-cell--three">{{ tx('操作', 'Actions') }}</th></tr></thead>
        <tbody><tr v-for="runner in filteredItems" :key="runner.id">
          <td><strong>{{ runner.name }}</strong></td>
          <td><code>{{ runner.id }}</code></td>
          <td><span class="status" :class="{ off: !runner.online || !runner.enabled }">{{ !runner.enabled ? tx('已停用', 'Disabled') : runner.online ? tx('在线', 'Online') : tx('离线', 'Offline') }}</span></td>
          <td>{{ runner.platform || '—' }}</td>
          <td>{{ runner.architecture || '—' }}</td>
          <td>{{ runner.version ? `v${runner.version}` : '—' }}</td>
          <td>{{ date(runner.last_seen_at) }}</td>
          <td class="table-action-cell table-action-cell--three"><div class="row-actions">
            <AIconButton icon="test" size="small" :label="tx('测试连通性', 'Test connectivity')" :disabled="!canWrite || operatingID === runner.id" @click="test(runner)" />
            <AIconButton icon="power" size="small" :variant="runner.enabled ? 'neutral' : 'accent'" :label="runner.enabled ? tx('停用 Runner', 'Disable runner') : tx('启用 Runner', 'Enable runner')" :disabled="(!canWrite && !runner.enabled) || operatingID === runner.id" @click="toggle(runner)" />
            <AIconButton icon="trash" size="small" variant="danger" :label="tx('删除 Runner', 'Delete runner')" :disabled="operatingID === runner.id" @click="pendingRemoval = runner" />
          </div></td>
        </tr></tbody>
      </table>
      <div v-if="!filteredItems.length" class="runner-list-empty" role="status">{{ loading && !items.length ? tx('正在读取 Runner…', 'Loading runners…') : items.length ? tx('没有匹配的 Runner', 'No matching runners') : tx('暂无 Runner 节点', 'No Runner nodes yet') }}</div>
    </div>

    <AModal :open="addOpen" :title="tx('新增 Runner', 'Add runner')" :description="enrollment ? tx('选择 Runner 所在平台，复制并运行安装命令，然后确认安装目录。', 'Choose the Runner platform, copy and run the installation command, then confirm the install directory.') : tx('输入节点名称，生成安装命令。', 'Enter a node name to generate the installation command.')" :close-label="tx('关闭', 'Close')" :close-disabled="loading" @close="addOpen=false">
      <form v-if="!enrollment" class="form" @submit.prevent="createEnrollment">
        <label class="field"><span>{{ tx('节点名称', 'Node name') }}</span><input v-model="newName" :placeholder="tx('例如：office-runner', 'Example: office-runner')" maxlength="80" required></label>
        <div class="form-actions"><AButton variant="secondary" :disabled="loading" @click="addOpen=false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="loading" :disabled="!canWrite">{{ tx('生成安装命令', 'Generate installation command') }}</AButton></div>
      </form>
      <div v-else class="form runner-install-flow">
        <ASegmentedControl class="runner-platform-tabs" v-model="runnerPlatform" :options="runnerPlatformChoices" :label="tx('Runner 平台', 'Runner platform')" stretch />
        <div class="runner-install-command">
          <strong>{{ tx('安装命令', 'Installation command') }}</strong>
          <ACopyCode :value="enrollmentCommand" layout="block" :label="tx('复制安装命令', 'Copy installation command')" :copied-label="tx('已复制', 'Copied')" @copy="copy(enrollmentCommand)" />
        </div>
      </div>
    </AModal>
    <AConfirmModal :open="!!pendingRemoval" :title="tx('删除 Runner', 'Delete runner')" :text="tx(`删除“${pendingRemoval?.name || ''}”会立即撤销远端凭据，但不会删除节点机器上的文件。`, `Deleting “${pendingRemoval?.name || ''}” immediately revokes remote credentials but does not remove files from the node.`)" :confirm-label="tx('确认删除', 'Delete runner')" confirm-icon="trash" :cancel-label="tx('取消', 'Cancel')" danger :busy="!!operatingID" @close="pendingRemoval = null" @confirm="pendingRemoval && remove(pendingRemoval)" />
  </div>
</template>

<style scoped>
.runner-filter .search-field{min-width:280px;flex:1 1 400px}.runner-filter .field:not(.search-field){max-width:190px}.runner-filter>.a-button{margin-left:auto}.runner-filter+.section{margin-top:0}
.runner-list-empty{display:grid;min-height:150px;place-items:center;padding:20px;color:var(--muted);font-size:var(--font-size-body);text-align:center}
.runner-install-command{display:grid;gap:9px}.runner-install-command>strong{font-size:var(--font-size-body)}.runner-install-command :deep(.a-copy-code--block pre){max-height:280px}
</style>
