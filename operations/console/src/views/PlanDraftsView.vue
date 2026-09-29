<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AEmpty, ALoadingState, AModal, ASelect, useToast } from '@aster/ui'
import { freezePlanDraft, getCurrentCommercialPlan, getCurrentOperationsOperatorID, getPlanDraft, listCommercialPlans, listPlanDrafts, savePlanDraft, OperationsAPIError, type CommercialPlanRecord, type FreezePlanDraftInput, type PlanDraftRecord, type SavePlanDraftInput } from '../api/client'
import CommercialPlanFields from '../components/CommercialPlanFields.vue'
import CommercialPlanSummary from '../components/CommercialPlanSummary.vue'
import WorkflowStepper from '../components/WorkflowStepper.vue'
import { definitionFromForm, emptyPlanForm, formFromDefinition } from '../commercial/plan-form'
import { submissionJournal } from '../commercial/submission-journal'

const toast = useToast()
const route = useRoute()
const router = useRouter()
const items = ref<PlanDraftRecord[]>([])
const plans = ref<CommercialPlanRecord[]>([])
const loading = ref(false)
const loadError = ref('')
const plansError = ref('')
const open = ref(false)
const editorStep = ref(1)
const editorSteps = ['基本信息', '功能权益', '资源额度', '报价规则']
const editorSections = ['identity', 'entitlements', 'quotas', 'offer'] as const
const routeStep = () => {
  const step = Number(route.query.step)
  return Number.isInteger(step) && step >= 1 && step <= editorSteps.length ? step : 1
}
const saving = ref(false)
const comparing = ref(false)
// Only modal validation and request recovery; normal operation results use toast.
const formError = ref('')
const form = reactive(emptyPlanForm())
const draftID = ref('')
const revision = ref(0)
const planID = ref('')
const version = ref(0)
const selection = ref('')
const planChoices = computed(() => [
  { value: '', label: '新套餐' },
  ...plans.value.map(plan => ({
    value: plan.snapshot.plan_id,
    label: `${plan.snapshot.definition.name} v${plan.snapshot.version}`,
  })),
])
const needsCompare = ref(false)
const latestDraft = ref<PlanDraftRecord | null>(null)
const latestPlan = ref<CommercialPlanRecord | null>(null)
const saveJournal = submissionJournal<SavePlanDraftInput>('draft', getCurrentOperationsOperatorID())
const pendingSave = saveJournal.pending
const saveJournalError = saveJournal.error
const freezeJournal = submissionJournal<FreezePlanDraftInput>('draft-freeze', getCurrentOperationsOperatorID())
const pendingFreeze = freezeJournal.pending
const freezeJournalError = freezeJournal.error
const freezeOpen = ref(false)
const freezeSelection = ref<PlanDraftRecord | null>(null)
const freezeLoading = ref(false)
const freezeError = ref('')
let locationRequest = 0
let freezeRequest = 0
let freezeLoadingID = ''
const fieldsLocked = computed(() => saving.value || comparing.value || !!pendingSave.value || needsCompare.value || !!latestPlan.value || !!latestDraft.value)
const localDefinition = computed(() => { try { return definitionFromForm(form) } catch { return null } })
function message(e: unknown, fallback: string) { return e instanceof Error ? e.message : fallback }
function restore(input: SavePlanDraftInput) {
  draftID.value = input.draft_id; revision.value = input.expected_revision
  planID.value = input.plan_id; version.value = input.expected_version
  Object.assign(form, formFromDefinition(input.definition))
}
if (pendingSave.value) {
  try { restore(pendingSave.value) } catch { saveJournalError.value = '待确认草稿无法读取，请保留原记录并核对操作' }
}
async function load() {
  if (loading.value) return
  loading.value = true; loadError.value = ''; plansError.value = ''
  const result = await Promise.allSettled([listPlanDrafts(), listCommercialPlans(100)])
  if (result[0].status === 'fulfilled') items.value = result[0].value
  else { items.value = []; loadError.value = message(result[0].reason, '读取草稿失败') }
  if (result[1].status === 'fulfilled') plans.value = result[1].value
  else { plans.value = []; plansError.value = message(result[1].reason, '读取套餐版本失败') }
  loading.value = false
}
function edit(record: PlanDraftRecord | null, updateUrl = true) {
  if (!pendingSave.value && !needsCompare.value) {
    draftID.value = record?.snapshot.draft_id ?? ''; revision.value = record?.snapshot.revision ?? 0
    planID.value = record?.snapshot.plan_id ?? ''; version.value = record?.snapshot.expected_version ?? 0
    selection.value = ''; formError.value = ''; latestPlan.value = null; latestDraft.value = null
    Object.assign(form, record ? formFromDefinition(record.snapshot.definition) : emptyPlanForm())
  }
  editorStep.value = 1
  open.value = true
  if (updateUrl) writeLocation(draftID.value ? { draft: draftID.value, step: 1 } : { mode: 'new', step: 1 })
}
function writeLocation(next: { draft?: string; step?: number; mode?: 'new'; freeze?: string } = {}, method: 'push' | 'replace' = 'push') {
  const query = { ...route.query }
  delete query.draft; delete query.step; delete query.mode; delete query.freeze
  if (next.draft) query.draft = next.draft
  if (next.step) query.step = String(next.step)
  if (next.mode) query.mode = next.mode
  if (next.freeze) query.freeze = next.freeze
  if (route.query.draft === query.draft && route.query.step === query.step && route.query.mode === query.mode && route.query.freeze === query.freeze) return
  void router[method]({ path: route.path, query })
}
function closeEditor() { open.value = false; writeLocation() }
function changeStep(step: number) {
  editorStep.value = step
  writeLocation(draftID.value ? { draft: draftID.value, step } : { mode: 'new', step })
}
async function restoreLocation() {
  const request = ++locationRequest
  const freezeID = typeof route.query.freeze === 'string' ? route.query.freeze : ''
  if (freezeID) {
    open.value = false
    if (freezeLoading.value && freezeLoadingID === freezeID) return
    if (freezeLoading.value) { freezeRequest++; freezeLoading.value = false; freezeLoadingID = '' }
    if (!freezeOpen.value || freezeSelection.value?.snapshot.draft_id !== freezeID) {
      try {
        const record = items.value.find(item => item.snapshot.draft_id === freezeID) ?? await getPlanDraft(freezeID)
        if (request !== locationRequest) return
        await inspectFreeze(record, false)
      } catch (value) { freezeError.value = message(value, '读取草稿失败') }
    }
    return
  }
  freezeRequest++
  freezeLoading.value = false
  freezeLoadingID = ''
  freezeOpen.value = false
  const id = typeof route.query.draft === 'string' ? route.query.draft : ''
  if (id) {
    if (pendingSave.value && draftID.value !== id) {
      writeLocation(draftID.value ? { draft: draftID.value, step: routeStep() } : { mode: 'new', step: routeStep() }, 'replace')
      return
    }
    if (!open.value || draftID.value !== id) {
      try {
        const record = items.value.find(item => item.snapshot.draft_id === id) ?? await getPlanDraft(id)
        if (request !== locationRequest) return
        edit(record, false)
      } catch (value) { loadError.value = message(value, '读取草稿失败'); open.value = false; return }
    }
    editorStep.value = routeStep()
    return
  }
  if (route.query.mode === 'new') {
    if (!open.value || draftID.value) edit(null, false)
    editorStep.value = routeStep()
    return
  }
  open.value = false
}
function selectBase() {
  const selected = plans.value.find(item => item.snapshot.plan_id === selection.value)
  planID.value = selected?.snapshot.plan_id ?? ''; version.value = selected?.snapshot.version ?? 0
  Object.assign(form, selected ? formFromDefinition(selected.snapshot.definition) : emptyPlanForm())
}
async function save() {
  if (saving.value || comparing.value || needsCompare.value || latestDraft.value || latestPlan.value || saveJournalError.value) return
  formError.value = ''
  if (!pendingSave.value) {
    try { saveJournal.prepare({ operation_id: `draft_${crypto.randomUUID()}`, draft_id: draftID.value, expected_revision: revision.value, plan_id: planID.value, expected_version: version.value, definition: definitionFromForm(form) }) }
    catch (e) { formError.value = message(e, '请检查草稿'); return }
  }
  saving.value = true
  let attempt: ReturnType<typeof saveJournal.begin> | null = null
  try {
    attempt = saveJournal.begin()
    const record = await savePlanDraft(pendingSave.value!)
    saveJournal.clear(); open.value = false; writeLocation({}, 'replace')
    toast.success(`草稿修订 ${record.snapshot.revision} 已保存`)
    await load()
  } catch (e) {
    const conflict = e instanceof OperationsAPIError && e.code === 'COMMERCIAL_DRAFT_REVISION_CONFLICT'
    if (conflict) { saveJournal.clear(); needsCompare.value = true }
    const definitive = conflict || (!!attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(e.status) && saveJournal.reject(attempt))
    formError.value = message(e, '保存草稿失败') + (conflict ? '。配置已保留，请比较最新修订' : definitive ? '。配置已保留，可修正后重试' : '。结果尚未确认，请重试原请求')
  } finally { saving.value = false }
}
async function compareDraft() {
  if (!draftID.value || comparing.value) return
  comparing.value = true; formError.value = ''
  try { latestDraft.value = await getPlanDraft(draftID.value) }
  catch (e) { formError.value = message(e, '读取最新草稿失败') }
  finally { comparing.value = false }
}
function retainDraft() {
  const record = latestDraft.value
  if (!record || !localDefinition.value) return
  draftID.value = record.snapshot.draft_id; revision.value = record.snapshot.revision
  planID.value = record.snapshot.plan_id; version.value = record.snapshot.expected_version
  latestDraft.value = null; needsCompare.value = false; formError.value = ''
}
async function comparePlan() {
  if (!planID.value || fieldsLocked.value) return
  comparing.value = true; formError.value = ''
  try {
    const current = await getCurrentCommercialPlan(planID.value)
    if (current.snapshot.version <= version.value) { toast.info('套餐基准已是当前版本'); return }
    latestPlan.value = current
  } catch (e) { formError.value = message(e, '读取当前套餐失败') }
  finally { comparing.value = false }
}
function retainOnPlan() {
  if (!latestPlan.value || !localDefinition.value) return
  version.value = latestPlan.value.snapshot.version; latestPlan.value = null
  toast.info('已保留你的配置，请保存新草稿修订后生成版本')
}
async function inspectFreeze(record: PlanDraftRecord | null, updateUrl = true) {
  if (freezeLoading.value) return
  const request = ++freezeRequest
  freezeLoadingID = record?.snapshot.draft_id ?? pendingFreeze.value?.draft_id ?? ''
  freezeOpen.value = true; freezeError.value = ''; freezeSelection.value = null; freezeLoading.value = true
  if (updateUrl) writeLocation({ freeze: record?.snapshot.draft_id ?? pendingFreeze.value?.draft_id ?? '' })
  try {
    const source = pendingFreeze.value
    if (source) {
      const stored = await getPlanDraft(source.draft_id, source.revision)
      if (stored.sha256 !== source.expected_sha256) throw new Error('待确认操作的草稿摘要与服务器不一致')
      if (request === freezeRequest) freezeSelection.value = stored
    } else if (record) {
      const stored = await getPlanDraft(record.snapshot.draft_id)
      if (request === freezeRequest) freezeSelection.value = stored
    }
  } catch (e) { if (request === freezeRequest) freezeError.value = message(e, '读取草稿失败') }
  finally { if (request === freezeRequest) { freezeLoading.value = false; freezeLoadingID = '' } }
}
async function freeze() {
  if (freezeLoading.value || freezeJournalError.value) return
  const selected = freezeSelection.value
  if (!selected) return
  freezeError.value = ''
  if (!pendingFreeze.value) {
    try { freezeJournal.prepare({ operation_id: `draftfreeze_${crypto.randomUUID()}`, draft_id: selected.snapshot.draft_id, revision: selected.snapshot.revision, expected_sha256: selected.sha256 }) }
    catch (e) { freezeError.value = message(e, '准备生成版本失败'); return }
  }
  freezeLoading.value = true
  let attempt: ReturnType<typeof freezeJournal.begin> | null = null
  try {
    attempt = freezeJournal.begin()
    const result = await freezePlanDraft(pendingFreeze.value!)
    freezeJournal.clear(); freezeOpen.value = false; writeLocation({}, 'replace')
    toast.success(`套餐版本 ${result.snapshot.version} 已生成`)
    await load()
  } catch (e) {
    const confirmed = e instanceof OperationsAPIError && ['COMMERCIAL_PLAN_VERSION_CONFLICT', 'COMMERCIAL_DRAFT_REVISION_CONFLICT'].includes(e.code)
    const definitive = confirmed || (!!attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(e.status) && freezeJournal.reject(attempt))
    if (confirmed) { freezeJournal.clear(); freezeSelection.value = null }
    freezeError.value = message(e, '生成套餐版本失败') + (confirmed ? '。请返回编辑草稿并核对最新修订与套餐基准' : definitive ? '。请核对选择后重试' : '。结果尚未确认，请重试原请求')
  } finally { freezeLoading.value = false }
}
onMounted(async () => { await load(); await restoreLocation() })
watch(() => [route.query.draft, route.query.step, route.query.mode, route.query.freeze], () => { void restoreLocation() })
</script>

<template>
  <section class="content plan-page">
    <div class="page-head">
      <h1>产品套餐</h1>
      <div class="inline-actions">
        <AButton v-if="open" variant="secondary" :disabled="saving || comparing" @click="closeEditor">返回列表</AButton>
        <AButton v-else icon="plus" @click="edit(null)">{{ pendingSave || needsCompare ? '继续保存' : '新建套餐' }}</AButton>
      </div>
    </div>

    <template v-if="!open">
      <div class="draft-toolbar">
        <AButton v-if="pendingFreeze" variant="secondary" @click="inspectFreeze(null)">继续生成版本</AButton>
        <span v-else></span>
        <AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton>
      </div>
      <div v-if="loadError || plansError || saveJournalError || freezeJournalError" class="draft-error" role="alert">{{ loadError || plansError || saveJournalError || freezeJournalError }}</div>
      <div class="table-wrap paginated-scroll">
        <ALoadingState v-if="loading" label="正在读取套餐" />
        <table v-else-if="!loadError && items.length" class="flat-data-table"><thead><tr><th>套餐</th><th>代码</th><th>修订</th><th>套餐基准</th><th>保存时间</th><th>操作</th></tr></thead><tbody>
          <tr v-for="item in items" :key="item.snapshot.draft_id"><td><strong>{{ item.snapshot.definition.name }}</strong></td><td>{{ item.snapshot.definition.code }}</td><td>r{{ item.snapshot.revision }}</td><td>{{ item.snapshot.expected_version ? `v${item.snapshot.expected_version}` : '新套餐' }}</td><td>{{ new Date(item.created_at).toLocaleString('zh-CN') }}</td><td><div class="row-actions"><AButton variant="secondary" @click="edit(item)">编辑草稿</AButton><AButton :disabled="!!pendingFreeze || !!pendingSave" @click="inspectFreeze(item)">生成版本</AButton></div></td></tr>
        </tbody></table>
        <AEmpty v-else-if="!loadError" title="暂无套餐" />
      </div>
    </template>

    <form v-else class="plan-editor" @submit.prevent="save">
      <WorkflowStepper :steps="editorSteps" :current="editorStep" vertical />
      <div v-if="formError || saveJournalError || needsCompare" class="editor-messages">
        <div v-if="formError || saveJournalError" class="draft-error" role="alert">{{ formError || saveJournalError }}</div>
        <AButton v-if="needsCompare" variant="secondary" :loading="comparing" @click="compareDraft">比较最新修订</AButton>
      </div>
      <div class="editor-viewport">
        <fieldset class="draft-fields" :disabled="fieldsLocked">
          <label v-if="!draftID && editorStep === 1" class="field"><span>套餐基准</span><ASelect v-model="selection" :options="planChoices" aria-label="草稿套餐基准" :disabled="!!plansError" @change="selectBase" /></label>
          <CommercialPlanFields :form="form" :code-locked="!!draftID || !!planID" :section="editorSections[editorStep - 1]!" />
        </fieldset>
        <aside class="editor-summary">
          <h3>{{ form.name || '新套餐' }}</h3>
          <dl><dt>代码</dt><dd>{{ form.code || '—' }}</dd><dt>授权版本</dt><dd>{{ form.edition || '—' }}</dd><dt>功能</dt><dd>{{ form.features.length + form.featureSets.length }}</dd><dt>报价</dt><dd>{{ form.kind === 'annual' ? '按年订阅' : form.kind === 'free' ? '免费' : form.kind === 'contact' ? '联系报价' : '—' }}</dd><dt>修订</dt><dd>r{{ revision }}</dd><dt>基准</dt><dd>{{ version ? `v${version}` : '新套餐' }}</dd></dl>
          <AButton v-if="draftID && planID" type="button" variant="secondary" @click="comparePlan">核对套餐当前版本</AButton>
        </aside>
      </div>
      <div class="editor-actions">
        <AButton type="button" variant="secondary" :disabled="editorStep === 1" @click="changeStep(editorStep - 1)">上一步</AButton>
        <AButton v-if="editorStep < editorSteps.length" type="button" @click="changeStep(editorStep + 1)">下一步</AButton>
        <AButton v-else type="submit" :loading="saving" :disabled="needsCompare || !!latestDraft || !!latestPlan || !!saveJournalError">{{ pendingSave ? '重试原请求' : '保存套餐' }}</AButton>
      </div>
    </form>

    <AModal :open="!!latestDraft || !!latestPlan" title="核对修改" description="保留你的配置会替代新基准中的对应配置，请逐项核对。原草稿与套餐版本继续保留。" @close="latestDraft = null; latestPlan = null">
      <template v-if="latestDraft || latestPlan"><h3>服务器当前配置</h3><p class="draft-identity">{{ latestDraft?.snapshot.definition.name ?? latestPlan!.snapshot.definition.name }} · {{ latestDraft ? `r${latestDraft.snapshot.revision}` : `v${latestPlan!.snapshot.version}` }}</p><CommercialPlanSummary :definition="(latestDraft?.snapshot.definition ?? latestPlan!.snapshot.definition)" /><h3>你的配置</h3><p class="draft-identity">{{ form.name }}</p><CommercialPlanSummary v-if="localDefinition" :definition="localDefinition" /><p v-else class="draft-error">请先补全当前配置</p><div class="form-actions"><AButton variant="secondary" @click="latestDraft = null; latestPlan = null">返回检查</AButton><AButton :disabled="!localDefinition" @click="latestDraft ? retainDraft() : retainOnPlan()">保留我的配置</AButton></div></template>
    </AModal>
    <AModal :open="freezeOpen" title="生成固定套餐版本" description="只采用下方已保存的草稿内容。生成版本不会自动公开报价或签发授权。" :close-disabled="freezeLoading" @close="freezeOpen = false; writeLocation()">
      <div v-if="freezeError" class="draft-error" role="alert">{{ freezeError }}</div>
      <ALoadingState v-if="freezeLoading" label="正在处理草稿" />
      <template v-else-if="freezeSelection"><h3 class="draft-identity">{{ freezeSelection.snapshot.definition.name }}</h3><p class="draft-identity">{{ freezeSelection.snapshot.definition.code }}</p><p>草稿 r{{ freezeSelection.snapshot.revision }} → 套餐 v{{ freezeSelection.snapshot.expected_version + 1 }}</p><CommercialPlanSummary :definition="freezeSelection.snapshot.definition" /></template>
      <div class="form-actions"><AButton variant="secondary" :disabled="freezeLoading" @click="freezeOpen = false; writeLocation()">返回</AButton><AButton :loading="freezeLoading" :disabled="!freezeSelection || !!freezeJournalError" @click="freeze">{{ pendingFreeze ? '重试原请求' : '生成固定版本' }}</AButton></div>
    </AModal>
  </section>
</template>

<style scoped>
.plan-page{display:flex;min-height:0;flex-direction:column}.draft-toolbar { display: flex; gap: 16px; align-items: center; justify-content: space-between; margin-bottom:14px; flex-wrap: wrap; }
.draft-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; margin: 16px 0; }
.draft-identity { overflow-wrap: anywhere; }
.draft-error { border: 1px solid var(--line); border-radius: 10px; padding: 12px; margin-bottom: 16px; font-size:var(--font-size-body); line-height: 1.6; }
.draft-fields { border: 0; min-width: 0; padding: 0; margin: 0; display: grid; gap: 16px; }
.plan-editor{display:grid;min-height:0;flex:1;grid-template-columns:190px minmax(0,1fr);grid-template-rows:auto minmax(0,1fr) auto;gap:8px 14px}.plan-editor>.workflow-stepper{grid-column:1;grid-row:1/4;min-height:0}.editor-messages{display:flex;grid-column:2;grid-row:1;align-items:center;gap:10px}.editor-messages .draft-error{margin:0}.editor-viewport{display:grid;min-height:0;grid-column:2;grid-row:2;grid-template-columns:minmax(0,1fr) 270px;gap:14px}.editor-viewport>.draft-fields,.editor-summary{border:1px solid var(--line);border-radius:14px;background:var(--surface);padding:22px}.editor-viewport>.draft-fields{align-content:start}.editor-summary h3{margin:0 0 20px}.editor-summary dl{display:grid;grid-template-columns:74px minmax(0,1fr);gap:14px 10px;margin:0 0 22px}.editor-summary dt{color:var(--muted);font-size:var(--font-size-body)}.editor-summary dd{margin:0;overflow-wrap:anywhere}.editor-actions{display:flex;grid-column:2;grid-row:3;align-items:center;justify-content:space-between}.page-head{display:flex;align-items:center;justify-content:space-between}.page-head h1{margin:0}@media(max-width:1100px){.plan-editor{grid-template-columns:1fr;grid-template-rows:auto auto minmax(0,1fr) auto}.plan-editor>.workflow-stepper{grid-column:1;grid-row:1}.editor-messages{grid-column:1;grid-row:2}.editor-viewport{grid-column:1;grid-row:3}.editor-actions{grid-column:1;grid-row:4}}@media(max-width:860px){.editor-viewport{grid-template-columns:1fr}.editor-summary{display:none}}
</style>
