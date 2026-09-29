<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AFilePicker, AFloatingPanel, AInfoTip, AModal, ASelect, useToast } from '@aster/ui'
import { request } from '@aster/sdk'
import { locale } from '../i18n'
import { useLicensedFeature } from '../license-status'

type PluginDisplay = { name: string; description: string }
type PluginVersion = { digest: string; version: string | null; available: boolean }
type PluginRecord = {
  bundle_id: string
  provider: string
  active_digest?: string
  active_version?: string
  active_display?: Record<string, PluginDisplay>
  activation_revision?: number
  versions: PluginVersion[]
  recovery_error?: string
}
type PluginStatus = { configured: boolean; plugins: PluginRecord[] }

const route = useRoute()
const router = useRouter()
const toast = useToast()
const canWritePlugin = useLicensedFeature('gateway')
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const activeTab = computed(() => route.query.tab === 'plugins' ? 'plugins' : 'runtime')
const configuration = reactive({ public_api_base_url: '', usage_multiplier: 1 })
const saving = ref(false)
const billing = reactive<{ configured: boolean; settlement_currency: 'CNY' | 'USD'; usd_to_cny: string }>({ configured: true, settlement_currency: 'CNY', usd_to_cny: '6.7' })
const billingSaving = ref(false)
const currencyOptions = computed(() => [{ value: 'CNY', label: tx('人民币（CNY）', 'Chinese yuan (CNY)') }, { value: 'USD', label: tx('美元（USD）', 'US dollar (USD)') }])
const pluginStatus = ref<PluginStatus | null>(null)
const pluginStatusError = ref('')
const selectedPlugin = ref<File | null>(null)
const uploadOpen = ref(false)
const pluginBusy = ref(false)
const switchBusyDigest = ref('')
const switchTargetId = ref('')
const pluginManageable = computed(() => canWritePlugin.value && !!pluginStatus.value?.configured && !pluginBusy.value)
const pluginDisabledReason = computed(() => {
  if (!canWritePlugin.value) return tx('当前授权不允许更新上游适配插件', 'The current license does not allow updating the upstream adapter plugin')
  if (pluginStatusError.value) return pluginStatusError.value
  if (!pluginStatus.value) return tx('正在读取插件状态', 'Loading plugin status')
  if (!pluginStatus.value.configured) return tx('服务端尚未配置插件签名公钥，请联系部署管理员', 'The server has no plugin signing key configured. Ask the deployment administrator')
  return ''
})
const uploadActionAnchor = ref<HTMLElement | null>(null)
const uploadHintOpen = ref(false)
const switchHintAnchor = ref<HTMLElement | null>(null)
const switchHintId = ref('')
const pluginRows = computed(() => pluginStatus.value?.plugins ?? [])
const switchTarget = computed(() => pluginRows.value.find(row => row.bundle_id === switchTargetId.value) ?? null)
const switchHintRow = computed(() => pluginRows.value.find(row => row.bundle_id === switchHintId.value) ?? null)
const switchHintReason = computed(() => switchHintRow.value ? switchDisabledReason(switchHintRow.value) : '')
const versionRows = computed(() => [...(switchTarget.value?.versions ?? [])]
  .sort((a, b) => Number(b.digest === switchTarget.value?.active_digest) - Number(a.digest === switchTarget.value?.active_digest)
    || (b.version ?? '').localeCompare(a.version ?? '', 'en', { numeric: true }) || a.digest.localeCompare(b.digest)))
function canSwitch(row: PluginRecord) {
  return row.activation_revision != null && row.versions.some(item => item.available && item.digest !== row.active_digest)
}
function switchDisabledReason(row: PluginRecord) {
  if (pluginBusy.value) return tx('插件操作进行中', 'A plugin operation is in progress')
  if (pluginDisabledReason.value) return pluginDisabledReason.value
  if (!canSwitch(row)) return tx('暂无其他可切换版本', 'No other version is available')
  return ''
}
function showSwitchHint(event: Event, row: PluginRecord) {
  if (!switchDisabledReason(row)) return
  switchHintAnchor.value = event.currentTarget as HTMLElement
  switchHintId.value = row.bundle_id
}
function hideSwitchHint(event: Event) {
  if (switchHintAnchor.value === event.currentTarget) switchHintId.value = ''
}
function pluginState(row: PluginRecord) {
  if (row.recovery_error) return { label: tx('需要处理', 'Needs attention'), tone: 'off' }
  if (!row.active_digest) return { label: tx('未激活', 'Inactive'), tone: 'warning' }
  return { label: tx('运行正常', 'Running normally'), tone: '' }
}
function displayFor(row: PluginRecord): PluginDisplay {
  const display = row.active_display?.[locale.value] ?? row.active_display?.['zh-CN'] ?? row.active_display?.['en-US']
  if (display) return display
  if (row.provider === 'openai') return { name: tx('OpenAI 上游适配插件', 'OpenAI upstream adapter'), description: tx('OpenAI API 与 Codex 订阅协议适配', 'OpenAI API and Codex subscription protocol adapters') }
  if (row.provider === 'deepseek') return { name: tx('DeepSeek 上游适配插件', 'DeepSeek upstream adapter'), description: tx('DeepSeek API 协议适配', 'DeepSeek API protocol adapter') }
  if (row.provider === 'glm') return { name: tx('GLM 上游适配插件', 'GLM upstream adapter'), description: tx('BigModel 与 Z.AI 通用 API、Coding Plan 协议适配', 'BigModel and Z.AI general API and Coding Plan protocol adapters') }
  return { name: row.bundle_id || '—', description: '' }
}

function closeUpload() {
  if (pluginBusy.value) return
  uploadOpen.value = false
  selectedPlugin.value = null
}

function selectTab(tab: 'runtime' | 'plugins') {
  if (activeTab.value === tab) return
  void router.push({ path: '/settings', query: tab === 'plugins' ? { tab: 'plugins' } : {} })
}

async function loadConfiguration() {
  try { Object.assign(configuration, await request<typeof configuration>('/api/admin/settings')) }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('配置加载失败', 'Could not load settings')) }
}
async function loadPluginStatus() {
  pluginStatusError.value = ''
  try { pluginStatus.value = await request<PluginStatus>('/api/admin/plugins/status') }
  catch (value) {
    pluginStatus.value = null
    pluginStatusError.value = value instanceof Error ? value.message : tx('插件状态加载失败', 'Could not load plugin status')
    toast.error(pluginStatusError.value)
  }
}
async function loadBilling() {
  try {
    const result = await request<{ configured: boolean; settlement_currency: 'CNY' | 'USD' | null; usd_to_cny: string | null }>('/api/admin/billing/settings')
    billing.configured = result.configured
    billing.settlement_currency = result.settlement_currency ?? 'CNY'
    billing.usd_to_cny = result.usd_to_cny ?? ''
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('费用配置加载失败', 'Could not load billing settings')) }
}
async function saveBilling() {
  billingSaving.value = true
  try {
    await request('/api/admin/billing/settings', { method: 'PUT', body: JSON.stringify({
      settlement_currency: billing.settlement_currency,
      usd_to_cny: billing.usd_to_cny.trim() || null,
    }) })
    billing.configured = true
    toast.success(tx('费用配置已保存。', 'Billing settings saved.'))
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('费用配置保存失败', 'Could not save billing settings')) }
  finally { billingSaving.value = false }
}
onMounted(() => { void loadConfiguration(); void loadBilling() })
watch(activeTab, tab => { if (tab === 'plugins') void loadPluginStatus() }, { immediate: true })

async function saveConfiguration() {
  saving.value = true
  try {
    Object.assign(configuration, await request('/api/admin/settings', { method: 'PUT', body: JSON.stringify(configuration) }))
    toast.success(tx('运行配置已保存。', 'Runtime settings saved.'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('保存失败', 'Could not save settings'))
  } finally { saving.value = false }
}

async function uploadPlugin() {
  const file = selectedPlugin.value
  if (!file || !pluginManageable.value) return
  if (file.size === 0 || file.size > 16 * 1024 * 1024) {
    toast.error(tx('插件包大小必须在 1 字节至 16 MiB 之间', 'Plugin bundle must be between 1 byte and 16 MiB'))
    return
  }
  pluginBusy.value = true
  try {
    await request('/api/admin/plugins/candidate', { method: 'POST', body: file, headers: { 'Content-Type': 'application/octet-stream' } })
    selectedPlugin.value = null
    uploadOpen.value = false
    toast.success(tx('插件包已激活', 'Plugin bundle activated'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('插件包激活失败', 'Could not activate the plugin bundle'))
  } finally { pluginBusy.value = false; await loadPluginStatus() }
}

async function switchPlugin(row: PluginRecord, digest: string) {
  const revision = row.activation_revision
  if (revision == null || !digest || !pluginManageable.value || digest === row.active_digest) return
  pluginBusy.value = true
  switchBusyDigest.value = digest
  try {
    await request('/api/admin/plugins/activate-version', {
      method: 'POST',
      body: JSON.stringify({ bundle_id: row.bundle_id, expected_activation_revision: revision, target_digest: digest }),
    })
    switchTargetId.value = ''
    toast.success(tx('插件版本已切换', 'Plugin version switched'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('插件版本切换失败', 'Could not switch the plugin version'))
  } finally { pluginBusy.value = false; switchBusyDigest.value = ''; await loadPluginStatus() }
}
</script>

<template>
  <div class="content system-management-page">
    <header class="page-head"><div><h1>{{ tx('系统管理', 'System management') }}</h1></div></header>
    <div class="tabs system-tabs" role="tablist" :aria-label="tx('系统管理', 'System management')">
      <button type="button" class="tab" :class="{ active: activeTab === 'runtime' }" role="tab" :aria-selected="activeTab === 'runtime'" aria-controls="runtime-settings-panel" @click="selectTab('runtime')">{{ tx('运行配置', 'Runtime settings') }}</button>
      <button type="button" class="tab" :class="{ active: activeTab === 'plugins' }" role="tab" :aria-selected="activeTab === 'plugins'" aria-controls="upstream-plugin-panel" @click="selectTab('plugins')">{{ tx('上游适配插件', 'Upstream adapter plugin') }}</button>
    </div>

    <section v-if="activeTab === 'runtime'" id="runtime-settings-panel" class="card form-card" role="tabpanel">
      <div class="section-head"><div class="inline-actions"><h2>{{ tx('公共 API 与结算', 'Public API and billing') }}</h2><AInfoTip :text="tx('用户文档和 Runner 安装命令会读取这里的地址。', 'Member documentation and runner install commands use this address.')" /></div></div>
      <form class="form" @submit.prevent="saveConfiguration">
        <label class="field"><span class="field-label">{{ tx('公共 API 基础地址', 'Public API base URL') }}<AInfoTip :text="tx('填写部署基础地址，不要包含 /v1/responses 等接口路径。', 'Use the deployment base URL without endpoint paths such as /v1/responses.')" /></span><input v-model="configuration.public_api_base_url" type="url" :placeholder="tx('由安装器根据当前部署地址生成', 'Generated by the installer from this deployment')" required></label>
        <div class="form-actions"><AButton type="submit" :loading="saving">{{ tx('保存运行配置', 'Save runtime settings') }}</AButton></div>
      </form>
    </section>

    <section v-if="activeTab === 'runtime'" class="card form-card billing-settings">
      <div class="section-head"><h2>{{ tx('费用结算', 'Billing settlement') }}</h2></div>
      <p>{{ tx('成员余额统一使用结算货币；不同模型可用美元或人民币定价。已有成员金额账本后不可更改结算货币。汇率由本地管理员维护，最终费用以模型官方账单为准。', 'Member balances use one settlement currency; model prices may use USD or CNY. The settlement currency cannot change after a member ledger is created. The local administrator maintains the exchange rate. The model provider’s official bill is authoritative.') }}</p>
      <form class="form" @submit.prevent="saveBilling">
        <label class="field"><span>{{ tx('结算货币', 'Settlement currency') }}</span><ASelect v-model="billing.settlement_currency" :options="currencyOptions" :aria-label="tx('结算货币', 'Settlement currency')" /></label>
        <label class="field"><span>{{ tx('1 美元兑人民币', 'CNY per USD') }}<AInfoTip :text="tx('当模型价格与结算货币不同时使用；汇率在请求准入时冻结。', 'Used when a model price currency differs from the settlement currency; frozen when a request is admitted.')" /></span><input v-model.trim="billing.usd_to_cny" type="text" inputmode="decimal" placeholder="6.7"></label>
        <div class="form-actions"><AButton type="submit" :loading="billingSaving">{{ tx('保存费用配置', 'Save billing settings') }}</AButton></div>
      </form>
    </section>

    <div v-else id="upstream-plugin-panel" class="plugin-list-panel" role="tabpanel">
      <div class="section-head plugin-list-toolbar">
        <h2>{{ tx('插件版本', 'Plugin versions') }}</h2>
        <div class="inline-actions">
          <span ref="uploadActionAnchor" class="plugin-upload-action" :class="{ 'is-disabled': !!pluginDisabledReason }" :tabindex="pluginDisabledReason ? 0 : undefined" :aria-label="pluginDisabledReason ? `${tx('上传插件', 'Upload plugin')}：${pluginDisabledReason}` : undefined" @mouseenter="uploadHintOpen = !!pluginDisabledReason" @mouseleave="uploadHintOpen = false" @focusin="uploadHintOpen = !!pluginDisabledReason" @focusout="uploadHintOpen = false">
            <AButton icon="upload" :disabled="!!pluginDisabledReason || pluginBusy" @click="uploadOpen = true">{{ tx('上传插件', 'Upload plugin') }}</AButton>
          </span>
          <AFloatingPanel :open="uploadHintOpen && !!pluginDisabledReason" :anchor="uploadActionAnchor" :width="320">{{ pluginDisabledReason }}</AFloatingPanel>
        </div>
      </div>
      <div class="table-wrap plugin-version-table">
        <table class="flat-data-table"><thead><tr><th>{{ tx('名称', 'Name') }}</th><th>{{ tx('说明', 'Description') }}</th><th>{{ tx('版本', 'Version') }}</th><th>{{ tx('摘要', 'Digest') }}</th><th>{{ tx('状态', 'Status') }}</th><th class="table-action-cell">{{ tx('操作', 'Actions') }}</th></tr></thead>
          <tbody>
            <tr v-for="row in pluginRows" :key="row.bundle_id">
              <td><strong class="plugin-cell-text plugin-name" :title="displayFor(row).name">{{ displayFor(row).name }}</strong></td>
              <td><span class="plugin-cell-text plugin-description" :title="displayFor(row).description || undefined">{{ displayFor(row).description || '—' }}</span></td>
              <td><strong>{{ row.active_version || '—' }}</strong></td>
              <td><code v-if="row.active_digest" :title="row.active_digest">{{ row.active_digest.slice(0, 16) }}</code><span v-else>—</span></td>
              <td><span class="status" :class="pluginState(row).tone" :title="row.recovery_error">{{ pluginState(row).label }}</span></td>
              <td class="table-action-cell"><span class="plugin-switch-action" :class="{ 'is-disabled': !!switchDisabledReason(row) }" :tabindex="switchDisabledReason(row) ? 0 : undefined" :aria-label="switchDisabledReason(row) ? `${tx('切换版本', 'Switch version')}：${switchDisabledReason(row)}` : undefined" @mouseenter="showSwitchHint($event, row)" @mouseleave="hideSwitchHint" @focusin="showSwitchHint($event, row)" @focusout="hideSwitchHint"><AButton variant="secondary" size="small" :disabled="!!switchDisabledReason(row)" @click="switchTargetId = row.bundle_id">{{ tx('切换版本', 'Switch version') }}</AButton></span></td>
            </tr>
          </tbody>
        </table>
        <div v-if="!pluginRows.length" class="plugin-list-empty" role="status">{{ pluginStatusError ? tx('插件版本读取失败', 'Could not load plugin versions') : !pluginStatus ? tx('正在读取插件版本…', 'Loading plugin versions…') : !pluginStatus.configured ? tx('服务端尚未配置插件', 'The server has no plugin configured') : tx('暂无插件版本', 'No plugin versions yet') }}</div>
      </div>
      <AFloatingPanel :open="!!switchHintReason" :anchor="switchHintAnchor" :width="260">{{ switchHintReason }}</AFloatingPanel>
      <AModal :open="!!switchTarget" :title="switchTarget ? `${tx('切换版本', 'Switch version')} · ${displayFor(switchTarget).name}` : tx('切换版本', 'Switch version')" :close-label="tx('关闭', 'Close')" :close-disabled="pluginBusy" wide @close="switchTargetId = ''">
        <div v-if="switchTarget" class="table-wrap plugin-switch-table">
          <table class="flat-data-table"><thead><tr><th>{{ tx('版本', 'Version') }}</th><th>{{ tx('摘要', 'Digest') }}</th><th>{{ tx('状态', 'Status') }}</th><th class="table-action-cell">{{ tx('操作', 'Actions') }}</th></tr></thead>
            <tbody><tr v-for="version in versionRows" :key="version.digest">
              <td>{{ version.version || '—' }}</td>
              <td><code :title="version.digest">{{ version.digest.slice(0, 16) }}</code></td>
              <td><span v-if="version.digest === switchTarget.active_digest" class="status">{{ tx('当前使用', 'Current') }}</span><span v-else class="status" :class="version.available ? 'neutral' : 'off'">{{ version.available ? tx('已归档', 'Archived') : tx('不可用', 'Unavailable') }}</span></td>
              <td class="table-action-cell"><AButton v-if="version.available && version.digest !== switchTarget.active_digest" variant="secondary" size="small" :disabled="pluginBusy" :loading="pluginBusy && switchBusyDigest === version.digest" @click="switchPlugin(switchTarget, version.digest)">{{ tx('切换', 'Switch') }}</AButton><span v-else>—</span></td>
            </tr></tbody>
          </table>
        </div>
      </AModal>
      <AModal :open="uploadOpen" :title="tx('上传上游适配插件', 'Upload upstream adapter plugin')" :close-label="tx('关闭', 'Close')" :close-disabled="pluginBusy" wide @close="closeUpload">
        <div class="file-upload-row">
          <AFilePicker v-model="selectedPlugin" :label="tx('选择签名插件包', 'Select signed plugin bundle')" :empty-label="tx('点击选择或拖入 .asterlua 文件', 'Choose or drop an .asterlua file')" :clear-label="tx('清除插件包', 'Clear plugin bundle')" accept=".asterlua,application/octet-stream" :disabled="!pluginManageable" :loading="pluginBusy" />
          <AButton icon="upload" :disabled="!pluginManageable || !selectedPlugin" :loading="pluginBusy" @click="uploadPlugin">{{ tx('上传并激活', 'Upload and activate') }}</AButton>
        </div>
      </AModal>
    </div>
  </div>
</template>

<style scoped>
.system-management-page{display:grid;gap:12px;align-content:start}
.system-management-page>.page-head{margin-bottom:4px}
.system-tabs{width:max-content;max-width:100%;margin-bottom:4px}
.field-label{display:flex;align-items:center;gap:4px}
.plugin-list-panel{display:grid;gap:12px;align-content:start}
.plugin-list-toolbar{margin:4px 0 0}
.plugin-upload-action{display:inline-flex;align-items:center}
.plugin-upload-action.is-disabled{cursor:help}
.plugin-upload-action:focus-visible{outline:2px solid var(--accent);outline-offset:3px;border-radius:10px}
.plugin-switch-action{display:inline-flex;align-items:center}
.plugin-switch-action.is-disabled{cursor:help}
.plugin-switch-action:focus-visible{outline:2px solid var(--accent);outline-offset:3px;border-radius:10px}
.plugin-version-table table{min-width:980px}
.plugin-cell-text{display:block;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.plugin-name{max-width:240px}
.plugin-description{max-width:350px;color:var(--muted);font-size:var(--font-size-body)}
.plugin-version-table td{vertical-align:middle}
.plugin-version-table td code{font:var(--font-size-body) var(--font-mono)}
.plugin-version-table .table-action-cell{width:140px}
.plugin-switch-table table{min-width:560px}
.plugin-switch-table td code{font:var(--font-size-body) var(--font-mono)}
.plugin-switch-table .table-action-cell{width:120px}
.plugin-list-empty{padding:24px;color:var(--muted);text-align:center}
@media(max-width:600px){.plugin-list-toolbar{align-items:flex-start;flex-direction:column}.plugin-list-toolbar>.inline-actions{width:100%;justify-content:space-between}}
</style>
