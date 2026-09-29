<script setup lang="ts">
import { computed, nextTick, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { AButton, ACopyCode, AEmpty, ALoadingState, AModal, ASelect, useToast } from '@aster/ui'
import { approvePublicCatalog, downloadPublicCatalog, exportPublicCatalog, getCatalogApproval, getCurrentOperationsOperatorID, listCatalogApprovals, listCommercialPlans, previewPublicCatalog, OperationsAPIError, type ApprovePublicCatalogInput, type CatalogApprovalRecord, type CommercialPlanRecord, type PublicCatalogPreview, type PublicCatalogRequest } from '../api/client'
import PublicCatalogSummary from '../components/PublicCatalogSummary.vue'
import ReauthActionModal from '../components/ReauthActionModal.vue'
import { submissionJournal } from '../commercial/submission-journal'

const props = withDefaults(defineProps<{ embedded?: boolean; initialPlanId?: string }>(), { embedded: false, initialPlanId: '' })
type Pending = { operation_id: string; approval: ApprovePublicCatalogInput; revision: string }
type SelectedPlan = PublicCatalogRequest['plans'][number] & { name: string }
const toast = useToast()
const route = useRoute()
const router = useRouter()
const records = ref<CatalogApprovalRecord[]>([]); const loading = ref(false); const loadError = ref('')
const plans = ref<CommercialPlanRecord[]>([]); const plansLoading = ref(false); const plansError = ref('')
const open = ref(false); const busy = ref(false); const formError = ref('')
const environment = ref<'local' | 'production'>('local'); const reason = ref(''); const selection = ref('')
const selected = ref<SelectedPlan[]>([]); const confirmation = ref<'approve' | 'export' | null>(null)
const preview = ref<PublicCatalogPreview | null>(null); const reviewed = ref<PublicCatalogRequest | null>(null)
const previewStart = ref<HTMLElement | null>(null)
const journal = submissionJournal<Pending>('catalog', getCurrentOperationsOperatorID())
const pending = journal.pending; const journalError = journal.error
const options = computed(() => plans.value.filter(p => !selected.value.some(s => s.plan_id === p.snapshot.plan_id)).map(p => ({ value: p.snapshot.plan_id, label: `${p.snapshot.definition.name} · v${p.snapshot.version}`, description: p.snapshot.definition.code })))
const detailOpen = ref(false); const detail = ref<CatalogApprovalRecord | null>(null); const detailError = ref(''); const detailBusy = ref(false)
const detailID = ref('')
let detailRequest = 0
const message = (e: unknown, fallback: string) => e instanceof Error ? e.message : fallback
const statusLabel = (s: CatalogApprovalRecord['status']) => s === 'exported' ? '已导出' : '已批准'
function upsert(record: CatalogApprovalRecord) { records.value = [record, ...records.value.filter(r => r.snapshot.id !== record.snapshot.id)] }
async function load() {
  if (loading.value) return
  loading.value = true; loadError.value = ''
  try { records.value = await listCatalogApprovals() }
  catch (e) { records.value = []; loadError.value = message(e, '读取公开目录失败') }
  finally { loading.value = false }
}
async function loadPlans() {
  plansLoading.value = true; plansError.value = ''
  try { plans.value = await listCommercialPlans(100) }
  catch (e) { plans.value = []; plansError.value = message(e, '读取套餐版本失败') }
  finally { plansLoading.value = false }
}
function sameRequest(a: PublicCatalogRequest, b: PublicCatalogRequest) {
  return a.operation_id === b.operation_id && a.environment === b.environment && a.reason === b.reason && a.plans.length === b.plans.length && a.plans.every((p, i) => p.plan_id === b.plans[i]?.plan_id && p.version === b.plans[i]?.version && p.expected_sha256 === b.plans[i]?.expected_sha256)
}
async function focusPreview() {
  await nextTick()
  if (!open.value || !previewStart.value) return
  previewStart.value.focus({ preventScroll: true })
  const dialog = previewStart.value.closest<HTMLElement>('[role="dialog"]')
  if (dialog) dialog.scrollTop = 0
}
function writeLocation(next: { catalog?: string; creating?: boolean } = {}, method: 'push' | 'replace' = 'push') {
  const query = { ...route.query, catalog: next.catalog || undefined, catalog_new: next.creating ? '1' : undefined }
  if (route.query.catalog === query.catalog && route.query.catalog_new === query.catalog_new) return
  void router[method]({ path: route.path, query })
}
async function start(updateUrl = true) {
  open.value = true; confirmation.value = null; formError.value = ''; preview.value = null; reviewed.value = null
  if (updateUrl) writeLocation({ creating: true })
  if (!pending.value) {
    selected.value = []; selection.value = ''; reason.value = ''; environment.value = 'local'
    await loadPlans()
    if (props.initialPlanId) { selection.value = props.initialPlanId; add() }
    return
  }
  busy.value = true
  try {
    const saved = pending.value
    if (saved.approval.request.operation_id !== saved.operation_id) throw new Error('待确认记录不一致，请保留原记录并核对')
    try {
      const existing = await getCatalogApproval(saved.revision)
      if (!sameRequest(existing.snapshot.request, saved.approval.request) || existing.public.sha256 !== saved.approval.expected_public_sha256) throw new Error('原批准内容与待确认记录不一致')
      upsert(existing); journal.clear(); open.value = false; toast.success('原目录批准已恢复')
      await show(existing.snapshot.id, false)
      writeLocation({ catalog: existing.snapshot.id }, 'replace')
      return
    } catch (e) { if (!(e instanceof OperationsAPIError) || e.status !== 404) throw e }
    const current = await previewPublicCatalog(saved.approval.request)
    if (current.sha256 !== saved.approval.expected_public_sha256 || current.catalog.revision !== saved.revision) throw new Error('原预览内容已不同，请保留待确认操作并核对')
    preview.value = current; reviewed.value = saved.approval.request
    environment.value = saved.approval.request.environment; reason.value = saved.approval.request.reason
    selected.value = current.catalog.plans.map((p, i) => ({ ...saved.approval.request.plans[i]!, name: p.name }))
    await focusPreview()
  } catch (e) { formError.value = message(e, '核对原批准失败') }
  finally { busy.value = false }
}
function close() { open.value = false; confirmation.value = null; writeLocation() }
function add() { const p = plans.value.find(p => p.snapshot.plan_id === selection.value); if (p && !selected.value.some(s => s.plan_id === p.snapshot.plan_id)) selected.value.push({ plan_id: p.snapshot.plan_id, version: p.snapshot.version, expected_sha256: p.sha256, name: p.snapshot.definition.name }); selection.value = '' }
function move(index: number, delta: number) { const next = index + delta; if (next < 0 || next >= selected.value.length) return; const [item] = selected.value.splice(index, 1); if (item) selected.value.splice(next, 0, item) }
async function inspect() {
  if (busy.value || pending.value || plansLoading.value || plansError.value) return
  if (!reason.value.trim()) { formError.value = '请填写批准说明'; return }
  busy.value = true; formError.value = ''; preview.value = null
  const input: PublicCatalogRequest = { operation_id: `catalog_${crypto.randomUUID()}`, environment: environment.value, reason: reason.value.trim(), plans: selected.value.map(p => ({ plan_id: p.plan_id, version: p.version, expected_sha256: p.expected_sha256 })) }
  try { preview.value = await previewPublicCatalog(input); reviewed.value = input; await focusPreview() }
  catch (e) { formError.value = message(e, '预览失败'); reviewed.value = null }
  finally { busy.value = false }
}
async function approve(password: string) {
  if (busy.value || !preview.value || !reviewed.value || journalError.value) return
  if (!password) return
  formError.value = ''; busy.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    if (!pending.value) journal.prepare({ operation_id: reviewed.value.operation_id, approval: { request: reviewed.value, expected_public_sha256: preview.value.sha256 }, revision: preview.value.catalog.revision })
    attempt = journal.begin()
    const record = await approvePublicCatalog(pending.value!.approval, password)
    upsert(record); journal.clear(); open.value = false; confirmation.value = null; toast.success('公开目录已批准')
    await show(record.snapshot.id, false)
    writeLocation({ catalog: record.snapshot.id }, 'replace')
  } catch (e) {
    const absent = e instanceof OperationsAPIError && e.number === 67710
    if (absent) { journal.clear(); preview.value = null; reviewed.value = null; confirmation.value = null }
    else if (attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 422].includes(e.status)) journal.reject(attempt)
    formError.value = message(e, '批准失败') + (pending.value ? '。保留原请求以便继续核对' : '。请核对后重试')
  } finally { busy.value = false }
}
async function show(id: string, updateUrl = true) {
  const request = ++detailRequest
  detailOpen.value = true; detailID.value = id; detail.value = null; detailError.value = ''; detailBusy.value = true
  if (updateUrl) writeLocation({ catalog: id })
  try { const value = await getCatalogApproval(id); if (request === detailRequest) detail.value = value }
  catch (e) { if (request === detailRequest) detailError.value = message(e, '读取目录失败') }
  finally { if (request === detailRequest) detailBusy.value = false }
}
function closeDetail() {
  detailRequest++; detailOpen.value = false; detail.value = null; confirmation.value = null
  writeLocation()
}
async function restoreLocation() {
  const id = typeof route.query.catalog === 'string' ? route.query.catalog : ''
  if (id) {
    open.value = false
    if (!detailOpen.value || detailID.value !== id) await show(id, false)
    return
  }
  detailRequest++; detailOpen.value = false; detail.value = null
  if (route.query.catalog_new === '1') {
    if (!open.value) await start(false)
  } else open.value = false
}
async function exportCatalog(password: string) {
  if (!detail.value || detailBusy.value) return
  if (!password) return
  detailBusy.value = true; detailError.value = ''
  try { detail.value = await exportPublicCatalog(detail.value.snapshot.id, password); upsert(detail.value); confirmation.value = null; toast.success('公开目录已导出到运营主机') }
  catch (e) { detailError.value = message(e, '导出失败') + '。可重试同一目录，已批准内容保持不变' }
  finally { detailBusy.value = false }
}
async function download() {
  if (!detail.value || detailBusy.value || detailError.value) return
  detailBusy.value = true
  try { await downloadPublicCatalog(detail.value); toast.success('目录版本和文件摘要已核对') }
  catch (e) { detailError.value = message(e, '下载失败') }
  finally { detailBusy.value = false }
}
async function copy(value: string) { try { await navigator.clipboard.writeText(value); toast.success('已复制') } catch { toast.error('复制失败，请检查剪贴板权限') } }
onMounted(() => { void load(); void restoreLocation() })
watch(() => [route.query.catalog, route.query.catalog_new], () => { void restoreLocation() })
</script>

<template>
  <section class="content">
    <div v-if="!props.embedded || !open" class="page-head"><h1>公开套餐目录</h1><AButton :disabled="busy || !!journalError" @click="start()">{{ pending ? '继续批准' : '新建公开目录' }}</AButton></div>
    <div v-if="!props.embedded || !open" class="catalog-toolbar"><RouterLink to="/commercial/plans">套餐与权益</RouterLink><RouterLink to="/commercial/publications">官网核对与受理</RouterLink><AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton></div>
    <p v-if="journalError" class="catalog-error" role="alert">{{ journalError }}</p>
    <p v-else-if="pending" class="catalog-note">有一笔结果尚未确认的批准，请继续核对原请求。</p>
    <div v-if="!props.embedded || !open" class="table-wrap"><ALoadingState v-if="loading" label="正在读取公开目录" />
      <div v-else-if="loadError" class="catalog-error" role="alert">{{ loadError }} <AButton variant="secondary" @click="load">重新读取目录</AButton></div>
      <table v-else-if="records.length" class="flat-data-table"><thead><tr><th>目录</th><th>目标环境</th><th>套餐数</th><th>状态</th><th>批准时间</th><th>操作</th></tr></thead><tbody><tr v-for="record in records" :key="record.snapshot.id"><td>{{ record.snapshot.request.reason }}</td><td>{{ record.snapshot.request.environment === 'local' ? '本地验证' : '生产' }}</td><td>{{ record.public.catalog.plans.length }}</td><td>{{ statusLabel(record.status) }}</td><td>{{ record.snapshot.approved_at }}</td><td><AButton variant="secondary" @click="show(record.snapshot.id)">查看</AButton></td></tr></tbody></table>
      <AEmpty v-else title="暂无公开目录" />
    </div>
    <component :is="props.embedded ? 'section' : AModal" v-if="!props.embedded || open" :open="open" title="批准公开目录" description="只公开选中套餐的展示内容、价格与权益，批准说明保留在运营端。" :close-disabled="busy || plansLoading" :class="{ 'catalog-workspace': props.embedded }" @close="close">
      <h2 v-if="props.embedded">批准公开目录</h2>
      <form class="form" @submit.prevent="preview ? confirmation = 'approve' : inspect()">
        <p v-if="formError" class="catalog-error" role="alert">{{ formError }}</p><ALoadingState v-if="busy || plansLoading" label="正在核对目录" />
        <template v-if="!preview && !pending">
          <p v-if="plansError" class="catalog-error" role="alert">{{ plansError }} <AButton variant="secondary" @click="loadPlans">重新读取套餐</AButton></p>
          <fieldset class="catalog-fields" :disabled="busy || plansLoading">
            <label class="field"><span>目标环境</span><ASelect v-model="environment" aria-label="目标环境" :options="[{ value: 'local', label: '本地验证' }, { value: 'production', label: '生产' }]" /></label>
            <label class="field"><span>批准说明</span><textarea v-model="reason" aria-label="批准说明" maxlength="1000" required /></label>
            <div class="catalog-add-row"><label class="field"><span>添加套餐版本</span><ASelect v-model="selection" aria-label="添加套餐版本" :options="options" searchable /></label><AButton type="button" variant="secondary" :disabled="!selection || !!plansError" @click="add">添加套餐</AButton></div>
            <ol class="catalog-selection"><li v-for="(item, index) in selected" :key="item.plan_id"><span>{{ item.name }} · v{{ item.version }}</span><div><AButton type="button" variant="secondary" :disabled="index === 0" @click="move(index, -1)">上移</AButton><AButton type="button" variant="secondary" :disabled="index === selected.length - 1" @click="move(index, 1)">下移</AButton><AButton type="button" variant="secondary" @click="selected.splice(index, 1)">移除</AButton></div></li></ol>
          </fieldset>
        </template>
        <template v-if="preview"><div ref="previewStart" tabindex="-1"><PublicCatalogSummary :catalog="preview.catalog" /></div><p class="catalog-note">批准说明 {{ reviewed?.reason }}</p></template>
        <div class="form-actions"><AButton type="button" variant="secondary" :disabled="busy || plansLoading" @click="close">返回</AButton><AButton v-if="preview && !pending" type="button" variant="secondary" :disabled="busy" @click="preview = null; reviewed = null">调整选择</AButton><AButton v-if="!pending || preview" type="submit" :loading="busy" :disabled="plansLoading || !!journalError || (!preview && !!plansError)">{{ preview ? (pending ? '重试原批准' : '确认批准') : '预览公开内容' }}</AButton></div>
      </form>
    </component>
    <AModal :open="detailOpen" title="公开目录详情" :close-disabled="detailBusy" @close="closeDetail">
      <ALoadingState v-if="detailBusy" label="正在核对目录文件" /><p v-if="detailError" class="catalog-error" role="alert">{{ detailError }}</p>
      <AButton v-if="!detail && detailError" variant="secondary" :disabled="detailBusy" @click="show(detailID)">重新读取目录</AButton>
      <template v-if="detail"><ACopyCode :value="detail.snapshot.id" label="复制目录版本" copied-label="已复制" @copy="copy(detail.snapshot.id)" /><p>状态 {{ statusLabel(detail.status) }}</p><PublicCatalogSummary :catalog="detail.public.catalog" /><p class="catalog-note">运营主机导出路径 {{ detail.snapshot.request.environment }}/{{ detail.snapshot.id }}/plans.json</p><ACopyCode :value="detail.public.sha256" label="复制公开摘要" copied-label="已复制" @copy="copy(detail.public.sha256)" /><div class="form-actions"><AButton variant="secondary" :disabled="detailBusy" @click="show(detail.snapshot.id)">重新读取</AButton><AButton :disabled="detailBusy" @click="confirmation = 'export'">{{ detail.status === 'exported' ? '核对并重新导出' : '导出到运营主机' }}</AButton><AButton v-if="detail.status === 'exported'" variant="secondary" :disabled="detailBusy || !!detailError" @click="download">下载 plans.json</AButton></div></template>
    </AModal>
    <ReauthActionModal :open="!!confirmation" :title="confirmation === 'export' ? '确认导出目录' : '确认批准目录'" :busy="busy || detailBusy" :error="confirmation === 'export' ? detailError : formError" @close="confirmation = null" @submit="confirmation === 'export' ? exportCatalog($event) : approve($event)" />
  </section>
</template>

<style scoped>
.catalog-toolbar { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin-bottom: 16px; }
.catalog-fields { border: 0; padding: 0; margin: 0; min-width: 0; display: grid; gap: 16px; }
.catalog-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.6; overflow-wrap: anywhere; }
.catalog-error { color: var(--danger); line-height: 1.6; overflow-wrap: anywhere; }
.catalog-selection { display: grid; gap: 14px; padding-left: 20px; }
.catalog-selection li span { display: block; margin-bottom: 8px; overflow-wrap: anywhere; }
.catalog-selection li div { display: flex; gap: 8px; flex-wrap: wrap; }
.catalog-add-row { display: grid; grid-template-columns: minmax(0, 1fr) max-content; align-items: end; gap: 14px; min-width: 0; }
.catalog-add-row .a-button { align-self: end; }
.form-actions { flex-wrap: wrap; }
.catalog-workspace { display: flex; min-height: 0; flex: 1; flex-direction: column; gap: 22px; }
.catalog-workspace > h2 { margin: 0; }
.catalog-workspace > .form { display: flex; min-height: 0; flex: 1; flex-direction: column; gap: 22px; }
.catalog-workspace .catalog-fields { grid-template-columns: repeat(2, minmax(0, 1fr)); align-items: start; gap: 22px; }
.catalog-workspace .catalog-selection { grid-column: 1 / -1; margin: 0; }
.catalog-workspace .catalog-add-row { grid-column: 1 / -1; }
.catalog-workspace .catalog-note { grid-column: 1 / -1; }
.catalog-workspace .form-actions { margin-top: auto; }
@media (max-width: 900px) { .catalog-workspace .catalog-fields { grid-template-columns: 1fr; } }
</style>
