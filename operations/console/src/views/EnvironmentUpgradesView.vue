<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AEmpty, AModal, APasswordInput, ASelect, useToast } from '@aster/ui'
import { createEnvironmentUpgrade, createUpgradeEnvironment, getEnvironmentUpgrade, inspectUpgradeEnvironment, listEnvironmentUpgrades, listReleaseArtifacts, listUpgradeEnvironments, rotateUpgradeCredentials, type EnvironmentUpgrade, type ReleaseArtifact, type TargetMaintenance, type UpgradeEnvironment, type UpgradeProbeSample } from '../api/client'

const toast = useToast()
const route = useRoute()
const router = useRouter()
const environments = ref<UpgradeEnvironment[]>([])
const artifacts = ref<ReleaseArtifact[]>([])
const tasks = ref<EnvironmentUpgrade[]>([])
const environmentID = ref('')
const artifactID = ref('')
const capability = ref<TargetMaintenance | null>(null)
const selectedTask = ref<EnvironmentUpgrade | null>(null)
const samples = ref<UpgradeProbeSample[]>([])
const loading = ref(false)
const saving = ref(false)
const checking = ref(false)
const createOpen = ref(false)
const rotateOpen = ref(false)
const connectionUnavailable = ref(false)
let lastLoadFailure = ''
const form = reactive({ name: '', installation_id: '', admin_url: '', member_url: '', api_url: '', ca_pem: '', model: '', credentials: { admin_email: '', admin_password: '', api_key: '' } })
let timer: ReturnType<typeof setTimeout> | undefined
let disposed = false
let cursor = 0
const environmentOptions = computed(() => environments.value.map(item => ({ value: item.id, label: item.name, description: item.installation_id })))
const artifactOptions = computed(() => artifacts.value.filter(item => item.runtime_linkage && item.runtime_linkage !== 'unverified' && item.signature_ref.startsWith('release-key:') && item.github_run_id).map(item => ({ value: item.id, label: item.version, description: `${item.platform} / ${item.architecture} · ${item.sha256.slice(0, 12)}` })))
const busy = computed(() => tasks.value.some(item => item.environment_id === environmentID.value && item.phase !== 'completed'))
const canStart = computed(() => Boolean(!connectionUnavailable.value && artifactID.value && capability.value?.correlated_upgrades && capability.value.upgrade_capabilities.supported_modes.includes('maintenance') && !capability.value.busy && !busy.value))
function label(value: string) {
  return ({ queued: '排队中', baseline: '升级前探测', uploading: '上传安装包', tracking: '跟踪目标升级', observing: '升级后观察', completed: '已结束', pending: '等待结果', unknown: '状态待确认', succeeded: '升级成功', failed: '失败', not_applicable: '不适用（维护升级允许中断）', recovered: '已恢复', recovered_with_observation_gap: '已恢复，观察存在缺口', insufficient_coverage: '覆盖不足', not_started: '未开始升级', health: 'API 入口', admin: '管理端页面', member: '用户端页面', model: '短模型请求', stream: 'SSE 请求' } as Record<string, string>)[value] || value
}
function errorText(value: unknown) { return value instanceof Error ? value.message : '读取环境升级失败' }
watch(environmentID, () => { capability.value = null })
watch([createOpen, rotateOpen], ([create, rotate]) => { if (!create && !rotate) { form.credentials.admin_password = ''; form.credentials.api_key = '' } })
async function reload() {
  if (loading.value || disposed) return
  loading.value = true
  try {
    const [envs, releases, jobs] = await Promise.all([listUpgradeEnvironments(), listReleaseArtifacts(), listEnvironmentUpgrades()])
    if (disposed) return
    environments.value = envs; artifacts.value = releases; tasks.value = jobs
    const requestedID = typeof route.query.upgrade === 'string' ? route.query.upgrade : ''
    if (requestedID && selectedTask.value?.id !== requestedID) {
      selectedTask.value = jobs.find(item => item.id === requestedID) ?? (await getEnvironmentUpgrade(requestedID, 0)).task
      samples.value = []; cursor = 0
    }
    if (selectedTask.value) {
      const id = selectedTask.value.id
      const detail = await getEnvironmentUpgrade(id, cursor)
      if (!disposed && selectedTask.value?.id === id) {
        selectedTask.value = detail.task
        samples.value = [...samples.value, ...detail.samples].slice(-1000)
        if (detail.samples.length) cursor = detail.samples[detail.samples.length - 1]!.sequence
      }
    }
    connectionUnavailable.value = false; lastLoadFailure = ''
  } catch (value) {
    if (!disposed) {
      connectionUnavailable.value = true; capability.value = null
      const detail = errorText(value)
      if (detail !== lastLoadFailure) { lastLoadFailure = detail; toast.error(detail) }
    }
  }
  finally { loading.value = false }
}
async function poll() { await reload(); if (!disposed) timer = setTimeout(poll, 2000) }
async function inspect() {
  checking.value = true; capability.value = null
  const id = environmentID.value
  try { const result = await inspectUpgradeEnvironment(id); if (environmentID.value === id) capability.value = result }
  catch (value) { toast.error(errorText(value)) }
  finally { checking.value = false }
}
async function saveEnvironment() {
  if (saving.value) return
  saving.value = true
  try { const env = await createUpgradeEnvironment(form); createOpen.value = false; environmentID.value = env.id; await reload(); toast.success('目标环境已保存，密钥不在页面回显') }
  catch (value) { toast.error(errorText(value)) }
  finally { saving.value = false }
}
async function rotateCredentials() {
  if (saving.value) return
  saving.value = true
  try { await rotateUpgradeCredentials(environmentID.value, form.credentials); rotateOpen.value = false; capability.value = null; toast.success('凭据已更新，后台会在下一次凭据检查时使用新版本') }
  catch (value) { toast.error(errorText(value)) }
  finally { saving.value = false }
}
function selectTask(task: EnvironmentUpgrade) {
  selectedTask.value = task; samples.value = []; cursor = 0
  if (route.query.upgrade !== task.id) void router.push({ path: route.path, query: { ...route.query, upgrade: task.id } })
  void reload()
}
async function startUpgrade() {
  if (saving.value || !canStart.value) return
  saving.value = true
  try { const task = await createEnvironmentUpgrade(environmentID.value, artifactID.value); tasks.value = [task, ...tasks.value]; selectTask(task); capability.value = null; toast.success('后台升级任务已创建') }
  catch (value) { capability.value = null; await reload(); toast.error(errorText(value)) }
  finally { saving.value = false }
}
onMounted(poll)
watch(() => route.query.upgrade, id => {
  if (!id) { selectedTask.value = null; samples.value = []; cursor = 0; return }
  if (selectedTask.value?.id !== id) { selectedTask.value = tasks.value.find(task => task.id === id) ?? null; samples.value = []; cursor = 0; void reload() }
})
onBeforeUnmount(() => { disposed = true; if (timer) clearTimeout(timer); form.credentials.admin_password = ''; form.credentials.api_key = '' })
</script>

<template>
  <section class="content">
    <div class="page-head"><h1>环境升级</h1><div class="inline-actions"><RouterLink to="/release-center">返回发布中心</RouterLink><AButton @click="createOpen = true">添加目标环境</AButton></div></div>
    <p v-if="connectionUnavailable" class="notice">暂时无法读取最新任务状态，正在重新连接。后台任务独立执行。</p>
    <div class="form">
      <div class="two-columns">
        <label class="field"><span>目标环境</span><ASelect v-model="environmentID" :options="environmentOptions" aria-label="目标环境" placeholder="选择目标环境" /></label>
        <label class="field"><span>已验证制品</span><ASelect v-model="artifactID" :options="artifactOptions" aria-label="已验证制品" placeholder="选择已验证制品" /></label>
      </div>
      <div class="inline-actions"><AButton variant="secondary" :disabled="!environmentID" @click="rotateOpen = true">更新环境凭据</AButton><AButton variant="secondary" :disabled="!environmentID" :loading="checking" @click="inspect">检查目标能力</AButton><AButton :disabled="!canStart" :loading="saving" @click="startUpgrade">开始维护升级与探测</AButton></div>
      <p v-if="capability" class="notice">当前 {{ capability.current_version }} · {{ capability.upgrade_capabilities.database_driver }}。维护升级期间，页面、API Key 调用和已有 stream 可能中断。升级前观察 30 秒，升级后观察 60 秒；模型调用最多 120 次，整体探测最多 30 分钟。</p>
      <p v-if="busy" class="notice">该环境已有未结束的升级任务，请先查看任务结果。</p>
    </div>
    <h2>升级记录</h2>
    <div class="table-wrap"><table v-if="tasks.length" class="flat-data-table"><thead><tr><th>目标环境</th><th>目标版本</th><th>阶段</th><th>升级结果</th><th>连续性验证</th><th>恢复结果</th><th>操作</th></tr></thead><tbody><tr v-for="task in tasks" :key="task.id"><td>{{ environments.find(env => env.id === task.environment_id)?.name || task.environment_id }}</td><td>{{ task.target_version }}</td><td>{{ label(task.phase) }}</td><td>{{ label(task.upgrade_result) }}</td><td>{{ label(task.continuity_result) }}</td><td>{{ label(task.recovery_result) }}</td><td><AButton variant="secondary" @click="selectTask(task)">查看过程</AButton></td></tr></tbody></table><AEmpty v-else title="暂无环境升级记录" /></div>
    <template v-if="selectedTask">
      <h2>本次探测过程</h2><p>升级：{{ label(selectedTask.upgrade_result) }} · 恢复：{{ label(selectedTask.recovery_result) }} · 模型请求 {{ selectedTask.model_requests }} / 120</p>
      <div class="table-wrap"><table v-if="selectedTask.summary?.length" class="flat-data-table"><thead><tr><th>探测</th><th>样本</th><th>失败</th><th>最长观察中断</th></tr></thead><tbody><tr v-for="item in selectedTask.summary" :key="item.kind"><td>{{ label(item.kind) }}</td><td>{{ item.attempts }}</td><td>{{ item.failures }}</td><td>{{ (item.longest_failure_ms / 1000).toFixed(1) }} 秒</td></tr></tbody></table></div>
      <p v-if="selectedTask.coverage_gap" class="notice">本次观察存在重启缺口或预算限制，缺失的证据不会计为通过。</p>
      <p v-if="selectedTask.error_code" class="notice">{{ selectedTask.error_code }}</p>
      <p>展示最近 1000 条样本；记录保留原始失败。当前检查入口、HTML、短模型请求与 SSE，完整浏览器交互、会话续接和结算证据在后续阶段接入。</p>
      <div class="table-wrap"><table class="flat-data-table"><thead><tr><th>时间</th><th>阶段</th><th>探测</th><th>结果</th><th>耗时</th><th>错误</th></tr></thead><tbody><tr v-for="sample in samples" :key="sample.sequence"><td>{{ new Date(sample.at).toLocaleTimeString() }}</td><td>{{ label(sample.phase) }}</td><td>{{ label(sample.kind) }}</td><td>{{ sample.ok ? '成功' : '失败' }} · {{ sample.status }}</td><td>{{ sample.duration_ms }} ms</td><td>{{ sample.error_code || '—' }}</td></tr></tbody></table></div>
    </template>
    <AModal :open="rotateOpen" title="更新环境凭据" @close="rotateOpen = false"><form class="form" @submit.prevent="rotateCredentials">
      <label class="field"><span>目标管理账号</span><input v-model="form.credentials.admin_email" type="email" required autocomplete="off"></label>
      <label class="field"><span>目标管理密码</span><APasswordInput v-model="form.credentials.admin_password" required autocomplete="new-password" /></label>
      <label class="field"><span>真实模型探测 API Key</span><APasswordInput v-model="form.credentials.api_key" required autocomplete="new-password" /></label>
      <AButton type="submit" :loading="saving">保存新凭据</AButton>
    </form></AModal>
    <AModal :open="createOpen" title="添加目标环境" @close="createOpen = false">
      <form class="form" @submit.prevent="saveEnvironment">
        <div class="two-columns"><label class="field"><span>环境名称</span><input v-model="form.name" required maxlength="100" placeholder="内部 140"></label><label class="field"><span>安装身份</span><input v-model="form.installation_id" required maxlength="128"></label></div>
        <label v-for="entry in (['admin_url', 'member_url', 'api_url'] as const)" :key="entry" class="field"><span>{{ { admin_url: '管理端入口', member_url: '用户端入口', api_url: 'API 入口' }[entry] }}</span><input v-model="form[entry]" type="url" required placeholder="https://..."></label>
        <label class="field"><span>内部 CA（可选，PEM）</span><textarea v-model="form.ca_pem" rows="3" maxlength="65536" /></label>
        <label class="field"><span>目标管理账号</span><input v-model="form.credentials.admin_email" type="email" required autocomplete="off"></label>
        <label class="field"><span>目标管理密码</span><APasswordInput v-model="form.credentials.admin_password" required autocomplete="new-password" /></label>
        <label class="field"><span>真实模型探测 API Key</span><APasswordInput v-model="form.credentials.api_key" required autocomplete="new-password" /></label>
        <label class="field"><span>探测模型</span><input v-model="form.model" required maxlength="128"></label>
        <p>管理授权用于目标升级，API Key 用于正常模型请求。凭据加密保存；非回环地址必须使用 HTTPS。</p>
        <div class="form-actions"><AButton type="submit" :loading="saving">保存环境</AButton></div>
      </form>
    </AModal>
  </section>
</template>
