<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('gateway')
import { computed, onMounted, ref, watch } from 'vue'
import { AButton, AConfirmModal, AFloatingPanel, AIconButton, AInfoTip, AModal, APagination, ASelect, useToast } from '@aster/ui'
import { BUSINESS_OPERATIONS, CAPABILITIES, formatDate, licenseOperationAvailable, request, type BusinessOperationId } from '@aster/sdk'
import { locale } from '../i18n'
import { adminProfile } from '../license-status'

type UpstreamAccount = {
  id: string
  provider: string
  email: string
  plan: string
  status: string
  credential_count: number
  active_credential_count: number
  last_success_runner_id?: string
  last_verified_at?: string
  created_at: string
  updated_at: string
}

type UpstreamProvider = {
  id: string
  display_name: string
  enrollment_actions: string[]
}

type ApiKeyChannel = {
  id: string
  display_name: string
  provider: string
  endpoint_profile: string
  enrollment_kind: 'api_key'
  billing_mode: string
  default_base_url: string
  native_protocols: string[]
  supports_model_discovery: boolean
}

type ModelDraft = {
  publicName: string
  upstreamName: string
  quotaUnit: 'token' | 'image'
}

type ApiKeyConnection = {
  id: string
  provider: string
  channel_id: string
  display_name: string
  billing_mode: string
  status: string
  revision: number
  created_at: string
  updated_at: string
}

type ConnectionRow =
  | { kind: 'subscription'; id: string; updatedAt: string; source: UpstreamAccount }
  | { kind: 'api_key'; id: string; updatedAt: string; source: ApiKeyConnection }

type PluginStatus = {
  configured: boolean
  plugins: { provider: string; active_digest?: string; recovery_error?: string }[]
}

type ApiKeyConnectionProbe = {
  auth_observation: 'passed' | 'failed' | 'unknown'
  discovery: 'passed' | 'unsupported' | 'invalid_response' | 'unauthorized' | 'rate_limited' | 'upstream_error'
  generation: 'untested'
  model_ids: string[]
  upstream_status?: number
}

const pluginStatus = ref<PluginStatus | null>(null)
const pluginReady = computed(() => {
  const plugin = pluginStatus.value?.plugins.find(item => item.provider === apiChannel.value.split('.')[0])
  return !!plugin?.active_digest && !plugin.recovery_error
})
const apiConnectionActionAnchor = ref<HTMLElement | null>(null)
const apiConnectionHintOpen = ref(false)

const apiConnections = ref<ApiKeyConnection[]>([])
const page = ref(1)
const pageSize = ref(50)
const keyword = ref('')
const categoryFilter = ref('')
const providerFilter = ref('')
const statusFilter = ref('')
const connectionObservations = ref<Record<string, ApiKeyConnectionProbe>>({})
const editingConnection = ref<ApiKeyConnection | null>(null)
const manualTarget = ref<ApiKeyConnection | null>(null)
const editName = ref('')
const editKey = ref('')
const apiKeyOpen = ref(false)
const apiChannel = ref('deepseek.api')
const apiDisplayName = ref('')
const apiKey = ref('')
const apiKeyVisible = ref(false)
const modelMode = ref<'automatic' | 'manual'>('automatic')
const modelRows = ref<ModelDraft[]>([{ publicName: '', upstreamName: '', quotaUnit: 'token' }])
const apiChannels = ref<ApiKeyChannel[]>([])
const apiConnectionDisabledReason = computed(() => {
  if (!canWrite.value) return tx('当前授权不允许新增 API Key 连接', 'The current license does not allow adding API Key connections')
  if (!pluginStatus.value) return tx('正在读取接入配置', 'Loading connection settings')
  if (!pluginReady.value) return pluginStatus.value.configured
    ? tx('上游适配插件尚未激活，请在系统管理中检查插件状态', 'The upstream adapter plugin is not active. Check its status in System management')
    : tx('服务端尚未配置上游适配插件，请联系部署管理员完成配置', 'The upstream adapter plugin is not configured. Ask the deployment administrator to configure it')
  if (!apiChannels.value.length) return tx('暂无可用的 API Key 接入通道', 'No API Key channels are available')
  return ''
})
const selectedApiChannel = computed(() => apiChannels.value.find(channel => channel.id === apiChannel.value))
const channelOptions = computed(() => apiChannels.value.map(channel => ({ value: channel.id, label: channel.display_name })))
const categoryOptions = computed(() => [
  { value: '', label: tx('全部类别', 'All types') },
  { value: 'subscription', label: tx('订阅账号', 'Subscription accounts') },
  { value: 'api_key', label: tx('API Key 连接', 'API Key connections') },
])
const providerOptions = computed(() => [
  { value: '', label: tx('全部提供方', 'All providers') },
  ...Array.from(new Set([...items.value.map(account => account.provider), ...apiConnections.value.map(connection => connection.provider)]))
    .sort((a, b) => providerLabel(a).localeCompare(providerLabel(b)))
    .map(provider => ({ value: provider, label: providerLabel(provider) })),
])
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'active', label: tx('可路由 / 已启用', 'Routable / enabled') },
  { value: 'unavailable', label: tx('凭据不可用', 'No active credential') },
  { value: 'disabled', label: tx('已停用', 'Disabled') },
])
const filteredAccounts = computed(() => {
  const needle = keyword.value.trim().toLocaleLowerCase()
  if (categoryFilter.value === 'api_key') return []
  return items.value.filter(account => {
    const state = account.status === 'disabled' ? 'disabled' : account.status === 'active' && account.active_credential_count ? 'active' : 'unavailable'
    return (!needle || `${account.email} ${account.id} ${account.provider} ${account.plan}`.toLocaleLowerCase().includes(needle))
      && (!providerFilter.value || account.provider === providerFilter.value)
      && (!statusFilter.value || state === statusFilter.value)
  })
})
const filteredApiConnections = computed(() => {
  const needle = keyword.value.trim().toLocaleLowerCase()
  if (categoryFilter.value === 'subscription') return []
  return apiConnections.value.filter(connection =>
    (!needle || `${connection.display_name} ${connection.id} ${connection.provider} ${connection.channel_id} ${apiChannelLabel(connection.channel_id)}`.toLocaleLowerCase().includes(needle))
    && (!providerFilter.value || connection.provider === providerFilter.value)
    && (!statusFilter.value || (connection.status === 'active' ? 'active' : 'disabled') === statusFilter.value))
})
const filteredRows = computed<ConnectionRow[]>(() => [
  ...filteredAccounts.value.map(account => ({ kind: 'subscription' as const, id: account.id, updatedAt: account.updated_at, source: account })),
  ...filteredApiConnections.value.map(connection => ({ kind: 'api_key' as const, id: connection.id, updatedAt: connection.updated_at, source: connection })),
].sort((left, right) => Date.parse(right.updatedAt) - Date.parse(left.updatedAt) || left.id.localeCompare(right.id)))
const pagedRows = computed(() => filteredRows.value.slice((page.value - 1) * pageSize.value, page.value * pageSize.value))
function resetFilters() {
  keyword.value = ''
  categoryFilter.value = ''
  providerFilter.value = ''
  statusFilter.value = ''
}

type EnrollmentTransition =
  | { state: 'action_required'; enrollment_id: string; provider_id: string; action: { type: 'open_url'; url: string; expires_at: string; callback_action: 'submit_callback' } }
  | { state: 'completed'; enrollment_id: string; provider_id: string; account_id: string; credential_instance_id: string; email: string; plan: string }

const items = ref<UpstreamAccount[]>([])
const providers = ref<UpstreamProvider[]>([])
const toast = useToast()
const oauthOpen = ref(false)
const authorizationURL = ref('')
const callbackURL = ref('')
const enrollmentID = ref('')
const loading = ref(false)
const operatingID = ref('')
const pendingRemoval = ref<UpstreamAccount | null>(null)
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const date = (value?: string) => formatDate(value, locale.value)
const shortID = (value?: string) => value ? `${value.slice(0, value.indexOf('_') + 1)}…${value.slice(-8)}` : '—'
const canAuthorize = computed(() => licenseOperationAvailable(adminProfile.value?.license, 'upstream_authorize'))
const canSync = computed(() => licenseOperationAvailable(adminProfile.value?.license, 'upstream_sync'))
const unavailableOperations = computed(() => BUSINESS_OPERATIONS.filter(operation =>
  adminProfile.value?.license?.state === 'active'
  && ['upstream_authorize', 'upstream_sync'].includes(operation.id)
  && !licenseOperationAvailable(adminProfile.value?.license, operation.id)))
function operationRequirement(operation: BusinessOperationId) {
  return BUSINESS_OPERATIONS.find(entry => entry.id === operation)?.requires
    .map(id => CAPABILITIES.find(feature => feature.id === id)?.label ?? id).join(' + ')
}

async function load() {
  loading.value = true
  try {
    const [accounts, availableProviders, connections, plugins] = await Promise.all([
      request<{ items: UpstreamAccount[] }>('/api/admin/upstream-accounts'),
      request<{ items: UpstreamProvider[]; api_key_channels: ApiKeyChannel[] }>('/api/admin/upstream-providers'),
      request<{ items: ApiKeyConnection[] }>('/api/admin/upstream-connections'),
      request<PluginStatus>('/api/admin/plugins/status'),
    ])
    items.value = accounts.items
    providers.value = availableProviders.items
    apiChannels.value = availableProviders.api_key_channels
    if (!apiChannels.value.some(channel => channel.id === apiChannel.value)) apiChannel.value = apiChannels.value[0]?.id ?? ''
    apiConnections.value = connections.items
    connectionObservations.value = {}
    pluginStatus.value = plugins
  }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('订阅/账号加载失败', 'Could not load subscription accounts')) }
  finally { loading.value = false }
}

function openApiConnectionModal() {
  modelMode.value = selectedApiChannel.value?.supports_model_discovery && canSync.value ? 'automatic' : 'manual'
  modelRows.value = [{ publicName: '', upstreamName: '', quotaUnit: 'token' }]
  apiKeyVisible.value = false
  apiKeyOpen.value = true
}

function selectApiChannel(value: string | number) {
  apiChannel.value = String(value)
  modelMode.value = selectedApiChannel.value?.supports_model_discovery && canSync.value ? 'automatic' : 'manual'
  modelRows.value = [{ publicName: '', upstreamName: '', quotaUnit: 'token' }]
}

function addModelRow() {
  if (modelRows.value.length < 64) modelRows.value.push({ publicName: '', upstreamName: '', quotaUnit: 'token' })
}

function quotaOptions(provider: string) {
  const options = [{ value: 'token', label: tx('按 Token', 'By token') }]
  if (provider !== 'deepseek') options.push({ value: 'image', label: tx('按张', 'Per image') })
  return options
}

function modelPayload(provider: string) {
  const models = modelRows.value.map(row => ({
    public_name: row.publicName.trim(),
    upstream_name: row.upstreamName.trim(),
    display_name: row.publicName.trim(),
    quota_unit: row.quotaUnit,
  }))
  const validName = (value: string) => /^[A-Za-z0-9._:/-]{1,160}$/.test(value)
  if (models.length === 0 || models.length > 64
    || models.some(model => !validName(model.public_name) || !validName(model.upstream_name)
      || (provider === 'deepseek' && model.quota_unit === 'image'))
    || new Set(models.map(model => model.public_name)).size !== models.length
    || new Set(models.map(model => model.upstream_name)).size !== models.length) {
    toast.error(tx('请检查模型名称、上游模型 ID 和额度类型', 'Check model names, upstream IDs, and quota types'))
    return null
  }
  return models
}

function openManualModels(connection: ApiKeyConnection) {
  modelRows.value = [{ publicName: '', upstreamName: '', quotaUnit: 'token' }]
  manualTarget.value = connection
}

function providerLabel(provider: string) {
  return ({ openai: 'OpenAI', deepseek: 'DeepSeek', glm: 'GLM' } as Record<string, string>)[provider] ?? provider
}

function connectionProviderLabel(connection: ApiKeyConnection) {
  if (connection.channel_id.startsWith('glm.bigmodel.')) return 'GLM · BigModel'
  if (connection.channel_id.startsWith('glm.zai.')) return 'GLM · Z.AI'
  return providerLabel(connection.provider)
}

function apiChannelLabel(channelID: string) {
  return apiChannels.value.find(channel => channel.id === channelID)?.display_name ?? channelID
}

function billingLabel(mode: string) {
  if (mode === 'coding_plan') return 'Coding Plan'
  if (mode === 'usage') return tx('按量 API', 'Usage API')
  return tx('未知计费方式', 'Unknown billing')
}

function discoveryLabel(value?: ApiKeyConnectionProbe['discovery']) {
  if (!value) return '—'
  const labels: Record<ApiKeyConnectionProbe['discovery'], [string, string]> = {
    passed: ['通过', 'Passed'], unsupported: ['不支持', 'Unsupported'],
    invalid_response: ['响应异常', 'Invalid response'], unauthorized: ['认证失败', 'Unauthorized'],
    rate_limited: ['请求受限', 'Rate limited'], upstream_error: ['上游错误', 'Upstream error'],
  }
  return tx(...labels[value])
}

async function createApiConnection() {
  if (!canWrite.value) return
  const automatic = modelMode.value === 'automatic'
  if (automatic && (!selectedApiChannel.value?.supports_model_discovery || !canSync.value)) {
    toast.error(tx('当前通道或授权不支持自动同步模型', 'Model discovery is unavailable for this channel or license'))
    return
  }
  const models = automatic ? [] : modelPayload(selectedApiChannel.value?.provider ?? '')
  if (!models) return
  loading.value = true
  try {
    const created = await request<{ id: string; revision: number }>('/api/admin/upstream-connections', {
      method: 'POST',
      body: JSON.stringify({ channel_id: apiChannel.value, display_name: apiDisplayName.value.trim(), api_key: apiKey.value.trim(), models }),
    })
    apiKey.value = ''
    apiDisplayName.value = ''
    apiKeyOpen.value = false
    if (automatic) {
      try {
        const result = await request<{ status: string; added: number }>(`/api/admin/upstream-connections/${created.id}/models/sync`, {
          method: 'POST', body: JSON.stringify({ expected_revision: created.revision }),
        })
        if (result.status === 'synced') {
          toast.success(tx(`连接已创建，发现 ${result.added} 个新模型。请在模型管理中启用需要开放的模型。`,
            `Connection created with ${result.added} new models. Enable the public models you need in Model Management.`))
        } else {
          toast.warning(tx('连接已创建，但该通道未提供模型列表。请手动添加模型。',
            'Connection created, but this channel has no model list. Add models manually.'))
        }
      } catch (value) {
        const detail = value instanceof Error ? value.message : tx('模型同步失败', 'model sync failed')
        toast.warning(tx(`连接已创建，但模型同步失败：${detail}。可使用操作列添加模型。`, `Connection created, but model sync failed: ${detail}. Use Add models in the actions column.`), 7_000)
      }
    } else {
      toast.success(tx('连接已创建。请在模型管理中启用需要开放的模型。', 'Connection created. Enable the public models you need in Model Management.'))
    }
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('创建 API Key 连接失败', 'Could not create the API Key connection'))
  } finally { loading.value = false }
}

async function addManualModels() {
  const connection = manualTarget.value
  if (!connection || !canWrite.value) return
  const models = modelPayload(connection.provider)
  if (!models) return
  operatingID.value = connection.id
  try {
    const result = await request<{ added: number }>(`/api/admin/upstream-connections/${connection.id}/models`, {
      method: 'POST',
      body: JSON.stringify({ expected_revision: connection.revision, models }),
    })
    manualTarget.value = null
    toast.success(tx(`已添加 ${result.added} 个模型，请在模型管理中启用。`, `Added ${result.added} models. Enable them in Model Management.`))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('添加模型失败', 'Could not add models'))
  } finally { operatingID.value = '' }
}

async function toggleApiConnection(connection: ApiKeyConnection) {
  if (!canWrite.value) return
  operatingID.value = connection.id
  try {
    await request(`/api/admin/upstream-connections/${connection.id}`, {
      method: 'PATCH',
      body: JSON.stringify({ expected_revision: connection.revision, enabled: connection.status !== 'active' }),
    })
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('连接状态更新失败', 'Could not update connection status'))
  } finally { operatingID.value = '' }
}

async function verifyApiConnection(connection: ApiKeyConnection) {
  if (!canSync.value) return
  operatingID.value = connection.id
  try {
    const result = await request<ApiKeyConnectionProbe>(`/api/admin/upstream-connections/${connection.id}/verify`, { method: 'POST' })
    connectionObservations.value[connection.id] = result
    if (result.discovery === 'unsupported') {
      toast.success(tx('该通道没有已确认的非生成模型列表接口，请手动配置模型。', 'This channel has no confirmed non-generation model list API. Configure models manually.'))
    } else if (result.discovery === 'passed') {
      toast.success(tx(`认证与模型列表验证通过，发现 ${result.model_ids.length} 个模型；生成能力尚未测试。`, `Authentication and model listing passed; ${result.model_ids.length} models found. Generation remains untested.`))
    } else {
      toast.error(tx(`连接验证结果：${result.discovery}；生成能力尚未测试。`, `Connection observation: ${result.discovery}; generation remains untested.`))
    }
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('连接验证失败', 'Could not verify connection'))
  } finally { operatingID.value = '' }
}

async function syncApiConnectionModels(connection: ApiKeyConnection) {
  if (!canSync.value) return
  operatingID.value = connection.id
  try {
    const result = await request<{ status: 'synced' | 'unsupported'; added: number; discovered?: number }>(
      `/api/admin/upstream-connections/${connection.id}/models/sync`, {
        method: 'POST', body: JSON.stringify({ expected_revision: connection.revision }),
      })
    if (result.status === 'unsupported') {
      toast.success(tx('该通道需手动配置模型。', 'Configure models manually for this channel.'))
    } else {
      toast.success(tx(`发现 ${result.discovered ?? 0} 个模型，新增 ${result.added} 个默认禁用的公开模型。`, `Found ${result.discovered ?? 0} models and added ${result.added} disabled public models.`))
    }
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('模型同步失败', 'Could not synchronize models'))
  } finally { operatingID.value = '' }
}

function openEditConnection(connection: ApiKeyConnection) {
  editingConnection.value = connection
  editName.value = connection.display_name
  editKey.value = ''
}

function closeEditConnection() {
  editingConnection.value = null
  editName.value = ''
  editKey.value = ''
}

async function saveApiConnection() {
  const connection = editingConnection.value
  if (!connection || !canWrite.value) return
  const name = editName.value.trim()
  const key = editKey.value.trim()
  if (!name) return
  if (name === connection.display_name && !key) {
    closeEditConnection()
    return
  }
  operatingID.value = connection.id
  try {
    await request(`/api/admin/upstream-connections/${connection.id}`, {
      method: 'PATCH',
      body: JSON.stringify({ expected_revision: connection.revision, ...(name !== connection.display_name ? { display_name: name } : {}), ...(key ? { api_key: key } : {}) }),
    })
    closeEditConnection()
    toast.success(tx('连接已更新', 'Connection updated'))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('编辑连接失败', 'Could not edit the connection'))
  } finally { operatingID.value = '' }
}

async function startOAuth() {
  if (!canAuthorize.value) return
  loading.value = true
  try {
    const provider = providers.value.find(item => item.id === 'openai')
    if (!provider) throw new Error(tx('当前版本没有可用的 OpenAI 接入驱动', 'No OpenAI enrollment driver is available in this version.'))
    const result = await request<EnrollmentTransition>(
      `/api/admin/upstream-providers/${provider.id}/enrollments`, { method: 'POST', body: '{}' },
    )
    if (result.state !== 'action_required' || result.action.type !== 'open_url' || result.action.callback_action !== 'submit_callback') {
      throw new Error(tx('上游提供方返回了不支持的接入步骤', 'The provider returned an unsupported enrollment action.'))
    }
    authorizationURL.value = result.action.url
    enrollmentID.value = result.enrollment_id
    callbackURL.value = ''
    oauthOpen.value = true
    window.open(result.action.url, '_blank', 'noopener,noreferrer')
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('无法发起 ChatGPT 授权', 'Could not start ChatGPT authorization')) }
  finally { loading.value = false }
}

async function completeOAuth() {
  if (!canAuthorize.value) return
  loading.value = true
  try {
    const expectedState = new URL(authorizationURL.value).searchParams.get('state')
    const pastedState = new URL(callbackURL.value.trim()).searchParams.get('state')
    if (!expectedState || expectedState !== pastedState) throw new Error(tx('粘贴的回调不属于本次登录，请重新打开当前登录链接', 'The pasted callback does not belong to this sign-in. Reopen the current login link.'))
    const result = await request<EnrollmentTransition>(`/api/admin/upstream-enrollments/${enrollmentID.value}/actions`, {
      method: 'POST', body: JSON.stringify({ type: 'submit_callback', callback_url: callbackURL.value }),
    })
    if (result.state !== 'completed') throw new Error(tx('上游接入尚未完成', 'Provider enrollment is not complete.'))
    try {
      const synced = await request<{ synced: number }>(`/api/admin/upstream-accounts/${result.account_id}/models/sync`, { method: 'POST' })
      toast.success(tx(`${result.email} 已新增一份独立凭据，并同步 ${synced.synced} 个模型。`, `${result.email} received a new independent credential; ${synced.synced} models synchronized.`))
    } catch (value) {
      const reason = value instanceof Error ? value.message : tx('模型同步失败', 'Model synchronization failed')
      toast.error(tx(`${result.email} 的凭据已保存。${reason}，请处理后重新同步模型。`, `The credential for ${result.email} was saved. ${reason}. Resolve this before synchronizing models again.`), 8_000)
    }
    oauthOpen.value = false
    enrollmentID.value = ''
    authorizationURL.value = ''
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('ChatGPT 授权未完成', 'ChatGPT authorization was not completed')) }
  finally { loading.value = false }
}

async function cancelEnrollment() {
  const current = enrollmentID.value
  oauthOpen.value = false
  enrollmentID.value = ''
  authorizationURL.value = ''
  callbackURL.value = ''
  if (!current) return
  try { await request(`/api/admin/upstream-enrollments/${current}`, { method: 'DELETE' }) }
  catch { /* Expired or already consumed enrollment sessions need no further action. */ }
}

async function syncModels(account: UpstreamAccount) {
  if (!canSync.value) return
  operatingID.value = account.id
  try {
    const result = await request<{ synced: number }>(`/api/admin/upstream-accounts/${account.id}/models/sync`, { method: 'POST' })
    toast.success(tx(`${account.email} 已同步 ${result.synced} 个模型。`, `${result.synced} models synchronized for ${account.email}.`))
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('模型同步失败', 'Model synchronization failed')) }
  finally { operatingID.value = '' }
}

async function toggle(account: UpstreamAccount) {
  operatingID.value = account.id
  try {
    const enabled = account.status !== 'active'
    await request(`/api/admin/upstream-accounts/${account.id}`, { method: 'PATCH', body: JSON.stringify({ enabled }) })
    toast.success(enabled ? tx(`${account.email} 已启用。`, `${account.email} enabled.`) : tx(`${account.email} 已停用。`, `${account.email} disabled.`))
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('账号状态更新失败', 'Could not update account status')) }
  finally { operatingID.value = '' }
}

async function remove(account: UpstreamAccount) {
  operatingID.value = account.id
  try {
    const result = await request<{ deleted_credentials: number }>(`/api/admin/upstream-accounts/${account.id}`, { method: 'DELETE' })
    pendingRemoval.value = null
    toast.success(tx(`${account.email} 已删除，同时清除 Control 中的 ${result.deleted_credentials} 份加密凭据。Runner 不持久化账号凭据。`, `${account.email} was deleted together with ${result.deleted_credentials} encrypted Control credential(s). Runners do not persist account credentials.`), 6_000)
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('账号删除失败', 'Could not delete account')) }
  finally { operatingID.value = '' }
}

watch([keyword, categoryFilter, providerFilter, statusFilter, pageSize], () => { page.value = 1 })
watch(filteredRows, rows => { page.value = Math.min(page.value, Math.max(1, Math.ceil(rows.length / pageSize.value))) })
onMounted(() => { void load() })
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head">
      <div><h1>{{ tx('订阅/账号', 'Subscriptions & accounts') }}</h1></div>
      <div class="inline-actions">
        <span ref="apiConnectionActionAnchor" class="api-connection-action" :class="{ 'is-disabled': !!apiConnectionDisabledReason }" :tabindex="apiConnectionDisabledReason ? 0 : undefined" :aria-label="apiConnectionDisabledReason ? `${tx('新增 API Key 连接', 'Add API Key connection')}：${apiConnectionDisabledReason}` : undefined" @mouseenter="apiConnectionHintOpen = !!apiConnectionDisabledReason" @mouseleave="apiConnectionHintOpen = false" @focusin="apiConnectionHintOpen = !!apiConnectionDisabledReason" @focusout="apiConnectionHintOpen = false"><AButton variant="secondary" icon="plus" :disabled="!!apiConnectionDisabledReason" @click="openApiConnectionModal">{{ tx('新增 API Key 连接', 'Add API Key connection') }}</AButton></span>
        <AFloatingPanel :open="apiConnectionHintOpen && !!apiConnectionDisabledReason" :anchor="apiConnectionActionAnchor" :width="320">{{ apiConnectionDisabledReason }}</AFloatingPanel>
        <AButton icon="plus" :loading="loading" :disabled="!canAuthorize" @click="startOAuth">{{ tx('新增 ChatGPT 账号', 'Add ChatGPT account') }}</AButton>
      </div>
    </header>
    <div v-if="unavailableOperations.length" class="notice" role="status">
      <p v-for="operation in unavailableOperations" :key="operation.id">{{ operation.label }}{{ tx('需要以下授权功能', ' requires these licensed features') }} {{ operationRequirement(operation.id) }}</p>
    </div>
    <div class="filter-bar connection-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :aria-label="tx('搜索订阅账号和 API Key 连接', 'Search subscription accounts and API Key connections')" :placeholder="tx('账号、连接名称或 ID', 'Account, connection name, or ID')"></label>
      <label class="field"><span>{{ tx('类别', 'Type') }}</span><ASelect v-model="categoryFilter" :options="categoryOptions" :aria-label="tx('类别', 'Type')" /></label>
      <label class="field"><span>{{ tx('提供方', 'Provider') }}</span><ASelect v-model="providerFilter" :options="providerOptions" :aria-label="tx('提供方', 'Provider')" /></label>
      <label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="statusFilter" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>
    <section class="section" :aria-label="tx('订阅与账号列表', 'Subscriptions and accounts list')">
      <div class="table-wrap connection-table-wrap"><table class="flat-data-table connection-table"><thead><tr>
        <th>{{ tx('账号或连接', 'Account or connection') }}</th><th>{{ tx('类别', 'Type') }}</th><th>{{ tx('提供方', 'Provider') }}</th><th>{{ tx('接入方式', 'Access method') }}</th><th>{{ tx('套餐', 'Plan') }}</th><th>{{ tx('计费方式', 'Billing') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('凭据池', 'Credentials') }}</th><th>{{ tx('最近成功 Runner', 'Last successful Runner') }}</th><th>{{ tx('最近验证', 'Last verified') }}</th><th>{{ tx('模型检查', 'Model check') }}</th><th>{{ tx('创建时间', 'Created') }}</th><th>{{ tx('更新时间', 'Updated') }}</th><th>{{ tx('ID', 'ID') }}</th><th class="table-action-cell">{{ tx('操作', 'Actions') }}</th>
      </tr></thead><tbody><tr v-for="row in pagedRows" :key="`${row.kind}:${row.id}`">
        <td><strong :title="row.kind === 'subscription' ? row.source.email : row.source.display_name">{{ row.kind === 'subscription' ? row.source.email : row.source.display_name }}</strong></td>
        <td><span class="status neutral">{{ row.kind === 'subscription' ? tx('订阅账号', 'Subscription account') : tx('API Key 连接', 'API Key connection') }}</span></td>
        <td><span class="status neutral" :title="row.kind === 'api_key' ? apiChannelLabel(row.source.channel_id) : undefined">{{ row.kind === 'subscription' ? providerLabel(row.source.provider) : connectionProviderLabel(row.source) }}</span></td>
        <td><span class="status neutral">{{ row.kind === 'subscription' ? tx('订阅登录', 'Subscription login') : 'API Key' }}</span></td>
        <td><span v-if="row.kind === 'subscription'" class="status neutral">{{ row.source.plan || '—' }}</span><span v-else>—</span></td>
        <td><span v-if="row.kind === 'api_key'" class="status neutral">{{ billingLabel(row.source.billing_mode) }}</span><span v-else>—</span></td>
        <td><span v-if="row.kind === 'subscription'" class="status" :class="{ off: row.source.status !== 'active' || !row.source.active_credential_count }">{{ row.source.status === 'disabled' ? tx('已停用', 'Disabled') : row.source.status !== 'active' || !row.source.active_credential_count ? tx('凭据不可用', 'No active credential') : tx('可路由', 'Routable') }}</span><span v-else class="status" :class="{ off: row.source.status !== 'active' }">{{ row.source.status === 'active' ? tx('已启用', 'Enabled') : tx('已停用', 'Disabled') }}</span></td>
        <td>{{ row.kind === 'subscription' ? `${row.source.active_credential_count} / ${row.source.credential_count}` : '—' }}</td>
        <td><code v-if="row.kind === 'subscription'" :title="row.source.last_success_runner_id">{{ shortID(row.source.last_success_runner_id) }}</code><span v-else>—</span></td>
        <td>{{ row.kind === 'subscription' ? date(row.source.last_verified_at) : '—' }}</td>
        <td><template v-if="row.kind === 'api_key'"><span v-if="connectionObservations[row.id]" class="status" :class="connectionObservations[row.id]?.discovery === 'passed' ? 'neutral' : 'warning'">{{ discoveryLabel(connectionObservations[row.id]?.discovery) }}</span><span v-else>—</span></template><span v-else>—</span></td>
        <td>{{ date(row.source.created_at) }}</td><td>{{ date(row.source.updated_at) }}</td><td><code :title="row.id">{{ shortID(row.id) }}</code></td>
        <td class="table-action-cell">
          <div v-if="row.kind === 'subscription'" class="row-actions"><AIconButton icon="sync" size="small" :label="tx('同步订阅账号模型', 'Sync subscription account models')" :disabled="!canSync || !!operatingID || row.source.status !== 'active'" @click="syncModels(row.source)" /><AIconButton :icon="row.source.status === 'active' ? 'pause' : 'play'" size="small" :label="row.source.status === 'active' ? tx('停用订阅账号', 'Disable subscription account') : tx('启用订阅账号', 'Enable subscription account')" :disabled="(!canWrite && row.source.status !== 'active') || !!operatingID" @click="toggle(row.source)" /><AIconButton icon="trash" size="small" variant="danger" :label="tx('删除订阅账号', 'Delete subscription account')" :disabled="!!operatingID" @click="pendingRemoval = row.source" /></div>
          <div v-else class="row-actions"><AIconButton icon="test" size="small" :label="tx('验证 API Key 连接', 'Verify API Key connection')" :disabled="!canSync || !!operatingID || row.source.status !== 'active'" @click="verifyApiConnection(row.source)" /><AIconButton v-if="apiChannels.find(channel => channel.id === row.source.channel_id)?.supports_model_discovery" icon="sync" size="small" :label="tx('同步连接模型', 'Sync connection models')" :disabled="!canSync || !!operatingID || row.source.status !== 'active'" @click="syncApiConnectionModels(row.source)" /><AIconButton icon="plus" size="small" :label="tx('为连接添加模型', 'Add models to connection')" :disabled="!canWrite || !!operatingID || row.source.status !== 'active'" @click="openManualModels(row.source)" /><AIconButton :icon="row.source.status === 'active' ? 'pause' : 'play'" size="small" :label="row.source.status === 'active' ? tx('停用 API Key 连接', 'Disable API Key connection') : tx('启用 API Key 连接', 'Enable API Key connection')" :disabled="!canWrite || !!operatingID" @click="toggleApiConnection(row.source)" /><AIconButton icon="edit" size="small" :label="tx('编辑 API Key 连接', 'Edit API Key connection')" :disabled="!canWrite || !!operatingID" @click="openEditConnection(row.source)" /></div>
        </td>
      </tr></tbody></table>
        <div v-if="!filteredRows.length" class="connection-list-empty" role="status">{{ loading && !items.length && !apiConnections.length ? tx('正在读取连接…', 'Loading connections…') : items.length || apiConnections.length ? tx('没有匹配的连接', 'No matching connections') : tx('暂无订阅账号或 API Key 连接', 'No subscription accounts or API Key connections yet') }}</div>
      </div>
      <APagination v-if="filteredRows.length" v-model:page="page" v-model:page-size="pageSize" :total="filteredRows.length" :locale="locale" />
    </section>

    <AModal :open="apiKeyOpen" :title="tx('新增 API Key 连接', 'Add API Key connection')" :close-label="tx('关闭', 'Close')" :close-disabled="loading" wide @close="apiKeyOpen = false">
      <form class="form" @submit.prevent="createApiConnection">
        <div class="field"><span>{{ tx('接入通道', 'Channel') }}</span><ASelect class="connection-channel-select" :model-value="apiChannel" :options="channelOptions" :aria-label="tx('接入通道', 'Channel')" required @update:model-value="selectApiChannel" /><small>{{ tx('按通道使用对应的认证与计费方式', 'Authentication and billing follow the selected channel') }}</small></div>
        <label class="field"><span>{{ tx('连接名称', 'Connection name') }}</span><input v-model="apiDisplayName" maxlength="160" required></label>
        <div class="field"><label for="new-upstream-api-key">API Key</label><div class="connection-secret-field"><input id="new-upstream-api-key" v-model="apiKey" :type="apiKeyVisible ? 'text' : 'password'" autocomplete="off" required><span class="connection-secret-toggle"><AIconButton :icon="apiKeyVisible ? 'eye-off' : 'eye'" :label="apiKeyVisible ? tx('隐藏 API Key', 'Hide API Key') : tx('显示 API Key', 'Show API Key')" size="small" @click="apiKeyVisible = !apiKeyVisible" /></span></div><small>{{ tx('密钥仅用于连接上游，不会回显', 'The key is used for the upstream connection and is never shown again') }}</small></div>
        <section class="connection-model-setup"><h3>{{ tx('模型接入', 'Model setup') }}</h3>
          <div v-if="selectedApiChannel?.supports_model_discovery && canSync" class="connection-model-choices">
            <label class="connection-model-choice" :class="{ selected: modelMode === 'automatic' }"><input v-model="modelMode" type="radio" value="automatic"><span><strong>{{ tx('创建后自动同步模型', 'Sync models after creation') }}</strong><small>{{ tx('读取上游模型列表；新模型默认停用，可在模型管理中启用', 'Read the upstream model list. New models stay disabled until enabled in Model Management.') }}</small></span></label>
            <label class="connection-model-choice" :class="{ selected: modelMode === 'manual' }"><input v-model="modelMode" type="radio" value="manual"><span><strong>{{ tx('手动添加模型', 'Add models manually') }}</strong><small>{{ tx('适用于需要自定义公开名称的情况', 'Use custom public model names') }}</small></span></label>
          </div>
          <p v-else class="connection-model-hint">{{ selectedApiChannel?.supports_model_discovery ? tx('当前授权无法同步模型，请手动添加。', 'Model sync is unavailable with the current license. Add models manually.') : tx('该通道暂不提供模型列表，请添加需要使用的模型。', 'This channel has no model list. Add the models you need.') }}</p>
          <div v-if="modelMode === 'manual'" class="connection-model-editor">
            <div class="connection-model-head" aria-hidden="true"><span>{{ tx('公开模型名称', 'Public model name') }}</span><span>{{ tx('上游模型 ID', 'Upstream model ID') }}</span><span>{{ tx('额度类型', 'Quota type') }}</span><span></span></div>
            <div v-for="(row, index) in modelRows" :key="index" class="connection-model-row"><label><span>{{ tx('公开模型名称', 'Public model name') }}</span><input v-model="row.publicName" maxlength="160" placeholder="glm-coding" required></label><label><span>{{ tx('上游模型 ID', 'Upstream model ID') }}</span><input v-model="row.upstreamName" maxlength="160" placeholder="glm-4.7" required></label><div class="connection-model-quota"><span>{{ tx('额度类型', 'Quota type') }}</span><ASelect :model-value="row.quotaUnit" :options="quotaOptions(selectedApiChannel?.provider ?? '')" :popup-min-width="135" :aria-label="tx('额度类型', 'Quota type')" @update:model-value="row.quotaUnit = String($event) === 'image' ? 'image' : 'token'" /></div><div class="connection-model-remove"><AIconButton icon="trash" size="small" :label="tx('移除这行模型', 'Remove this model')" :disabled="modelRows.length === 1" @click="modelRows.splice(index, 1)" /></div></div>
            <div class="connection-model-add"><AIconButton icon="plus" size="small" :label="tx('添加模型', 'Add model')" :disabled="modelRows.length >= 64" @click="addModelRow" /></div>
          </div>
        </section>
        <div class="form-actions"><AButton variant="secondary" :disabled="loading" @click="apiKeyOpen = false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="loading" :disabled="!canWrite">{{ modelMode === 'automatic' ? tx('创建并同步', 'Create and sync') : tx('创建连接', 'Create connection') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="!!manualTarget" :title="tx('手动添加模型', 'Add models manually')" :description="manualTarget?.display_name ?? ''" :close-label="tx('关闭', 'Close')" :close-disabled="!!operatingID" wide @close="manualTarget = null">
      <form class="form" @submit.prevent="addManualModels">
        <p class="connection-model-hint">{{ tx('填写公开模型名称和上游模型 ID。新增模型默认停用，添加后在模型管理中启用。', 'Enter a public model name and upstream model ID. New models are disabled until enabled in Model Management.') }}</p>
        <div class="connection-model-editor">
          <div class="connection-model-head" aria-hidden="true"><span>{{ tx('公开模型名称', 'Public model name') }}</span><span>{{ tx('上游模型 ID', 'Upstream model ID') }}</span><span>{{ tx('额度类型', 'Quota type') }}</span><span></span></div>
          <div v-for="(row, index) in modelRows" :key="index" class="connection-model-row"><label><span>{{ tx('公开模型名称', 'Public model name') }}</span><input v-model="row.publicName" maxlength="160" placeholder="glm-coding" required></label><label><span>{{ tx('上游模型 ID', 'Upstream model ID') }}</span><input v-model="row.upstreamName" maxlength="160" placeholder="glm-4.7" required></label><div class="connection-model-quota"><span>{{ tx('额度类型', 'Quota type') }}</span><ASelect :model-value="row.quotaUnit" :options="quotaOptions(manualTarget?.provider ?? '')" :popup-min-width="135" :aria-label="tx('额度类型', 'Quota type')" @update:model-value="row.quotaUnit = String($event) === 'image' ? 'image' : 'token'" /></div><div class="connection-model-remove"><AIconButton icon="trash" size="small" :label="tx('移除这行模型', 'Remove this model')" :disabled="modelRows.length === 1" @click="modelRows.splice(index, 1)" /></div></div>
          <div class="connection-model-add"><AIconButton icon="plus" size="small" :label="tx('添加模型', 'Add model')" :disabled="modelRows.length >= 64" @click="addModelRow" /></div>
        </div>
        <div class="form-actions"><AButton variant="secondary" :disabled="!!operatingID" @click="manualTarget = null">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="!!operatingID" :disabled="!canWrite">{{ tx('保存模型', 'Save models') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="!!editingConnection" :title="tx('编辑连接', 'Edit connection')" :close-label="tx('关闭', 'Close')" :close-disabled="!!operatingID" @close="closeEditConnection">
      <form class="form" @submit.prevent="saveApiConnection">
        <label class="field"><span>{{ tx('连接名称', 'Connection name') }}</span><input v-model="editName" maxlength="160" required></label>
        <label class="field"><span>{{ tx('新 API Key（留空则不更换）', 'New API Key (leave blank to keep current)') }}</span><input v-model="editKey" type="password" autocomplete="new-password" minlength="8" maxlength="1024"></label>
        <div class="form-actions"><AButton variant="secondary" :disabled="!!operatingID" @click="closeEditConnection">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="!!operatingID">{{ tx('保存', 'Save') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="oauthOpen" :title="tx('新增 ChatGPT 账号', 'Add ChatGPT account')" :description="tx('接入会话由 OpenAI 驱动管理；Control 只在提交回调时向健康 Runner 签发一次性兑换任务。', 'The OpenAI driver owns this enrollment session. Control issues a one-time exchange task to a healthy Runner only after the callback is submitted.')" :close-label="tx('关闭', 'Close')" :close-disabled="loading" @close="cancelEnrollment">
      <form class="form" @submit.prevent="completeOAuth">
        <div class="inline-actions oauth-help"><AInfoTip :text="tx('登录后会跳转到 localhost；若页面无法打开，复制地址栏中的完整 URL 并粘贴到下方。', 'After sign-in, the browser redirects to localhost. If it cannot open, copy the full address-bar URL and paste it below.')" /><span>{{ tx('回调地址说明', 'Callback instructions') }}</span></div>
        <a :href="authorizationURL" target="_blank" rel="noopener noreferrer" class="link-button">{{ tx('重新打开 ChatGPT 登录页', 'Reopen ChatGPT sign-in') }}</a>
        <label class="field"><span>{{ tx('完整回调地址', 'Full callback URL') }}</span><input v-model="callbackURL" class="code" placeholder="http://localhost:1455/auth/callback?code=...&state=..." required></label>
        <div class="form-actions"><AButton variant="secondary" :disabled="loading" @click="cancelEnrollment">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="loading" :disabled="!canWrite || !canAuthorize">{{ tx('完成接入', 'Complete connection') }}</AButton></div>
      </form>
    </AModal>
    <AConfirmModal :open="!!pendingRemoval" :title="tx('删除订阅/账号', 'Delete subscription account')" :text="tx(`删除“${pendingRemoval?.email || ''}”会停止任务分配，并级联删除 Control 中属于该账号的全部加密凭据和模型映射。Runner 从不落盘保存账号凭据。`, `Deleting “${pendingRemoval?.email || ''}” stops routing and cascades deletion of all encrypted Control credentials and model mappings for the account. Runners never persist account credentials.`)" :confirm-label="tx('确认删除', 'Delete account')" confirm-icon="trash" :cancel-label="tx('取消', 'Cancel')" danger :busy="!!operatingID" @close="pendingRemoval = null" @confirm="pendingRemoval && remove(pendingRemoval)" />
  </div>
</template>

<style scoped>
.api-connection-action{display:inline-flex;flex:0 0 auto}.api-connection-action.is-disabled{cursor:help}.api-connection-action.is-disabled:deep(.a-button){pointer-events:none}.api-connection-action:focus-visible{outline:2px solid var(--accent);outline-offset:2px;border-radius:10px}
.connection-filter{flex-wrap:wrap}.connection-filter .search-field{min-width:260px;flex:1 1 290px}.connection-filter .field:not(.search-field){min-width:140px;max-width:190px;flex:1 1 140px}.connection-filter>.a-button{margin-left:auto}.connection-filter+.section{margin-top:0}
.connection-table-wrap{max-width:100%}
.connection-table th,.connection-table td{padding-block:11px}
.connection-table td:first-child strong{display:inline-block;max-width:220px;overflow:hidden;text-overflow:ellipsis;vertical-align:middle;white-space:nowrap}
.connection-table .table-action-cell{position:sticky;right:0;z-index:1;width:1%;min-width:0;background:var(--surface);white-space:nowrap}
.connection-table thead .table-action-cell{z-index:2;background:var(--surface-2)}
.connection-table tbody tr:hover .table-action-cell{background:color-mix(in srgb,var(--accent-soft) 42%,var(--surface))}
.connection-table .row-actions{justify-content:flex-end;flex-wrap:nowrap;gap:5px}
.connection-list-empty{display:grid;min-height:150px;place-items:center;padding:20px;color:var(--muted);font-size:var(--font-size-body);text-align:center}
.connection-channel-select:deep(.a-select-trigger){padding-right:18px}
.connection-secret-field{position:relative;min-width:0}.connection-secret-field input{width:100%;padding-right:46px}.connection-secret-toggle{position:absolute;top:50%;right:6px;display:block;width:32px;height:32px;transform:translateY(-50%)}
.connection-model-setup{display:grid;gap:11px;padding-top:17px;border-top:1px solid var(--line)}.connection-model-setup h3{margin:0;font-size:var(--font-size-body)}.connection-model-hint{margin:0;color:var(--muted);font-size:var(--font-size-body)}
.connection-model-choices{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px}.connection-model-choice{display:flex;align-items:flex-start;gap:10px;padding:11px 12px;border:1px solid var(--line-strong);border-radius:10px;cursor:pointer}.connection-model-choice.selected{border-color:var(--accent);background:var(--accent-soft)}.connection-model-choice input{width:18px;height:18px;min-height:18px;padding:0;flex:0 0 18px;margin:2px 0 0;accent-color:var(--accent)}.connection-model-choice span{display:grid;gap:3px}.connection-model-choice strong{font-size:var(--font-size-body)}.connection-model-choice small{color:var(--muted);font-size:var(--font-size-caption)}
.connection-model-editor{display:grid;gap:9px}.connection-model-head,.connection-model-row{display:grid;grid-template-columns:minmax(0,1fr) minmax(0,1fr) 135px 36px;align-items:end;gap:9px}.connection-model-head{color:var(--muted);font-size:var(--font-size-caption);font-weight:700}.connection-model-row label,.connection-model-quota{display:grid;gap:5px;min-width:0}.connection-model-row label>span,.connection-model-quota>span{display:none}.connection-model-row input{min-width:0}.connection-model-quota:deep(.a-select-trigger){padding-right:13px}.connection-model-remove{display:flex;min-height:var(--control-height);align-items:center;justify-content:center}.connection-model-add{display:flex;justify-content:flex-end;padding-right:2px}
@media(max-width:640px){.connection-table .table-action-cell{position:static;box-shadow:none}.connection-model-choices{grid-template-columns:1fr}.connection-model-head{display:none}.connection-model-row{grid-template-columns:minmax(0,1fr) 36px}.connection-model-row label,.connection-model-quota{grid-column:1}.connection-model-row label>span,.connection-model-quota>span{display:block;color:var(--muted);font-size:var(--font-size-caption)}.connection-model-remove{grid-column:2;grid-row:1;align-self:end}}
</style>
