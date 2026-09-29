<script setup lang="ts">
import { onMounted, reactive, ref } from 'vue'
import { RouterLink } from 'vue-router'
import { AButton, ACopyCode, AEmpty, ALoadingState, AModal, useToast } from '@aster/ui'
import { freezeCommercialPlan, getCommercialPlan, getCurrentCommercialPlan, getCurrentOperationsOperatorID, listCommercialPlans, OperationsAPIError, type CommercialPlanDefinition, type CommercialPlanRecord, type FreezeCommercialPlanInput } from '../api/client'
import CommercialPlanFields from '../components/CommercialPlanFields.vue'
import CommercialPlanSummary from '../components/CommercialPlanSummary.vue'
import { definitionFromForm, emptyPlanForm, formFromDefinition, formatAmount } from '../commercial/plan-form'
import { submissionJournal } from '../commercial/submission-journal'

const toast = useToast()
const items = ref<CommercialPlanRecord[]>([])
const loading = ref(false)
const loadError = ref('')
const open = ref(false)
const saving = ref(false)
const formError = ref('')
const form = reactive(emptyPlanForm())
const base = ref<{ plan_id: string; version: number } | null>(null)
const journal = submissionJournal<FreezeCommercialPlanInput>('plan', getCurrentOperationsOperatorID())
const pending = journal.pending
const journalError = journal.error
if (pending.value) {
  try {
    Object.assign(form, formFromDefinition(pending.value.definition))
    if (pending.value.plan_id) base.value = { plan_id: pending.value.plan_id, version: pending.value.expected_version }
  } catch { journalError.value = '待确认套餐记录无法读取，请保留记录并核对原操作' }
}
const needsRebase = ref(false)
const conflictLoading = ref(false)
const conflictCandidate = ref<CommercialPlanRecord | null>(null)
const conflictDraft = ref<CommercialPlanDefinition | null>(null)
const detail = ref<CommercialPlanRecord | null>(null)
const detailError = ref('')
const detailLoading = ref(false)
const latestVersion = ref(0)
let detailRequest = 0

function message(error: unknown, fallback: string) { return error instanceof Error ? error.message : fallback }
async function load() {
  if (loading.value) return
  loading.value = true; loadError.value = ''
  try { items.value = await listCommercialPlans(100) }
  catch (error) { loadError.value = message(error, '读取套餐失败') }
  finally { loading.value = false }
}
function edit(record: CommercialPlanRecord | null) {
  // An ambiguous response must retry the same frozen request, even after closing.
  if (!pending.value && !needsRebase.value) {
    base.value = record ? { plan_id: record.snapshot.plan_id, version: record.snapshot.version } : null
    Object.assign(form, record ? formFromDefinition(record.snapshot.definition) : emptyPlanForm())
    formError.value = ''
  }
  open.value = true
}
async function save() {
  if (saving.value || needsRebase.value || journalError.value) return
  formError.value = ''
  if (!pending.value) {
    try {
      journal.prepare({
        operation_id: `plan_${crypto.randomUUID()}`, plan_id: base.value?.plan_id ?? '',
        expected_version: base.value?.version ?? 0, definition: definitionFromForm(form),
      })
    } catch (error) { formError.value = message(error, '请检查套餐配置'); return }
  }
  saving.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    attempt = journal.begin()
    const record = await freezeCommercialPlan(pending.value!)
    items.value = [record, ...items.value.filter(item => item.snapshot.plan_id !== record.snapshot.plan_id)]
    journal.clear(); open.value = false
    toast.success(`套餐版本 ${record.snapshot.version} 已保存`)
  } catch (error) {
    // Validation/permission failures are definitive. Network and 5xx responses
    // may have committed, so preserve both the payload and its operation ID.
    const confirmedVersionConflict = error instanceof OperationsAPIError && error.code === 'COMMERCIAL_PLAN_VERSION_CONFLICT'
    const definitive = confirmedVersionConflict || (!!attempt && error instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(error.status) && journal.reject(attempt))
    needsRebase.value = confirmedVersionConflict && !!base.value
    if (confirmedVersionConflict) journal.clear()
    formError.value = message(error, '保存套餐失败') + (needsRebase.value ? '。你的配置已保留，请比较最新版本' : definitive ? '。配置已保留，可修正后重试' : '。结果尚未确认，请重试原请求以核对保存结果')
  } finally { saving.value = false }
}
async function reviewConflict() {
  if (!base.value || conflictLoading.value) return
  conflictLoading.value = true
  try {
    const draft = definitionFromForm(form)
    const current = await getCurrentCommercialPlan(base.value.plan_id)
    if (current.snapshot.version <= base.value.version) throw new Error('当前版本未变化，请检查套餐代码或操作标识')
    conflictDraft.value = draft; conflictCandidate.value = current
  } catch (error) { formError.value = message(error, '读取最新版本失败') }
  finally { conflictLoading.value = false }
}
function acceptRebase() {
  if (!conflictCandidate.value) return
  base.value = { plan_id: conflictCandidate.value.snapshot.plan_id, version: conflictCandidate.value.snapshot.version }
  items.value = [conflictCandidate.value, ...items.value.filter(item => item.snapshot.plan_id !== base.value!.plan_id)]
  conflictCandidate.value = null; conflictDraft.value = null; needsRebase.value = false; formError.value = ''
  toast.success('已保留你的配置，请核对后保存新版本')
}
function show(record: CommercialPlanRecord) {
  detailRequest++; detailLoading.value = false; detailError.value = ''
  detail.value = record; latestVersion.value = record.snapshot.version
}
function closeDetail() { detailRequest++; detail.value = null; detailLoading.value = false }
async function viewVersion(version: number) {
  const record = detail.value
  if (!record || detailLoading.value || version < 1 || version > latestVersion.value) return
  const request = ++detailRequest
  detailLoading.value = true; detailError.value = ''
  try {
    const result = await getCommercialPlan(record.snapshot.plan_id, version)
    if (request === detailRequest) detail.value = result
  } catch (error) { if (request === detailRequest) detailError.value = message(error, '读取版本失败') }
  finally { if (request === detailRequest) detailLoading.value = false }
}
async function copy(text: string) {
  try { await navigator.clipboard.writeText(text); toast.success('已复制版本摘要') }
  catch { toast.error('复制失败，请检查浏览器剪贴板权限') }
}
function price(record: CommercialPlanRecord) {
  const offer = record.snapshot.definition.offer
  return offer.kind === 'annual' ? `${formatAmount(offer.annual_amount_minor, offer.currency)} / 年` : offer.kind === 'free' ? '免费' : '联系报价'
}
onMounted(load)
</script>

<template>
  <section class="content">
    <div class="page-head"><div><h1>套餐与权益</h1><p>每个版本固定功能、额度、价格和期限，已有订单保留成交时的完整权益。</p></div><AButton icon="plus" @click="edit(null)">{{ pending || needsRebase ? '继续保存' : '新增套餐' }}</AButton></div>
    <div class="commercial-toolbar"><RouterLink to="/commercial/plan-drafts">套餐草稿</RouterLink><RouterLink to="/commercial/catalogs">公开目录</RouterLink><AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton></div>
    <div v-if="loadError" class="commercial-error" role="alert">{{ loadError }}</div>
    <div v-if="journalError" class="commercial-error" role="alert">{{ journalError }}</div>
    <p v-else-if="pending" class="commercial-note">有一笔尚未确认结果的保存，请使用继续保存核对原请求。</p>
    <div class="table-wrap">
      <ALoadingState v-if="loading" label="正在读取套餐" />
      <table v-else-if="items.length" class="flat-data-table"><thead><tr><th>套餐</th><th>代码</th><th>当前版本</th><th>报价</th><th>保存时间</th><th>操作</th></tr></thead><tbody>
        <tr v-for="item in items" :key="item.snapshot.plan_id">
          <td><strong>{{ item.snapshot.definition.name }}</strong></td><td>{{ item.snapshot.definition.code }}</td><td>v{{ item.snapshot.version }}</td><td>{{ price(item) }}</td><td>{{ new Date(item.created_at).toLocaleString('zh-CN') }}</td>
          <td><div class="row-actions"><AButton variant="secondary" @click="show(item)">查看版本</AButton><AButton variant="secondary" @click="edit(item)">修订</AButton></div></td>
        </tr>
      </tbody></table>
      <AEmpty v-else-if="!loadError" title="还没有套餐" text="配置实际权益并保存第一个版本。保存版本不会自动公开报价或签发授权。" />
    </div>
    <p class="commercial-note">当前显示最近 100 个套餐。旧版记录与订单仍可通过旧版入口查看。</p>
    <AModal :open="open" :title="base ? `修订套餐 v${base.version + 1}` : '新增套餐'" description="保存后形成不可变版本，后续修改生成新版本。此处不执行公开报价或授权签发。" :close-disabled="saving || conflictLoading" @close="open = false">
      <form class="form" @submit.prevent="save">
        <div v-if="formError" class="commercial-error" role="alert">{{ formError }}</div>
        <div v-if="needsRebase" class="commercial-error"><p>其他操作已改变当前版本。先比较最新版本与你的配置，再决定是否生成下一版本。</p><AButton variant="secondary" :loading="conflictLoading" @click="reviewConflict">比较最新版本</AButton></div>
        <fieldset class="commercial-fields" :disabled="saving || !!pending || needsRebase"><CommercialPlanFields :form="form" :code-locked="!!base" /></fieldset>
        <div class="form-actions"><AButton variant="secondary" type="button" :disabled="saving || conflictLoading" @click="open = false">返回</AButton><AButton type="submit" :loading="saving" :disabled="needsRebase || !!journalError">{{ pending ? '重试原请求' : '保存版本' }}</AButton></div>
      </form>
    </AModal>
    <AModal :open="!!conflictCandidate" title="核对版本冲突" description="继续后将保留你的全部配置并生成下一版本。最新版本中的其他修改不会自动合并，请逐项核对。" @close="conflictCandidate = null">
      <template v-if="conflictCandidate && conflictDraft">
        <h3>已保存 v{{ conflictCandidate.snapshot.version }} · {{ conflictCandidate.snapshot.definition.name }}</h3><CommercialPlanSummary :definition="conflictCandidate.snapshot.definition" />
        <h3>你的配置 · {{ conflictDraft.name }}</h3><CommercialPlanSummary :definition="conflictDraft" />
        <div class="form-actions"><AButton variant="secondary" @click="conflictCandidate = null">返回检查</AButton><AButton @click="acceptRebase">保留我的配置</AButton></div>
      </template>
    </AModal>
    <AModal :open="!!detail" :title="`${detail?.snapshot.definition.name ?? '套餐'} v${detail?.snapshot.version ?? ''}`" @close="closeDetail">
      <template v-if="detail">
        <div class="commercial-version-nav"><AButton variant="secondary" :disabled="detailLoading || detail.snapshot.version <= 1" @click="viewVersion(detail.snapshot.version - 1)">上一版本</AButton><span>v{{ detail.snapshot.version }} / v{{ latestVersion }}</span><AButton variant="secondary" :disabled="detailLoading || detail.snapshot.version >= latestVersion" @click="viewVersion(detail.snapshot.version + 1)">下一版本</AButton></div>
        <p v-if="detailError" class="commercial-error" role="alert">{{ detailError }}</p>
        <ALoadingState v-if="detailLoading" label="正在读取版本" />
        <CommercialPlanSummary v-else :definition="detail.snapshot.definition" />
        <p class="commercial-note">版本摘要</p><ACopyCode :value="detail.sha256" label="复制版本摘要" copied-label="已复制" @copy="copy(detail.sha256)" />
        <p class="commercial-note">保存版本只固定内容，不代表已批准公开或签发。</p>
      </template>
    </AModal>
  </section>
</template>

<style scoped>
.commercial-toolbar, .commercial-version-nav { display: flex; align-items: center; justify-content: space-between; gap: 12px; margin: 12px 0 18px; }
.commercial-toolbar a { color: var(--muted); font-size:var(--font-size-body); }
.commercial-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; margin: 16px 0; }
.commercial-error { border: 1px solid var(--line); border-radius: 10px; padding: 12px; margin-bottom: 16px; font-size:var(--font-size-body); line-height: 1.6; }
.commercial-fields { display: grid; gap: 16px; border: 0; padding: 0; margin: 0; min-width: 0; }
</style>
