<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { AButton, ADateRange, AEmpty, ALoadingState, APagination, ASelect, useToast } from '@aster/ui'
import { formatDate, request, type AuditEvent } from '@aster/sdk'
import { locale } from '../i18n'

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const toast = useToast()
const items = ref<AuditEvent[]>([])
const loading = ref(true)
const action = ref('')
const outcome = ref('')
const keyword = ref('')
const fromDate = ref('')
const toDate = ref('')
const today = localDate(new Date())
const page = ref(1)
const pageSize = ref(50)
const total = ref(0)
let filterTimer: number | undefined

const actionChoices = computed(() => [
  { value: '', label: tx('全部动作', 'All actions') },
  ...[
    'owner.initialize', 'identity.password.update', 'settings.update', 'member.create',
    'member.status.update', 'member.delete', 'member.quota.grant', 'quota_request.create',
    'quota_request.review', 'voucher.create', 'voucher.delete', 'voucher.redeem',
    'api_key.create', 'api_key.revoke', 'runner_enrollment.create', 'runner.register',
    'runner.status.update', 'runner.delete', 'upstream_account.status.update',
    'upstream_account.delete', 'upstream_credential.create', 'upstream_credential.refresh',
    'upstream_model.sync', 'model.update', 'release.upgrade',
  ].map(value => ({ value, label: actionLabel(value) })),
])
const outcomeChoices = computed(() => [
  { value: '', label: tx('全部结果', 'All outcomes') },
  { value: 'succeeded', label: tx('成功', 'Succeeded') },
  { value: 'failed', label: tx('失败', 'Failed') },
])
function localDate(value: Date) {
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
}
function actionLabel(value: string) {
  const labels: Record<string, [string, string]> = {
    'owner.initialize': ['初始化所有者', 'Initialize owner'],
    'identity.password.update': ['修改密码', 'Change password'],
    'settings.update': ['修改运行配置', 'Update runtime settings'],
    'member.create': ['创建成员', 'Create member'],
    'member.status.update': ['修改成员状态', 'Update member status'],
    'member.delete': ['删除成员', 'Delete member'],
    'member.quota.grant': ['调整成员额度', 'Adjust member quota'],
    'quota_request.create': ['提交额度申请', 'Create quota request'],
    'quota_request.review': ['审批额度申请', 'Review quota request'],
    'voucher.create': ['创建兑换券', 'Create voucher'],
    'voucher.delete': ['删除兑换券', 'Delete voucher'],
    'voucher.redeem': ['兑换额度券', 'Redeem voucher'],
    'api_key.create': ['创建 API Key', 'Create API key'],
    'api_key.revoke': ['撤销 API Key', 'Revoke API key'],
    'runner_enrollment.create': ['创建 Runner 注册令牌', 'Create Runner enrollment'],
    'runner.register': ['注册 Runner', 'Register Runner'],
    'runner.status.update': ['修改 Runner 状态', 'Update Runner status'],
    'runner.delete': ['删除 Runner', 'Delete Runner'],
    'upstream_account.status.update': ['修改订阅/账号状态', 'Update subscription account'],
    'upstream_account.delete': ['删除订阅/账号', 'Delete subscription account'],
    'upstream_credential.create': ['新增上游凭据实例', 'Create credential instance'],
    'upstream_credential.refresh': ['刷新上游凭据实例', 'Refresh credential instance'],
    'upstream_model.sync': ['同步上游模型', 'Sync upstream models'],
    'model.update': ['修改模型状态', 'Update model status'],
    'release.upgrade': ['升级客户运行时', 'Upgrade customer runtime'],
  }
  const label = labels[value]
  return label ? tx(label[0], label[1]) : value
}
function actorLabel(item: AuditEvent) {
  if (item.actor_role === 'system') return tx('本机系统', 'Local system')
  return item.actor_display_name || item.actor_email || item.actor_identity_id?.slice(-12) || item.actor_role
}
function roleLabel(role: AuditEvent['actor_role']) {
  const labels: Record<AuditEvent['actor_role'], [string, string]> = {
    owner: ['所有者', 'Owner'], admin: ['管理员', 'Admin'], member: ['成员', 'Member'],
    system: ['系统', 'System'], runner: ['Runner', 'Runner'],
  }
  return tx(labels[role][0], labels[role][1])
}
function targetLabel(item: AuditEvent) {
  return item.target_id ? `${item.target_type} · ${item.target_id}` : item.target_type
}
function query() {
  const params = new URLSearchParams({ limit: String(pageSize.value), offset: String((page.value - 1) * pageSize.value) })
  if (fromDate.value && toDate.value) { params.set('from', fromDate.value); params.set('to', toDate.value) }
  if (action.value) params.set('action', action.value)
  if (outcome.value) params.set('outcome', outcome.value)
  if (keyword.value.trim()) params.set('keyword', keyword.value.trim())
  return params
}
async function load() {
  loading.value = true
  try {
    const result = await request<{ items: AuditEvent[]; total: number }>(`/api/admin/audit-events?${query()}`)
    items.value = result.items
    total.value = result.total
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('安全审计加载失败', 'Could not load security audit'))
  } finally { loading.value = false }
}
function reset() {
  const unchanged = !action.value && !outcome.value && !keyword.value && !fromDate.value && !toDate.value
  action.value = ''; outcome.value = ''; keyword.value = ''; fromDate.value = ''; toDate.value = ''
  if (unchanged) void load()
}

onMounted(load)
watch([action, outcome, keyword, fromDate, toDate], () => {
  if (Boolean(fromDate.value) !== Boolean(toDate.value)) return
  window.clearTimeout(filterTimer)
  filterTimer = window.setTimeout(() => {
    if (page.value === 1) void load()
    else page.value = 1
  }, 200)
})
watch([page, pageSize], () => void load())
onBeforeUnmount(() => window.clearTimeout(filterTimer))
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head">
      <div><h1>{{ tx('安全审计', 'Security audit') }}</h1></div>
    </header>

    <div class="filter-bar">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :placeholder="tx('执行人、动作、目标或序号', 'Actor, action, target, or sequence')"></label>
      <div class="field date-field"><span>{{ tx('时间范围', 'Time range') }}</span><ADateRange v-model:from="fromDate" v-model:to="toDate" :max="today" :max-range-days="366" :start-label="tx('开始日期', 'Start date')" :end-label="tx('结束日期', 'End date')" :locale="locale" /></div>
      <label class="field"><span>{{ tx('动作', 'Action') }}</span><ASelect v-model="action" :options="actionChoices" :aria-label="tx('动作', 'Action')" searchable /></label>
      <label class="field"><span>{{ tx('结果', 'Outcome') }}</span><ASelect v-model="outcome" :options="outcomeChoices" :aria-label="tx('结果', 'Outcome')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="reset">{{ tx('重置', 'Reset') }}</AButton>
    </div>

    <div class="table-wrap paginated-scroll">
      <ALoadingState v-if="loading" :label="tx('正在校验并读取审计链…', 'Verifying and loading the audit chain…')" />
      <table v-else-if="items.length" class="flat-data-table audit-table">
        <thead><tr><th>{{ tx('时间', 'Time') }}</th><th>{{ tx('执行人', 'Actor') }}</th><th>{{ tx('邮箱', 'Email') }}</th><th>{{ tx('角色', 'Role') }}</th><th>{{ tx('动作', 'Action') }}</th><th>{{ tx('动作码', 'Action code') }}</th><th>{{ tx('目标', 'Target') }}</th><th>{{ tx('结果', 'Outcome') }}</th></tr></thead>
        <tbody><tr v-for="item in items" :key="item.id"><td>{{ formatDate(item.created_at, locale) }}</td><td><strong>{{ actorLabel(item) }}</strong></td><td>{{ item.actor_email || '—' }}</td><td>{{ roleLabel(item.actor_role) }}</td><td><strong>{{ actionLabel(item.action) }}</strong></td><td><code>{{ item.action }}</code></td><td><code :title="item.target_id">{{ targetLabel(item) }}</code></td><td><span class="status" :class="{ off:item.outcome==='failed' }">{{ item.outcome === 'succeeded' ? tx('成功', 'Succeeded') : tx('失败', 'Failed') }}</span></td></tr></tbody>
      </table>
      <AEmpty v-else icon="audit" :title="tx('没有匹配的审计记录', 'No matching audit events')" />
    </div>
    <APagination v-if="total > 0" v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="loading" :locale="locale" />
  </div>
</template>

<style scoped>
.filter-bar{align-items:center}.filter-bar .field{min-width:150px;flex:1}.filter-bar .search-field{min-width:220px;flex:1}.filter-bar .date-field{width:220px;min-width:220px;flex:0 0 220px}.filter-bar>.a-button{margin-left:auto}.date-field .a-date-range{width:220px;min-width:220px;max-width:220px}.audit-table{min-width:1300px}.audit-table td{vertical-align:top}.audit-table td strong,.audit-table td code{display:block}.audit-table td code{max-width:360px;overflow:hidden;text-overflow:ellipsis;color:var(--muted)}@media(max-width:760px){.filter-bar .field,.filter-bar .search-field,.filter-bar .date-field{min-width:100%}}
.filter-bar .field{min-width:100px;flex:1 1 120px}.filter-bar .search-field{min-width:140px;flex:0 1 220px}.filter-bar .date-field{width:220px;min-width:220px;flex:0 0 220px}.filter-bar>.a-button{flex:0 0 auto}@media(max-width:1120px){.filter-bar{flex-wrap:wrap}.filter-bar .search-field{flex-grow:1}}@media(max-width:760px){.filter-bar .field,.filter-bar .search-field,.filter-bar .date-field{width:100%;min-width:100%;flex-basis:100%}.date-field .a-date-range{width:100%;max-width:100%}}
</style>
