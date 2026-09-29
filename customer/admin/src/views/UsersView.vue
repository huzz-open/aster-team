<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('member')
import { computed, onMounted, reactive, ref, useId, watch } from 'vue'
import { AButton, ACheckbox, AConfirmModal, ACopyCode, AEmpty, AIconButton, AInfoTip, ALoadingState, AModal, APagination, APasswordInput, ASegmentedControl, ASelect, useToast } from '@aster/ui'
import { copyText, createClientRequestId, formatDate, formatMoney, request, type Model, type MoneyEntry, type MoneySnapshot, type User } from '@aster/sdk'
import { locale } from '../i18n'
import RasterIconButton from '../components/RasterIconButton.vue'

const items = ref<User[]>([])
const grantAmountId = useId()
const page = ref(1)
const pageSize = ref(50)
const keyword = ref('')
const statusFilter = ref('')
const balanceFilter = ref('')
const filteredItems = computed(() => {
  const needle = keyword.value.trim().toLocaleLowerCase()
  return items.value.filter((item) => {
    const matchesKeyword = !needle || `${item.display_name} ${item.email}`.toLocaleLowerCase().includes(needle)
    const state = item.status === 'disabled' ? 'disabled' : item.password_change_required ? 'password_change_required' : 'active'
    const matchesStatus = !statusFilter.value || state === statusFilter.value
    const matchesBalance = !balanceFilter.value
      || (balanceFilter.value === 'positive' ? Number(item.money_balance) > 0 : Number(item.money_balance) <= 0)
    return matchesKeyword && matchesStatus && matchesBalance
  })
})
const pagedItems = computed(() => filteredItems.value.slice((page.value - 1) * pageSize.value, page.value * pageSize.value))
const toast = useToast()
const memberOpen = ref(false)
const batchOpen = ref(false)
const operatingID = ref('')
const loading = ref(false)
const createSaving = ref(false)
const batchSaving = ref(false)
const pendingAction = ref<{ user: User; enabled?: boolean; remove?: boolean } | null>(null)
const form = reactive({ email: '', display_name: '', password: '' })
type CreatedCredential = { id: string; email: string; display_name: string; password: string }
const batch = reactive({ emails: '', password_mode: 'random', fixed_password: '' })
const createdCredentials = ref<CreatedCredential[]>([])
const quotaOpen = ref(false)
const quotaHistoryOpen = ref(false)
const quotaUser = ref<User | null>(null)
const quotaSaving = ref(false)
const quotaHistoryLoading = ref(false)
const quotaHistory = ref<MoneyEntry[]>([])
const quotaForm = reactive<{ amount: string; reason: string }>({ amount: '', reason: '' })
const quotaRequestID = ref('')
const quotaHasAmount = computed(() => /^\d+(?:\.\d{1,9})?$/.test(quotaForm.amount.trim()) && Number(quotaForm.amount) > 0)
const parallelUser = ref<User | null>(null)
const parallelLimit = ref<number | ''>('')
const parallelLoading = ref(false)
const parallelSaving = ref(false)
type ModelAccess = { mode: 'selected' | 'all_enabled'; revision: number; model_ids: string[] }
const accessUser = ref<User | null>(null)
const accessPolicy = ref<ModelAccess | null>(null)
const accessModels = ref<Model[]>([])
const accessSelected = ref<string[]>([])
const accessMode = ref<'selected' | 'all_enabled'>('selected')
const accessSearch = ref('')
const accessSaving = ref(false)
const accessLoading = ref(false)
const accessVisibleModels = computed(() => {
  const needle = accessSearch.value.trim().toLocaleLowerCase()
  return accessModels.value.filter(model => !needle
    || `${model.public_name} ${model.display_name} ${model.provider}`.toLocaleLowerCase().includes(needle))
})
const passwordResetUser = ref<User | null>(null)
const passwordResetSaving = ref(false)
const temporaryPassword = ref('')
const passwordModeOptions = [
  { value: 'random', label: '随机密码' },
  { value: 'fixed', label: '固定密码' },
]
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const statusOptions = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'active', label: tx('正常', 'Active') },
  { value: 'password_change_required', label: tx('待首次改密', 'Password change required') },
  { value: 'disabled', label: tx('已禁用', 'Disabled') },
])
const balanceOptions = computed(() => [
  { value: '', label: tx('全部余额', 'All balances') },
  { value: 'positive', label: tx('有余额', 'Has balance') },
  { value: 'empty', label: tx('余额为零', 'Zero balance') },
])
const accessModeOptions = computed(() => [
  { value: 'selected', label: tx('指定模型', 'Selected models') },
  { value: 'all_enabled', label: tx('所有当前及未来启用的模型', 'All current and future enabled models') },
])
const date = (value?: string) => formatDate(value, locale.value)

async function load() {
  loading.value = true
  try {
    const result = await request<{ items: User[] }>('/api/admin/users')
    items.value = result.items
    page.value = Math.min(page.value, Math.max(1, Math.ceil(items.value.length / pageSize.value)))
  }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('成员列表加载失败', 'Could not load members')) }
  finally { loading.value = false }
}

async function create() {
  createSaving.value = true
  try {
    await request('/api/admin/users', { method: 'POST', body: JSON.stringify(form) })
    memberOpen.value = false
    Object.assign(form, { email: '', display_name: '', password: '' })
    toast.success(tx('成员已创建。', 'Member created.'))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('创建失败', 'Could not create member'))
  } finally {
    createSaving.value = false
  }
}

function openBatch() {
  Object.assign(batch, { emails: '', password_mode: 'random', fixed_password: '' })
  createdCredentials.value = []
  batchOpen.value = true
}

async function createBatch() {
  batchSaving.value = true
  const emails = batch.emails.split(/\r?\n/).map(value => value.trim()).filter(Boolean)
  try {
    const result = await request<{ items: CreatedCredential[] }>('/api/admin/users/batch', {
      method: 'POST', body: JSON.stringify({ emails, password_mode: batch.password_mode, fixed_password: batch.fixed_password }),
    })
    createdCredentials.value = result.items
    toast.success(tx(`已创建 ${result.items.length} 个成员。请立即复制账密，关闭窗口后不会再次显示密码。`, `${result.items.length} members created. Copy the credentials now; passwords will not be shown again after closing.`), 6_000)
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('批量创建失败', 'Batch creation failed')) }
  finally { batchSaving.value = false }
}

function credentialText(credentials = createdCredentials.value): string {
  return credentials.map(item => `${item.email}\t${item.password}`).join('\n')
}

async function copyCredentials() {
  try { await copyText(credentialText()); toast.success(tx('全部成员账密已复制。', 'All member credentials copied.')) }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('复制失败', 'Copy failed')) }
}

async function setEnabled(user: User, enabled: boolean) {
  const action = enabled ? tx('启用', 'Enable') : tx('禁用', 'Disable')
  operatingID.value = user.id
  try {
    await request(`/api/admin/users/${user.id}`, { method: 'PATCH', body: JSON.stringify({ enabled }) })
    toast.success(enabled ? tx(`${user.display_name} 已启用。`, `${user.display_name} enabled.`) : tx(`${user.display_name} 已禁用。`, `${user.display_name} disabled.`))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx(`${action}失败`, `${action} failed`))
  } finally {
    operatingID.value = ''
  }
}

async function remove(user: User) {
  operatingID.value = user.id
  try {
    await request(`/api/admin/users/${user.id}`, { method: 'DELETE' })
    toast.success(tx(`${user.display_name} 已删除。`, `${user.display_name} deleted.`))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('删除失败', 'Delete failed'))
  } finally {
    operatingID.value = ''
  }
}

function askEnabled(user: User, enabled: boolean) { pendingAction.value = { user, enabled } }
function askRemove(user: User) { pendingAction.value = { user, remove: true } }
async function confirmPendingAction() {
  const action = pendingAction.value
  if (!action) return
  if (action.remove) await remove(action.user)
  else await setEnabled(action.user, Boolean(action.enabled))
  pendingAction.value = null
}

function statusLabel(user: User) {
  if (user.status === 'disabled') return tx('已禁用', 'Disabled')
  if (user.status !== 'active') return user.status
  return user.password_change_required ? tx('待首次改密', 'Password change required') : tx('正常', 'Active')
}

function openPasswordReset(user: User) {
  passwordResetUser.value = user
  temporaryPassword.value = ''
}

function closePasswordReset() {
  passwordResetUser.value = null
  temporaryPassword.value = ''
}

async function resetMemberPassword() {
  if (!passwordResetUser.value) return
  passwordResetSaving.value = true
  try {
    const result = await request<{ temporary_password: string }>(`/api/admin/users/${passwordResetUser.value.id}/password-reset`, { method: 'POST' })
    temporaryPassword.value = result.temporary_password
    toast.success(tx('成员密码已重置。', 'Member password reset.'))
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('密码重置失败', 'Could not reset password')) }
  finally { passwordResetSaving.value = false }
}

async function copyTemporaryPassword() {
  try { await copyText(temporaryPassword.value) }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('复制失败', 'Copy failed')) }
}

function resetFilters() {
  keyword.value = ''
  statusFilter.value = ''
  balanceFilter.value = ''
}

function openQuota(user: User) {
  quotaUser.value = user
  Object.assign(quotaForm, { amount: '', reason: '' })
  quotaRequestID.value = createClientRequestId()
  quotaOpen.value = true
}

async function grantQuota() {
  if (!quotaUser.value) return
  if (!quotaHasAmount.value) {
    toast.error(tx('请填写有效金额。', 'Enter a valid amount.'))
    return
  }
  quotaSaving.value = true
  try {
    await request(`/api/admin/users/${quotaUser.value.id}/money/grants`, {
      method: 'POST', body: JSON.stringify({ amount: quotaForm.amount.trim(), reason: quotaForm.reason.trim(), request_id: quotaRequestID.value }),
    })
    toast.success(tx(`已更新 ${quotaUser.value.display_name} 的余额。`, `Balance updated for ${quotaUser.value.display_name}.`))
    quotaOpen.value = false
    await load()
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('金额发放失败', 'Could not grant amount')) }
  finally { quotaSaving.value = false }
}

async function openQuotaHistory(user: User) {
  quotaUser.value = user
  quotaHistory.value = []
  quotaHistoryOpen.value = true
  quotaHistoryLoading.value = true
  try { quotaHistory.value = (await request<MoneySnapshot>(`/api/admin/users/${user.id}/money`)).entries.filter(item => item.kind === 'grant') }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('金额记录加载失败', 'Could not load grant history')) }
  finally { quotaHistoryLoading.value = false }
}

async function openParallelPolicy(user: User) {
  parallelUser.value = user
  parallelLimit.value = ''
  parallelLoading.value = true
  try {
    const result = await request<{ max_parallel: number | null }>(`/api/admin/users/${user.id}/billing-policy`)
    parallelLimit.value = result.max_parallel ?? ''
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('并行上限加载失败', 'Could not load concurrency limit')) }
  finally { parallelLoading.value = false }
}
async function saveParallelPolicy() {
  if (!parallelUser.value) return
  const limit = parallelLimit.value === '' ? null : Number(parallelLimit.value)
  if (limit !== null && (!Number.isInteger(limit) || limit < 1 || limit > 1_000)) {
    toast.error(tx('并行上限需为 1–1000，或留空表示不限制。', 'Enter 1–1000, or leave blank for no limit.'))
    return
  }
  parallelSaving.value = true
  try {
    await request(`/api/admin/users/${parallelUser.value.id}/billing-policy`, { method: 'PUT', body: JSON.stringify({ max_parallel: limit }) })
    toast.success(tx('成员并行上限已保存。', 'Member concurrency limit saved.'))
    parallelUser.value = null
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('并行上限保存失败', 'Could not save concurrency limit')) }
  finally { parallelSaving.value = false }
}

async function openModelAccess(user: User) {
  accessUser.value = user
  accessPolicy.value = null
  accessLoading.value = true
  accessSearch.value = ''
  try {
    const [policy, models] = await Promise.all([
      request<ModelAccess>(`/api/admin/users/${user.id}/model-access`),
      request<{ items: Model[] }>('/api/admin/models'),
    ])
    accessPolicy.value = policy
    accessMode.value = policy.mode
    accessSelected.value = [...policy.model_ids]
    accessModels.value = models.items
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('模型权限加载失败', 'Could not load model access'))
  } finally { accessLoading.value = false }
}

async function saveModelAccess() {
  if (!accessUser.value || !accessPolicy.value) return
  accessSaving.value = true
  try {
    const result = await request<ModelAccess>(`/api/admin/users/${accessUser.value.id}/model-access`, {
      method: 'PUT',
      body: JSON.stringify({ expected_revision: accessPolicy.value.revision, mode: accessMode.value,
        model_ids: accessMode.value === 'selected' ? accessSelected.value : [] }),
    })
    accessPolicy.value = result
    accessUser.value = null
    toast.success(tx('模型权限已更新', 'Model access updated'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('模型权限保存失败', 'Could not save model access'))
  } finally { accessSaving.value = false }
}

onMounted(load)
watch(pageSize, () => { page.value = 1 })
watch([keyword, statusFilter, balanceFilter], () => { page.value = 1 })
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head">
      <div><h1>{{ tx('成员管理', 'Members') }}</h1></div>
      <div class="inline-actions"><AButton icon="plus" variant="secondary" @click="openBatch" :disabled="!canWrite">{{ tx('批量创建', 'Batch create') }}</AButton><AButton icon="plus" @click="memberOpen = true" :disabled="!canWrite">{{ tx('新增成员', 'Add member') }}</AButton></div>
    </header>
    <div class="filter-bar member-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :placeholder="tx('成员名称或邮箱', 'Member name or email')"></label>
      <label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="statusFilter" :options="statusOptions" :aria-label="tx('状态', 'Status')" /></label>
      <label class="field"><span>{{ tx('余额', 'Balance') }}</span><ASelect v-model="balanceFilter" :options="balanceOptions" :aria-label="tx('余额', 'Balance')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>
    <div class="table-wrap paginated-scroll">
      <ALoadingState v-if="loading && !items.length" :label="tx('正在读取成员…', 'Loading members…')" />
      <table v-else-if="filteredItems.length" class="usage-table flat-data-table">
        <thead><tr><th>{{ tx('成员', 'Member') }}</th><th>{{ tx('邮箱', 'Email') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('金额余额', 'Balance') }}</th><th>{{ tx('累计发放', 'Granted') }}</th><th>{{ tx('累计费用', 'Spent') }}</th><th>{{ tx('创建时间', 'Created') }}</th><th>{{ tx('操作', 'Actions') }}</th></tr></thead>
        <tbody>
          <tr v-for="item in pagedItems" :key="item.id">
            <td><strong>{{ item.display_name }}</strong></td>
            <td class="code">{{ item.email }}</td>
            <td><span class="status" :class="{ off: item.status !== 'active', warning: item.status === 'active' && item.password_change_required }">{{ statusLabel(item) }}</span></td>
            <td>{{ formatMoney(item.money_balance, item.money_currency, 2) }}</td><td>{{ formatMoney(item.money_credited, item.money_currency, 2) }}</td><td>{{ formatMoney(item.money_debited, item.money_currency) }}</td><td>{{ date(item.created_at) }}</td>
            <td>
              <div class="row-actions">
                <RasterIconButton :icon="item.status === 'active' ? 'disable-user' : 'enable-user'" :variant="item.status === 'active' ? 'neutral' : 'accent'" :label="item.status === 'active' ? tx('禁用成员', 'Disable member') : tx('启用成员', 'Enable member')" :disabled="(!canWrite && item.status !== 'active') || operatingID === item.id" @click="askEnabled(item, item.status !== 'active')" />
                <AIconButton icon="payment" size="small" variant="accent" :label="tx('发放金额', 'Grant amount')" :disabled="!canWrite || operatingID === item.id" @click="openQuota(item)" />
                <AIconButton icon="audit" size="small" :label="tx('查看金额发放记录', 'View grant history')" :disabled="operatingID === item.id" @click="openQuotaHistory(item)" />
                <AIconButton icon="settings" size="small" :label="tx('设置并行上限', 'Set concurrency limit')" :disabled="!canWrite || operatingID === item.id" @click="openParallelPolicy(item)" />
                <AIconButton icon="model" size="small" :label="tx('模型权限', 'Model access')" :disabled="operatingID === item.id" @click="openModelAccess(item)" />
                <AIconButton icon="key" size="small" :label="tx('重置成员密码', 'Reset member password')" :disabled="operatingID === item.id || item.status !== 'active'" @click="openPasswordReset(item)" />
                <AIconButton icon="trash" size="small" variant="danger" :label="tx('删除成员', 'Delete member')" :disabled="operatingID === item.id" @click="askRemove(item)" />
              </div>
            </td>
          </tr>
        </tbody>
      </table>
      <AEmpty v-else :title="items.length ? tx('没有匹配的成员', 'No matching members') : tx('还没有成员', 'No members yet')" />
    </div>
    <APagination v-if="filteredItems.length > 0" v-model:page="page" v-model:page-size="pageSize" :total="filteredItems.length" :locale="locale" />

    <AModal :open="memberOpen" :title="tx('新增团队成员', 'Add team member')" :description="tx('这是一次性初始密码。成员首次登录后必须修改，并自行创建 API Key。', 'This is a one-time initial password. The member must change it on first sign-in and create their own API key.')" :close-label="tx('关闭', 'Close')" :close-disabled="createSaving" @close="memberOpen = false">
      <form class="form" @submit.prevent="create">
        <label class="field"><span>{{ tx('显示名称', 'Display name') }}</span><input v-model="form.display_name" required></label>
        <label class="field"><span>{{ tx('邮箱', 'Email') }}</span><input v-model="form.email" type="email" required></label>
        <div class="field"><span>{{ tx('初始密码', 'Initial password') }}</span><APasswordInput v-model="form.password" :aria-label="tx('初始密码', 'Initial password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /></div>
        <div class="form-actions"><AButton variant="secondary" :disabled="createSaving" @click="memberOpen = false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="createSaving" :disabled="!canWrite">{{ tx('创建成员', 'Create member') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="batchOpen" :title="tx('批量创建成员', 'Batch create members')" :description="createdCredentials.length ? tx('初始密码仅在这里显示一次，请立即复制或发送。', 'Initial passwords are shown only once. Copy or send them now.') : tx('每行填写一个邮箱；显示名称会根据邮箱前缀生成。', 'Enter one email per line. Display names are generated from email prefixes.')" :close-label="tx('关闭', 'Close')" :close-disabled="batchSaving" @close="batchOpen = false">
      <form v-if="!createdCredentials.length" class="form" @submit.prevent="createBatch">
        <label class="field"><span>{{ tx('成员邮箱（一行一个）', 'Member emails (one per line)') }}</span><textarea v-model="batch.emails" rows="9" placeholder="alice@example.com&#10;bob@example.com" required /></label>
        <ASegmentedControl v-model="batch.password_mode" :options="passwordModeOptions" :label="tx('初始密码方式', 'Initial password mode')" />
        <div v-if="batch.password_mode === 'fixed'" class="field"><span>{{ tx('固定初始密码', 'Fixed initial password') }}</span><APasswordInput v-model="batch.fixed_password" :aria-label="tx('固定初始密码', 'Fixed initial password')" minlength="12" autocomplete="new-password" required :show-label="tx('显示密码', 'Show password')" :hide-label="tx('隐藏密码', 'Hide password')" /><small>{{ tx('所有成员使用同一个至少 12 位的临时密码，首次登录仍必须修改。', 'All members receive the same temporary password of at least 12 characters and must change it on first sign-in.') }}</small></div>
        <div class="form-actions"><AButton variant="secondary" :disabled="batchSaving" @click="batchOpen=false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="batchSaving" :disabled="!canWrite">{{ tx('批量创建成员', 'Create members') }}</AButton></div>
      </form>
      <div v-else class="form">
        <div class="table-wrap credential-table"><table class="flat-data-table"><thead><tr><th>{{ tx('成员', 'Member') }}</th><th>{{ tx('邮箱', 'Email') }}</th><th>{{ tx('临时密码', 'Temporary password') }}</th></tr></thead><tbody><tr v-for="item in createdCredentials" :key="item.id"><td><strong>{{ item.display_name }}</strong></td><td class="code">{{ item.email }}</td><td><code>{{ item.password }}</code></td></tr></tbody></table></div>
        <div class="form-actions"><AButton variant="secondary" @click="copyCredentials">{{ tx('复制全部账密', 'Copy all credentials') }}</AButton></div>
      </div>
    </AModal>
    <AConfirmModal :open="!!pendingAction" :title="pendingAction?.remove ? tx('删除成员', 'Delete member') : pendingAction?.enabled ? tx('启用成员', 'Enable member') : tx('禁用成员', 'Disable member')" :text="pendingAction?.remove ? tx(`成员“${pendingAction?.user.display_name || ''}”将无法登录，已有 Key 会永久撤销；历史审计记录会保留。`, `“${pendingAction?.user.display_name || ''}” will lose access and existing keys will be revoked. Audit history is retained.`) : pendingAction?.enabled ? tx('该成员未撤销的 Key 将恢复使用。', 'Non-revoked keys will work again.') : tx('该成员的当前会话和 Key 将立即停止使用。', 'Current sessions and keys will stop immediately.')" :confirm-label="pendingAction?.remove ? tx('确认删除', 'Delete member') : tx('确认执行', 'Confirm')" :confirm-icon="pendingAction?.remove ? 'trash' : undefined" :cancel-label="tx('取消', 'Cancel')" :danger="pendingAction?.remove || pendingAction?.enabled === false" :busy="!!operatingID" @close="pendingAction = null" @confirm="confirmPendingAction" />
    <AModal :open="!!passwordResetUser" :title="tx('重置成员密码', 'Reset member password')" :description="temporaryPassword ? tx('临时密码只显示这一次，请立即复制并安全发送给成员。', 'This temporary password is shown only once. Copy it now and send it securely to the member.') : tx(`为 ${passwordResetUser?.display_name || ''} 生成新的临时密码，并撤销该成员的全部现有会话。`, `Generate a new temporary password for ${passwordResetUser?.display_name || ''} and revoke all existing sessions for this member.`)" :close-label="tx('关闭', 'Close')" :close-disabled="passwordResetSaving" @close="closePasswordReset">
      <div v-if="temporaryPassword" class="form">
        <div class="notice">{{ tx('成员下次登录时必须立即设置自己的新密码。', 'The member must set their own new password immediately after the next sign-in.') }}</div>
        <ACopyCode :value="temporaryPassword" :label="tx('复制', 'Copy')" :copied-label="tx('已复制', 'Copied')" @copy="copyTemporaryPassword" />
      </div>
      <div v-else class="form">
        <div class="notice">{{ tx('此操作不会显示或恢复旧密码。审计记录会保留执行管理员、目标成员和时间。', 'This does not reveal or recover the old password. The administrator, member, and time are recorded in the audit log.') }}</div>
        <div class="form-actions"><AButton variant="secondary" :disabled="passwordResetSaving" @click="closePasswordReset">{{ tx('取消', 'Cancel') }}</AButton><AButton :loading="passwordResetSaving" @click="resetMemberPassword">{{ tx('生成临时密码', 'Generate temporary password') }}</AButton></div>
      </div>
    </AModal>
    <AModal :open="quotaOpen" :title="tx('发放金额', 'Grant amount')" :close-label="tx('关闭', 'Close')" :close-disabled="quotaSaving" @close="quotaOpen=false">
      <form class="form" @submit.prevent="grantQuota">
        <div class="quota-target"><strong>{{ quotaUser?.display_name }}</strong><span>{{ quotaUser?.email }}</span></div>
        <label class="field" :for="grantAmountId"><span>{{ tx('发放金额', 'Amount to grant') }}</span><input :id="grantAmountId" v-model.trim="quotaForm.amount" type="text" inputmode="decimal" placeholder="0.00" required></label>
        <label class="field"><span class="field-label">{{ tx('发放原因', 'Reason') }}<AInfoTip :text="tx('管理员、金额、原因与时间会写入不可覆盖的账本。', 'The administrator, amount, reason, and time are recorded in the append-only ledger.')" /></span><textarea v-model="quotaForm.reason" rows="3" minlength="2" maxlength="200" required></textarea></label>
        <div class="form-actions"><AButton variant="secondary" :disabled="quotaSaving" @click="quotaOpen=false">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="quotaSaving" :disabled="!canWrite || !quotaHasAmount">{{ tx('确认发放', 'Grant amount') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="quotaHistoryOpen" :title="tx(`${quotaUser?.display_name || ''} · 金额发放记录`, `${quotaUser?.display_name || ''} · Grant history`)" :close-label="tx('关闭', 'Close')" @close="quotaHistoryOpen=false">
      <ALoadingState v-if="quotaHistoryLoading" :label="tx('正在读取金额记录…', 'Loading grants…')" />
      <div v-else-if="quotaHistory.length" class="quota-history"><article v-for="item in quotaHistory" :key="item.id"><div><strong>+{{ formatMoney(item.amount, quotaUser?.money_currency, 2) }}</strong><span>{{ date(item.created_at) }}</span></div><p>{{ item.details.reason || '—' }}</p></article></div>
      <AEmpty v-else icon="audit" :title="tx('暂无金额发放记录', 'No grants yet')" />
    </AModal>
    <AModal :open="!!parallelUser" :title="tx('成员并行上限', 'Member concurrency limit')" :close-label="tx('关闭', 'Close')" :close-disabled="parallelSaving" @close="parallelUser=null">
      <ALoadingState v-if="parallelLoading" :label="tx('正在读取配置…', 'Loading settings…')" />
      <form v-else class="form" @submit.prevent="saveParallelPolicy">
        <p>{{ parallelUser?.display_name }} · {{ tx('限制该成员同时运行的请求数。留空表示不限制；余额不足参考费用时系统仍会自动串行等待。', 'Limit simultaneous requests for this member. Leave blank for no limit. Low balances still trigger automatic serial admission.') }}</p>
        <label class="field"><span>{{ tx('最多并行请求', 'Maximum parallel requests') }}</span><input v-model.number="parallelLimit" type="number" min="1" max="1000" step="1" :placeholder="tx('不限制', 'Unlimited')"></label>
        <div class="form-actions"><AButton variant="secondary" @click="parallelUser=null">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="parallelSaving">{{ tx('保存', 'Save') }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="!!accessUser" :title="tx(`${accessUser?.display_name || ''} · 模型权限`, `${accessUser?.display_name || ''} · Model access`)" :close-label="tx('关闭', 'Close')" :close-disabled="accessSaving" @close="accessUser = null">
      <ALoadingState v-if="accessLoading" :label="tx('正在读取模型权限…', 'Loading model access…')" />
      <form v-else-if="accessPolicy" class="form" @submit.prevent="saveModelAccess">
        <label class="field"><span>{{ tx('授权方式', 'Access mode') }}</span><ASelect v-model="accessMode" :options="accessModeOptions" :aria-label="tx('授权方式', 'Access mode')" /></label>
        <template v-if="accessMode === 'selected'">
          <label class="field"><span>{{ tx('搜索模型', 'Search models') }}</span><input v-model="accessSearch" :placeholder="tx('名称或厂商', 'Name or provider')"></label>
          <div class="access-summary">{{ tx(`已授权 ${accessSelected.length} 个模型`, `${accessSelected.length} models selected`) }}</div>
          <div class="access-model-list">
            <ACheckbox v-for="model in accessVisibleModels" :key="model.id" v-model="accessSelected" class="access-model" :value="model.id"><strong :title="model.public_name">{{ model.public_name }}</strong><small>{{ model.provider }} · {{ model.enabled ? tx('已启用', 'Enabled') : tx('已停用', 'Disabled') }}</small></ACheckbox>
            <AEmpty v-if="!accessVisibleModels.length" :title="tx('没有匹配的模型', 'No matching models')" />
          </div>
        </template>
        <div class="form-actions"><AButton variant="secondary" :disabled="accessSaving" @click="accessUser = null">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="accessSaving" :disabled="!canWrite">{{ tx('保存权限', 'Save access') }}</AButton></div>
      </form>
    </AModal>
  </div>
</template>

<style scoped>
.member-filter .search-field{min-width:260px;flex:1 1 420px}.member-filter .field:not(.search-field){max-width:210px}.member-filter>.a-button{margin-left:auto}
.quota-target{display:grid;gap:2px;padding:11px;border:1px solid var(--line);border-radius:10px;background:var(--surface-2)}.quota-target span{color:var(--muted);font-size:var(--font-size-caption)}.field-label{display:flex;align-items:center;gap:3px}.quota-history{display:grid;gap:8px}.quota-history article{display:grid;grid-template-columns:1fr auto;gap:5px 12px;padding:11px;border:1px solid var(--line);border-radius:10px;background:var(--surface-2)}.quota-history article>div{display:flex;align-items:center;justify-content:space-between;grid-column:1/-1}.quota-history article strong{color:var(--positive)}.quota-history article span{color:var(--muted);font-size:var(--font-size-caption)}.quota-history article p{color:var(--text-soft)}
.quota-model-table table{min-width:0}.quota-model-table th:last-child,.quota-model-table td:last-child{width:150px}.quota-model-table input{width:100%;min-width:100px}
.access-summary{color:var(--muted);font-size:var(--font-size-body)}.access-model-list{display:grid;gap:6px;max-height:320px;overflow:auto}.access-model{align-items:center;padding:8px;border-color:var(--line);border-radius:8px}.access-model :deep(.a-checkbox-copy){width:100%;display:flex;align-items:center;gap:8px}.access-model strong{min-width:0;overflow:hidden;text-overflow:ellipsis;white-space:nowrap}.access-model small{flex:0 0 auto;margin-left:auto;color:var(--muted);white-space:nowrap}
</style>
