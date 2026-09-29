<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { AButton, ACopyCode, AEmpty, ALoadingState, AModal, APagination, ASelect, useToast } from '@aster/ui'
import { approveFreeDistribution, downloadFreeDistribution, getCurrentOperationsOperatorID, getFreeDistribution, issueFreeDistribution, listCommercialPlans, listFreeDistributions, listV2IssuerProfiles, OperationsAPIError, type ApproveFreeDistributionInput, type CommercialPlanRecord, type FreeDistributionRecord, type V2IssuerProfile } from '../api/client'
import { submissionJournal } from '../commercial/submission-journal'
import CommercialPlanSummary from '../components/CommercialPlanSummary.vue'

const toast = useToast()
const records = ref<FreeDistributionRecord[]>([])
const page = ref(1); const pageSize = ref(50); const statusFilter = ref('')
const loading = ref(false); const listLoadError = ref('')
const approvalOpen = ref(false); const approving = ref(false); const approvalError = ref('')
const plans = ref<CommercialPlanRecord[]>([]); const plansLoading = ref(false); const plansError = ref('')
const form = reactive({ planID: '', starts: '', reason: '' })
const approvalPassword = ref(''); const issuePassword = ref('')
const journal = submissionJournal<ApproveFreeDistributionInput>('distribution', getCurrentOperationsOperatorID())
const pending = journal.pending; const journalError = journal.error
const plan = computed(() => plans.value.find(value => value.snapshot.plan_id === form.planID))
const planOptions = computed(() => plans.value.filter(value => value.snapshot.definition.offer.kind === 'free').map(value => ({ value: value.snapshot.plan_id, label: `${value.snapshot.definition.name} · v${value.snapshot.version}`, description: value.snapshot.definition.code })))
const detail = ref<FreeDistributionRecord | null>(null); const detailError = ref(''); const detailLoading = ref(false)
const profiles = ref<V2IssuerProfile[]>([]); const profilesError = ref(''); const profilesLoading = ref(false)
const selectedKey = ref(''); const issuing = ref(false); const downloading = ref(false)
let detailRequest = 0
const keyOptions = computed(() => profiles.value.filter(value => value.policy.sources.includes('free_distribution') && value.policy.bindings.includes('unbound')).map(value => ({ value: value.key_id, label: value.key_id })))
const profile = computed(() => profiles.value.find(value => value.key_id === selectedKey.value))
const statusName = (value: string) => ({ approved: '已批准', prepared: '待完成签发', issued: '已签发' }[value] ?? value)
const statusOptions = [{ value: '', label: '全部状态' }, { value: 'approved', label: '待签发' }, { value: 'prepared', label: '签发待恢复' }, { value: 'issued', label: '已签发' }]
const filteredRecords = computed(() => statusFilter.value ? records.value.filter(record => record.status === statusFilter.value) : records.value)
const pagedRecords = computed(() => filteredRecords.value.slice((page.value - 1) * pageSize.value, page.value * pageSize.value))
const message = (value: unknown, fallback: string) => value instanceof Error ? value.message : fallback
function upsert(record: FreeDistributionRecord) { records.value = [record, ...records.value.filter(value => value.snapshot.id !== record.snapshot.id)] }
async function load() {
  if (loading.value) return
  loading.value = true; listLoadError.value = ''
  try { records.value = await listFreeDistributions() } catch (value) { listLoadError.value = message(value, '读取分发记录失败') } finally { loading.value = false }
}
async function loadPlans() {
  if (plansLoading.value) return
  plansLoading.value = true; plansError.value = ''
  try { plans.value = await listCommercialPlans(100) } catch (value) { plansError.value = message(value, '读取套餐失败') } finally { plansLoading.value = false }
}
function startApproval() {
  approvalError.value = ''; approvalPassword.value = ''
  if (pending.value) Object.assign(form, { planID: pending.value.plan_id, starts: pending.value.not_before.slice(0, 19), reason: pending.value.reason })
  else { Object.assign(form, { planID: '', starts: '', reason: '' }); void loadPlans() }
  approvalOpen.value = true
}
function closeApproval() { approvalOpen.value = false; approvalPassword.value = '' }
async function approve() {
  if (approving.value || journalError.value || (!pending.value && (plansLoading.value || plansError.value))) return
  approvalError.value = ''
  if (!approvalPassword.value) { approvalError.value = '请输入当前密码'; return }
  if (!pending.value) {
    try {
      if (!plan.value || plan.value.snapshot.definition.offer.kind !== 'free') throw new Error('请选择固定免费套餐版本')
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}(:\d{2})?$/.test(form.starts)) throw new Error('请明确填写 UTC 生效时间')
      const value = form.starts.length === 16 ? `${form.starts}:00` : form.starts
      const notBefore = new Date(`${value}Z`).toISOString()
      if (notBefore !== `${value}.000Z`) throw new Error('生效日期无效')
      if (!form.reason.trim()) throw new Error('请填写批准说明')
      journal.prepare({ operation_id: `distribution_${crypto.randomUUID()}`, plan_id: plan.value.snapshot.plan_id, plan_version: plan.value.snapshot.version, expected_sha256: plan.value.sha256, not_before: notBefore, reason: form.reason.trim() })
    } catch (value) { approvalError.value = message(value, '请核对分发配置'); return }
  }
  approving.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    attempt = journal.begin()
    const record = await approveFreeDistribution(pending.value!, approvalPassword.value)
    upsert(record); journal.clear(); approvalOpen.value = false
    toast.success('免费分发已批准')
  } catch (value) {
    const definite = !!attempt && value instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(value.status) && journal.reject(attempt)
    approvalError.value = message(value, '批准失败') + (definite ? '。配置已保留，请核对后重试' : '。结果尚未确认，请重试原请求')
  } finally { approvalPassword.value = ''; approving.value = false }
}
async function show(record: FreeDistributionRecord) {
  issuePassword.value = ''
  const request = ++detailRequest
  detail.value = record; detailLoading.value = true; detailError.value = ''; selectedKey.value = record.claims?.key_id ?? ''
  profiles.value = []; profilesError.value = ''; profilesLoading.value = true
  const [latest, keys] = await Promise.allSettled([getFreeDistribution(record.snapshot.id), listV2IssuerProfiles()])
  if (request !== detailRequest) return
  if (latest.status === 'fulfilled') { detail.value = latest.value; selectedKey.value = latest.value.claims?.key_id ?? ''; upsert(latest.value) }
  else detailError.value = message(latest.reason, '读取分发记录失败')
  if (keys.status === 'fulfilled') profiles.value = keys.value
  else profilesError.value = message(keys.reason, '读取签发配置失败')
  detailLoading.value = false; profilesLoading.value = false
}
function closeDetail() { detailRequest++; detail.value = null; detailLoading.value = false; profilesLoading.value = false; issuePassword.value = '' }
async function issue() {
  if (!detail.value || issuing.value || detailLoading.value || detailError.value || profilesLoading.value || profilesError.value || !selectedKey.value) return
  const id = detail.value.snapshot.id
  if (!issuePassword.value) { toast.error('请输入当前密码'); return }
  issuing.value = true; detailError.value = ''
  try { const record = await issueFreeDistribution(id, selectedKey.value, issuePassword.value); detail.value = record; upsert(record); toast.success('免费授权已签发') }
  catch (value) { detailError.value = message(value, '签发失败') + '。请重新读取此记录后继续，已准备的声明保持不变' }
  finally { issuePassword.value = ''; issuing.value = false }
}
async function download() {
  if (!detail.value || downloading.value || detailLoading.value || detailError.value) return
  downloading.value = true
  try { await downloadFreeDistribution(detail.value); toast.success('授权文件摘要已核对') }
  catch (value) { detailError.value = message(value, '下载失败') }
  finally { downloading.value = false }
}
async function copy(value: string) { try { await navigator.clipboard.writeText(value); toast.success('已复制') } catch { toast.error('复制失败，请检查剪贴板权限') } }
onMounted(load)
watch(statusFilter, () => { page.value = 1 })
watch(pageSize, () => { page.value = 1 })
</script>

<template>
  <section class="content paginated-page">
    <div class="page-head"><div><p class="eyebrow">FREE LICENSES</p><h1>免费授权</h1><p>从固定免费套餐批准分发，签发免机器绑定的完整权益授权。</p></div><AButton icon="plus" @click="startApproval">{{ pending ? '继续批准' : '批准免费授权' }}</AButton></div>
    <div class="distribution-toolbar"><span>批准、签发和正式打包分别记录，保存套餐不会自动授权。</span><AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton></div>
    <div class="filter-bar"><label class="field"><span>状态</span><ASelect v-model="statusFilter" aria-label="免费授权状态" :options="statusOptions" /></label><div class="filter-actions"><AButton variant="secondary" @click="statusFilter = ''">重置</AButton></div></div>
    <p v-if="journalError" class="distribution-error" role="alert">{{ journalError }}</p>
    <p v-else-if="pending" class="distribution-note">有一笔结果尚未确认的批准，请继续核对原请求。</p>
    <div class="table-wrap paginated-scroll"><ALoadingState v-if="loading && !records.length" label="正在读取分发记录" />
      <div v-else-if="listLoadError" class="distribution-error" role="alert"><strong>免费分发记录未能加载</strong><p>{{ listLoadError }}</p><AButton variant="secondary" @click="load">重新读取分发列表</AButton></div>
      <table v-else-if="pagedRecords.length" class="flat-data-table"><thead><tr><th>分发</th><th>套餐</th><th>版本</th><th>生效时间</th><th>状态</th><th>操作</th></tr></thead><tbody>
        <tr v-for="record in pagedRecords" :key="record.snapshot.id"><td>{{ record.snapshot.id }}</td><td>{{ record.snapshot.plan.definition.name }}</td><td>v{{ record.snapshot.plan.version }}</td><td>{{ record.snapshot.not_before }}</td><td><span class="status" :class="{ warning: record.status !== 'issued' }">{{ statusName(record.status) }}</span></td><td><AButton size="small" variant="secondary" @click="show(record)">查看</AButton></td></tr>
      </tbody></table><AEmpty v-else title="还没有免费分发" text="选择已保存的免费套餐版本，核对后批准分发。" />
    </div>
    <APagination v-if="filteredRecords.length > 0" v-model:page="page" v-model:page-size="pageSize" :total="filteredRecords.length" :loading="loading" />
    <AModal :open="approvalOpen" title="批准免费分发" description="本操作固定免费版本、生效时间和批准说明，不执行网站发布或安装包发行。" :close-disabled="approving" @close="closeApproval">
      <form class="form" @submit.prevent="approve">
        <p v-if="approvalError" class="distribution-error" role="alert">{{ approvalError }}</p>
        <p v-if="plansError" class="distribution-error" role="alert">{{ plansError }} <AButton variant="secondary" @click="loadPlans">重新读取</AButton></p>
        <ALoadingState v-if="plansLoading" label="正在读取免费套餐" />
        <dl v-if="pending" class="distribution-info"><dt>套餐版本</dt><dd>{{ pending.plan_id }} v{{ pending.plan_version }}</dd><dt>生效时间</dt><dd>{{ pending.not_before }}</dd><dt>批准说明</dt><dd>{{ pending.reason }}</dd></dl>
        <fieldset v-else class="distribution-fields" :disabled="approving || plansLoading">
          <label class="field"><span>免费套餐版本</span><ASelect v-model="form.planID" aria-label="免费套餐版本" :options="planOptions" required searchable /></label>
          <label class="field"><span>生效时间（UTC）</span><input v-model="form.starts" type="datetime-local" step="1" aria-label="生效时间（UTC）" required><small>填写 UTC 时间，北京时间减去 8 小时。</small></label>
          <label class="field"><span>批准说明</span><textarea v-model="form.reason" aria-label="批准说明" maxlength="2000" required /></label>
        </fieldset>
        <CommercialPlanSummary v-if="plan" :definition="plan.snapshot.definition" />
        <label class="field"><span>当前密码</span><input v-model="approvalPassword" aria-label="当前密码" type="password" autocomplete="current-password" :disabled="approving" required><small>用于确认本次批准，重试时需要重新输入。</small></label>
        <div class="form-actions"><AButton type="button" variant="secondary" :disabled="approving" @click="closeApproval">返回</AButton><AButton type="submit" :loading="approving" :disabled="!!journalError || (!pending && (!!plansError || plansLoading))">{{ pending ? '重试原请求' : '确认批准' }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="!!detail" title="免费分发详情" :close-disabled="issuing || downloading" @close="closeDetail">
      <template v-if="detail">
        <p v-if="detailError" class="distribution-error" role="alert">{{ detailError }}</p>
        <ALoadingState v-if="detailLoading" label="正在核对分发记录" />
        <ACopyCode :value="detail.snapshot.id" label="复制分发编号" copied-label="已复制" @copy="copy(detail.snapshot.id)" />
        <dl class="distribution-info"><dt>状态</dt><dd>{{ statusName(detail.status) }}</dd><dt>生效时间</dt><dd>{{ detail.snapshot.not_before }}</dd><dt>批准人</dt><dd>{{ detail.snapshot.approved_by }}</dd><dt>批准时间</dt><dd>{{ detail.snapshot.approved_at }}</dd><dt>批准说明</dt><dd>{{ detail.snapshot.reason }}</dd></dl>
        <CommercialPlanSummary :definition="detail.snapshot.plan.definition" />
        <p v-if="profilesError" class="distribution-note">{{ profilesError }}</p>
        <template v-if="detail.status !== 'issued'">
          <label class="field"><span>受限签发密钥</span><ASelect v-model="selectedKey" aria-label="受限签发密钥" :options="keyOptions" :disabled="!!detail.claims || profilesLoading || issuing" /></label>
          <p v-if="!profilesLoading && !profilesError && !keyOptions.length" class="distribution-note">未配置可用于免费分发的 v2 签发密钥。请按运营指南配置受限密钥后重试。</p>
          <details v-if="profile"><summary>查看签发范围</summary><ACopyCode :value="JSON.stringify(profile, null, 2)" label="复制公开签发配置" copied-label="已复制" @copy="copy(JSON.stringify(profile, null, 2))" /></details>
          <label class="field"><span>当前密码</span><input v-model="issuePassword" aria-label="当前密码" type="password" autocomplete="current-password" :disabled="issuing" required><small>用于确认本次签发，密码不会保存在分发记录中。</small></label>
          <div class="form-actions"><AButton variant="secondary" :disabled="issuing || downloading" @click="show(detail)">重新读取</AButton><AButton :loading="issuing" :disabled="!selectedKey || !!detailError || !!profilesError || profilesLoading || detailLoading" @click="issue">{{ detail.status === 'prepared' ? '继续签发' : '签发授权' }}</AButton></div>
        </template>
        <template v-else><p class="distribution-note">授权文件 SHA-256</p><ACopyCode :value="detail.document_sha256 || ''" label="复制授权摘要" copied-label="已复制" @copy="copy(detail.document_sha256 || '')" /><div class="form-actions"><AButton variant="secondary" :disabled="issuing || downloading" @click="show(detail)">重新读取</AButton><AButton :loading="downloading" :disabled="!!detailError || detailLoading" @click="download">下载授权文件</AButton></div></template>
      </template>
    </AModal>
  </section>
</template>

<style scoped>
.distribution-toolbar { flex:0 0 auto;display: flex; justify-content: space-between; align-items: center; gap: 16px; margin: 0 0 14px; }
.distribution-toolbar span, .distribution-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; }
.distribution-note { margin: 16px 0; }
.distribution-error { border: 1px solid var(--line); border-radius: 10px; padding: 12px; font-size:var(--font-size-body); line-height: 1.6; }
.distribution-fields { display: grid; gap: 16px; border: 0; margin: 0; padding: 0; min-width: 0; }
.distribution-info { display: grid; grid-template-columns: 100px minmax(0, 1fr); gap: 12px; font-size:var(--font-size-body); line-height: 1.5; }
.distribution-info dt { color: var(--muted); }
.distribution-info dd { margin: 0; overflow-wrap: anywhere; }
details { margin: 16px 0; }
summary { cursor: pointer; margin-bottom: 12px; }
</style>
