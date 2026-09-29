<script setup lang="ts">
import { computed, reactive, ref } from 'vue'
import { AButton, AEmpty, ALoadingState, AModal, APasswordInput, ASelect, useToast } from '@aster/ui'
import {
  createReleaseTask,
  decideReleasePublish,
  downloadReleaseArtifact,
  executeReleasePublish,
  getReleaseCapabilities,
  listFreeDistributions,
  listReleasePublishRequests,
  listReleaseTasks,
  requestReleasePublish,
  retryReleaseTask,
  reverifyReleaseTask,
  type ReleaseCapabilities,
  type FreeDistributionRecord,
  type ReleasePublishRequest,
  type ReleaseTask,
  type ReleaseTaskArtifact,
  type ReleaseTarget,
} from '../api/client'
import { isSemanticVersion, MAXIMUM_SEMANTIC_VERSION_LENGTH, SEMANTIC_VERSION_HELP, SEMANTIC_VERSION_INPUT_PATTERN } from '../release-version'

const tasks = ref<ReleaseTask[]>([])
const loading = ref(true)
const toast = useToast()
const capabilities = ref<ReleaseCapabilities | null>(null)
const publishRequests = ref<ReleasePublishRequest[]>([])
const createOpen = ref(false)
const formalOpen = ref(false)
const formalAction = ref<'approve' | 'reject' | 'execute'>('approve')
const selectedPublishRequest = ref<ReleasePublishRequest | null>(null)
const saving = ref(false)
const downloadingArtifactIDs = reactive(new Set<string>())
const activeTaskID = ref('')
const activeTaskAction = ref<'retry' | 'reverify' | ''>('')
const form = reactive({ version: '', source_ref: 'main', free_distribution_id: '' })
const freeDistributions = ref<FreeDistributionRecord[]>([])
const formalForm = reactive({ comment: '', current_password: '' })
const versionInvalid = computed(() => form.version.length > 0 && !isSemanticVersion(form.version))
const freeDistributionOptions = computed(() => freeDistributions.value.filter(item => item.status === 'issued'
  && item.document_sha256
  && item.document?.claims.source.kind === 'free_distribution'
  && item.document.claims.binding.mode === 'unbound'
  && item.document.claims.validity.expiry.mode === 'none').map(item => ({
  value: item.snapshot.id,
  label: `${item.snapshot.plan.definition.name} · v${item.snapshot.plan.version}`,
  description: `${item.snapshot.id} · ${item.document_sha256!.slice(0, 12)}`,
})))

function statusText(status: string) {
  return ({ requested: '待审批', approved: '已批准', rejected: '已拒绝', publishing: '发布中', published: '已发布', failed: '失败', dispatching: '正在触发', queued: '排队中', in_progress: '构建中', verifying: '复验中', completed: '已完成', cancelled: '已取消' } as Record<string, string>)[status] || status
}

function statusClass(status: string) {
  return {
    off: ['failed', 'rejected', 'cancelled'].includes(status),
    warning: ['requested', 'dispatching', 'queued', 'in_progress', 'verifying', 'publishing'].includes(status),
  }
}

function targetLabel(target: ReleaseTarget) { return target.platform === 'windows' ? 'Windows' : 'Linux' }
function targetKey(target: ReleaseTarget) { return target.platform + '-' + target.architecture }
function packageFor(item: ReleaseTask, target: ReleaseTarget) {
  return item.packages.find(pkg => pkg.platform === target.platform && pkg.architecture === target.architecture)
}
function packageStatus(pkg?: ReleaseTaskArtifact) {
  if (!pkg) return '未生成安装包'
  return ({ pending: '未复验', queued: '复验中', verified: '复验通过', failed: '复验失败', unavailable: '产物不可用' } as Record<string, string>)[pkg.verification_status] || pkg.verification_status
}
function packageDetails(pkg?: ReleaseTaskArtifact) {
  return [packageStatus(pkg), pkg?.file_name, pkg?.verification_error_code].filter(Boolean).join('\n')
}
function taskStatus(item: ReleaseTask) {
  if (item.status === 'completed') {
    const complete = capabilities.value?.targets.every(target => packageFor(item, target)?.verification_status === 'verified')
    return { text: complete ? '复验完成' : '部分复验完成', tone: complete ? 'completed' : 'verifying' }
  }
  if (item.status === 'failed') return { text: item.github_conclusion === 'success' ? '复验失败' : '构建失败', tone: 'failed' }
  return { text: statusText(item.status), tone: item.status }
}
function taskStatusDetails(item: ReleaseTask) {
  return [
    '任务：' + item.id, '阶段：' + item.phase, '任务状态：' + statusText(item.status),
    'GitHub：' + (item.github_conclusion || (item.github_run_url ? '等待完成' : '等待创建 Run')),
    ...item.packages.map(pkg => pkg.platform + '：' + packageDetails(pkg)),
    item.error_code ? '错误码：' + item.error_code : '', '更新时间：' + formatTime(item.updated_at),
  ].filter(Boolean).join('\n')
}
function canReverify(item: ReleaseTask, pkg?: ReleaseTaskArtifact) {
  return ['completed', 'failed', 'verifying'].includes(item.status) && item.github_conclusion === 'success' && Boolean(pkg?.github_digest_sha256) && Boolean(pkg && ['pending', 'failed', 'unavailable'].includes(pkg.verification_status))
}
function packageOptions(item: ReleaseTask, action: 'download' | 'reverify') {
  return (capabilities.value?.targets || []).map(target => {
    const pkg = packageFor(item, target)
    const downloading = Boolean(pkg?.release_artifact_id && downloadingArtifactIDs.has(pkg.release_artifact_id))
    return {
      value: targetKey(target), label: targetLabel(target) + ' ' + target.architecture,
      description: downloading ? '下载中…' : packageDetails(pkg),
      disabled: action === 'download' ? !pkg?.release_artifact_id || pkg.verification_status !== 'verified' || downloading : saving.value || !canReverify(item, pkg),
    }
  })
}
function selectedPackage(item: ReleaseTask, key: string | number) {
  return item.packages.find(pkg => pkg.platform + '-' + pkg.architecture === String(key))
}
function publishablePackage(item: ReleaseTask) {
  return item.packages.find(pkg => pkg.platform === 'linux' && pkg.verification_status === 'verified' && pkg.release_artifact_id)
}

function formatTime(value?: string | null) {
  return value ? new Date(value).toLocaleString('zh-CN') : '—'
}

function shortSHA(value: string) { return value.length > 12 ? value.slice(0, 12) : value }

function openGitHubRun(item: ReleaseTask) {
  if (item.github_run_url) window.open(item.github_run_url, '_blank', 'noopener,noreferrer')
}

async function downloadArtifact(item: ReleaseTask, key: string | number) {
  const pkg = selectedPackage(item, key)
  const artifactID = pkg?.release_artifact_id
  if (!artifactID || pkg.verification_status !== 'verified' || downloadingArtifactIDs.has(artifactID)) return
  downloadingArtifactIDs.add(artifactID)
  try { await downloadReleaseArtifact(artifactID); toast.success('已开始保存安装包') }
  catch (value) { toast.error(value instanceof Error ? value.message : '下载发布物失败') }
  finally { downloadingArtifactIDs.delete(artifactID) }
}

async function loadTasks() {
  loading.value = true
  try {
    const [taskItems, releaseCapabilities, formalItems] = await Promise.all([listReleaseTasks(), getReleaseCapabilities(), listReleasePublishRequests()])
    tasks.value = taskItems
    capabilities.value = releaseCapabilities
    publishRequests.value = formalItems
    if (!form.source_ref) form.source_ref = releaseCapabilities.default_source_ref || 'main'
    try {
      freeDistributions.value = await listFreeDistributions()
      if (!freeDistributionOptions.value.some(option => option.value === form.free_distribution_id)) form.free_distribution_id = freeDistributionOptions.value[0]?.value || ''
    } catch {
      freeDistributions.value = []
      form.free_distribution_id = ''
    }
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '读取发布中心失败')
  } finally {
    loading.value = false
  }
}

async function createTask() {
  saving.value = true
  try {
    await createReleaseTask(form.version.trim(), form.source_ref.trim(), form.free_distribution_id)
    createOpen.value = false
    await loadTasks()
    toast.success('发布任务创建成功')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '创建发布任务失败')
  } finally {
    saving.value = false
  }
}

async function retryTask(item: ReleaseTask) {
  saving.value = true
  activeTaskID.value = item.id
  activeTaskAction.value = 'retry'
  try {
    await retryReleaseTask(item.id)
    await loadTasks()
    toast.success('重试构建请求已提交')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '重试构建失败')
  } finally {
    saving.value = false
    activeTaskID.value = ''
    activeTaskAction.value = ''
  }
}

async function reverifyTask(item: ReleaseTask, key: string | number) {
  const pkg = selectedPackage(item, key)
  if (saving.value || !pkg || !canReverify(item, pkg)) return
  saving.value = true
  activeTaskID.value = item.id
  activeTaskAction.value = 'reverify'
  try {
    await reverifyReleaseTask(item.id, pkg.id)
    await loadTasks()
    toast.success('重新复验请求已提交')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '重新复验失败')
  } finally {
    saving.value = false
    activeTaskID.value = ''
    activeTaskAction.value = ''
  }
}

async function requestFormalPublish(artifactID: string) {
  saving.value = true
  try {
    await requestReleasePublish(artifactID)
    await loadTasks()
    toast.success('正式发布申请已创建')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '创建正式发布申请失败')
  } finally {
    saving.value = false
  }
}

async function requestTaskPublish(item: ReleaseTask) {
  const pkg = publishablePackage(item)
  if (pkg?.release_artifact_id) await requestFormalPublish(pkg.release_artifact_id)
}

function openFormalAction(item: ReleasePublishRequest, action: 'approve' | 'reject' | 'execute') {
  selectedPublishRequest.value = item
  formalAction.value = action
  formalForm.comment = ''
  formalForm.current_password = ''
  formalOpen.value = true
}

async function submitFormalAction() {
  const item = selectedPublishRequest.value
  if (!item) return
  saving.value = true
  try {
    if (formalAction.value === 'execute') await executeReleasePublish(item.id, formalForm.current_password)
    else await decideReleasePublish(item.id, formalAction.value === 'approve' ? 'approved' : 'rejected', formalForm.comment, formalForm.current_password)
    formalOpen.value = false
    await loadTasks()
    toast.success(formalAction.value === 'execute' ? '正式发布已执行' : formalAction.value === 'approve' ? '正式发布申请已批准' : '正式发布申请已拒绝')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '正式发布操作失败')
  } finally {
    saving.value = false
  }
}

void loadTasks()
</script>

<template>
  <section class="content release-center">
    <div class="page-head">
      <h1>发布中心</h1>
      <div class="inline-actions">
        <RouterLink to="/environment-upgrades">环境升级</RouterLink>
        <AButton icon="sync" variant="secondary" :loading="loading" @click="loadTasks">刷新</AButton>
        <AButton icon="plus" :disabled="!capabilities?.configured" @click="createOpen = true">创建验证构建</AButton>
      </div>
    </div>

    <details class="release-service"><summary>{{ loading ? '正在连接构建服务' : capabilities?.configured ? '构建服务已连接' : '构建服务未配置' }}</summary><div class="notice">
      <template v-if="capabilities?.configured">已连接 <code>{{ capabilities.repository }}</code> / <code>{{ capabilities.workflow_file }}</code>。下载的 GitHub 安装包按 SHA-256 缓存在 Operations 文件存储中；再次复验会复用缓存并完整重复签名、清单、文件树、校验和与运行时检查。</template>
      <template v-else>当前未配置 GitHub 构建能力，只能查看历史任务。</template>
    </div></details>

    <h2 class="release-tasks-title">发布任务</h2>
    <div class="table-wrap">
      <ALoadingState v-if="loading && !tasks.length" label="正在读取发布任务…" />
      <table v-else-if="tasks.length" class="flat-data-table">
        <thead><tr><th>版本</th><th>来源提交</th><th>免费证书</th><th>模式</th><th>任务状态</th><th v-for="target in capabilities?.targets" :key="targetKey(target)">{{ targetLabel(target) }} 复验</th><th>更新时间</th><th>操作</th></tr></thead>
        <tbody>
          <tr v-for="item in tasks" :key="item.id">
            <td><strong>{{ item.version }}</strong></td>
            <td class="code">{{ shortSHA(item.source_commit_sha) }}</td>
            <td class="code" :title="item.free_distribution_id + ' · ' + item.free_license_sha256">{{ shortSHA(item.free_license_sha256) }}</td>
            <td>{{ item.mode === 'release' ? '正式发布' : '验证构建' }}</td>
            <td><span class="status task-status" :class="statusClass(taskStatus(item).tone)" :title="taskStatusDetails(item)" tabindex="0" :aria-label="taskStatusDetails(item)">{{ taskStatus(item).text }}</span></td>
            <td v-for="target in capabilities?.targets" :key="targetKey(target)"><span class="status" :class="{ off: ['failed', 'unavailable'].includes(packageFor(item, target)?.verification_status || ''), warning: !['verified', 'failed', 'unavailable'].includes(packageFor(item, target)?.verification_status || '') }" :title="packageDetails(packageFor(item, target))">{{ packageStatus(packageFor(item, target)) }}</span></td>
            <td>{{ formatTime(item.updated_at) }}</td>
            <td>
              <div class="inline-actions task-actions">
                <AButton size="small" variant="ghost" :disabled="!item.github_run_url" @click="openGitHubRun(item)">打开 GitHub</AButton>
                <AButton size="small" variant="secondary" :disabled="!['failed', 'cancelled'].includes(item.status) || saving" :loading="saving && activeTaskID === item.id && activeTaskAction === 'retry'" @click="retryTask(item)">重试构建</AButton>
                <ASelect :options="packageOptions(item, 'reverify')" placeholder="重新复验" :aria-label="'选择复验平台 ' + item.version" :disabled="saving" :popup-min-width="260" @change="reverifyTask(item, $event)" />
                <ASelect :options="packageOptions(item, 'download')" placeholder="下载" :aria-label="'选择下载平台 ' + item.version" :popup-min-width="260" @change="downloadArtifact(item, $event)" />
                <AButton size="small" :disabled="!publishablePackage(item) || !capabilities?.publishing_configured || saving" @click="requestTaskPublish(item)">申请发布 Linux</AButton>
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      <AEmpty v-else title="暂无发布任务" />
    </div>

    <div class="section-head formal-head"><div><h2>正式发布审批</h2></div></div>
    <div class="table-wrap">
      <table v-if="publishRequests.length" class="flat-data-table">
        <thead><tr><th>版本</th><th>Tag</th><th>来源提交</th><th>状态</th><th>GitHub Release</th><th>更新时间</th><th>操作</th></tr></thead>
        <tbody><tr v-for="item in publishRequests" :key="item.id">
          <td><strong>{{ item.version }}</strong></td>
          <td class="code">{{ item.tag_name }}</td>
          <td class="code">{{ shortSHA(item.source_commit_sha) }}</td>
          <td><span class="status" :class="statusClass(item.status)" :title="item.error_code || undefined">{{ statusText(item.status) }}</span></td>
          <td><a v-if="item.html_url" :href="item.html_url" target="_blank" rel="noreferrer">打开 Release</a><span v-else>—</span></td>
          <td>{{ formatTime(item.updated_at) }}</td>
          <td><div class="inline-actions">
            <AButton v-if="item.status === 'requested'" size="small" @click="openFormalAction(item, 'approve')">批准</AButton>
            <AButton v-if="item.status === 'requested'" size="small" variant="secondary" @click="openFormalAction(item, 'reject')">拒绝</AButton>
            <AButton v-if="item.status === 'approved' || item.status === 'failed'" size="small" :disabled="!capabilities?.publishing_configured" @click="openFormalAction(item, 'execute')">{{ item.status === 'failed' ? '重试发布' : '执行发布' }}</AButton>
          </div></td>
        </tr></tbody>
      </table>
      <AEmpty v-else title="暂无正式发布申请" />
    </div>

    <AModal :open="createOpen" title="创建无 tag 验证构建" @close="createOpen = false">
      <form class="form" @submit.prevent="createTask">
        <div class="notice">后端只触发固定 Workflow。版本必须与来源提交的 <code>package.json</code> 一致；来源只能是 <code>main</code> 或已经属于 main 的完整 commit SHA。</div>
        <label class="field"><span>版本号</span><input v-model.trim="form.version" required placeholder="2.0.0" :pattern="SEMANTIC_VERSION_INPUT_PATTERN" :maxlength="MAXIMUM_SEMANTIC_VERSION_LENGTH" :title="SEMANTIC_VERSION_HELP" aria-describedby="release-version-help"><small id="release-version-help">{{ SEMANTIC_VERSION_HELP }}</small><small v-if="versionInvalid" class="danger-text">请输入有效的 SemVer 2.0.0 版本号。</small></label>
        <label class="field"><span>来源分支 / commit</span><input v-model="form.source_ref" required placeholder="main"></label>
        <label class="field"><span>随包免费证书</span><ASelect v-model="form.free_distribution_id" aria-label="随包免费证书" :options="freeDistributionOptions" required searchable placeholder="选择已签发的永久免费分发" /><small>只列出未绑定且不设到期日的已签发证书；任务会冻结其 SHA-256，Linux 与 Windows 产物必须包含同一份证书。</small></label>
        <div class="form-actions"><AButton variant="secondary" type="button" :disabled="saving" @click="createOpen = false">取消</AButton><AButton type="submit" :loading="saving" :disabled="versionInvalid || !form.free_distribution_id">预检并触发</AButton></div>
      </form>
    </AModal>

    <AModal :open="formalOpen" :title="formalAction === 'execute' ? '执行正式发布' : formalAction === 'approve' ? '批准正式发布' : '拒绝正式发布'" @close="formalOpen = false">
      <form class="form" @submit.prevent="submitFormalAction">
        <div class="notice"><template v-if="formalAction === 'execute'">此操作会创建受保护的 <code>{{ selectedPublishRequest?.tag_name }}</code>、上传已验收的同一份安装包和校验文件并公开 GitHub Release；不会重新构建。</template><template v-else>申请人与审批人必须不同。审批决定会进入追加审计记录，不能覆盖修改。</template></div>
        <label v-if="formalAction !== 'execute'" class="field"><span>审批意见</span><textarea v-model="formalForm.comment" maxlength="1000" rows="4" required></textarea></label>
        <div class="field"><span>当前密码</span><APasswordInput v-model="formalForm.current_password" aria-label="当前密码" autocomplete="current-password" required /></div>
        <div class="form-actions"><AButton variant="secondary" type="button" :disabled="saving" @click="formalOpen = false">取消</AButton><AButton type="submit" :loading="saving">确认</AButton></div>
      </form>
    </AModal>
  </section>
</template>

<style scoped>
.release-service{margin-bottom:20px}.release-service summary{width:max-content;cursor:pointer;color:var(--text-soft);font-weight:650}.release-service .notice{margin-top:10px}
.release-tasks-title { margin: 24px 0 14px; }
.formal-head { margin-top: 32px; }
.task-status { cursor: help; outline-offset: 3px; }
.task-status:focus-visible { outline: 2px solid var(--accent); }
.task-actions { display: grid; grid-auto-flow: column; grid-auto-columns: max-content; gap: 7px; min-width: max-content; }
.task-actions :deep(.a-button) { width: 100%; }
.task-actions :deep(.a-select) { min-width: 104px; }
</style>
