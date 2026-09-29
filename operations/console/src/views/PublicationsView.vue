<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { RouterLink, useRoute, useRouter } from 'vue-router'
import { AButton, ACopyCode, AEmpty, ALoadingState, AModal, ASelect, useToast } from '@aster/ui'
import { acceptPublication, getCatalogApproval, getCurrentOperationsOperatorID, getPublication, getPublicationHead, listCatalogApprovals, listPublications, listPublicationFailures, preparePublication, OperationsAPIError, type CatalogApprovalRecord, type PreparePublicationInput, type PublicationRecord, type PublicationFailure } from '../api/client'
import PublicCatalogSummary from '../components/PublicCatalogSummary.vue'
import ReauthActionModal from '../components/ReauthActionModal.vue'
import { submissionJournal } from '../commercial/submission-journal'

const props = withDefaults(defineProps<{ embedded?: boolean }>(), { embedded: false })
const toast = useToast()
const route = useRoute()
const router = useRouter()
const items = ref<PublicationRecord[]>([]); const loading = ref(false); const loadError = ref('')
const heads = ref({ local: '', production: '' })
const open = ref(false); const busy = ref(false); const formError = ref('')
const revision = ref(''); const reason = ref(''); const until = ref(''); const consent = ref(false)
const confirmation = ref<'prepare' | 'accept' | null>(null)
const catalog = ref<CatalogApprovalRecord | null>(null); const expectedHead = ref(''); const buildSHA = ref(''); const buildName = ref('')
const availableCatalogs = ref<CatalogApprovalRecord[]>([])
const catalogOptions = computed(() => availableCatalogs.value.filter(item => item.status === 'exported').map(item => ({
  value: item.snapshot.id,
  label: item.snapshot.request.reason,
  description: `${environmentLabel(item.snapshot.request.environment)} · ${item.snapshot.id}`,
})))
const buildIdentity = ref<{ revision: string; sha256: string; environment: string } | null>(null)
const journal = submissionJournal<PreparePublicationInput>('publication', getCurrentOperationsOperatorID())
const pending = journal.pending; const journalError = journal.error
const detailOpen = ref(false); const detailBusy = ref(false); const detailError = ref(''); const detailID = ref('')
const detail = ref<PublicationRecord | null>(null)
const failures = ref<PublicationFailure[]>([]); const detailHead = ref<string | null>(null); const sourceID = ref('')
const acceptUnknown = ref(false)
let detailRequest = 0
const environmentLabel = (environment: string) => environment === 'local' ? '本地验证' : '生产'
const message = (e: unknown, fallback: string) => e instanceof Error ? e.message : fallback
const ready = computed(() => !!pending.value || (!!catalog.value && !!buildSHA.value && !!until.value && !!reason.value.trim() && consent.value))
const detailCatalog = computed(() => detail.value?.snapshot.catalog)
const canReplace = computed(() => !acceptUnknown.value && detail.value?.status === 'prepared' && detailHead.value !== null && (detailHead.value !== detail.value.snapshot.request.expected_active_id || Date.parse(detail.value.snapshot.request.accept_until) <= Date.now()))
const failureLabels: Record<PublicationFailure['code'], string> = { target_not_configured: '核对地址未配置', content_unverified: '官网内容未通过核对', invalid_evidence: '核对证据不匹配或已失效', head_conflict: '其他发布已更新当前记录', deadline_expired: '受理或核对时限已过', commit_unconfirmed: '提交结果未确认 请读取原记录' }
const statusLabel = (item: PublicationRecord) => item.status === 'prepared' ? '待核对' : heads.value[item.snapshot.catalog.request.environment] === item.snapshot.id ? '最近已核对' : '历史已核对'
const dateLabel = (value: string) => new Date(value).toLocaleString('zh-CN', { hour12: false })
function upsert(item: PublicationRecord) { items.value = [item, ...items.value.filter(old => old.snapshot.id !== item.snapshot.id)] }
async function load() {
  if (loading.value) return
  loading.value = true; loadError.value = ''
  try {
    const [records, local, production] = await Promise.all([listPublications(), getPublicationHead('local'), getPublicationHead('production')])
    items.value = records; heads.value = { local, production }
  } catch (e) { items.value = []; loadError.value = message(e, '读取发布核对记录失败') }
  finally { loading.value = false }
}
function writeLocation(next: { publication?: string; creating?: boolean } = {}, method: 'push' | 'replace' = 'push') {
  const query = { ...route.query, publication: next.publication || undefined, publication_new: next.creating ? '1' : undefined }
  if (route.query.publication === query.publication && route.query.publication_new === query.publication_new) return
  void router[method]({ path: route.path, query })
}
async function start(updateUrl = true) {
  open.value = true; formError.value = ''; confirmation.value = null
  if (updateUrl) writeLocation({ creating: true })
  if (!pending.value) { revision.value = ''; reason.value = ''; until.value = ''; catalog.value = null; buildSHA.value = ''; buildName.value = ''; buildIdentity.value = null; consent.value = false; sourceID.value = '' }
  else {
    try { restoreInput(pending.value) } catch (e) { formError.value = message(e, '原请求记录损坏，请保留记录并核对') }
  }
  if (props.embedded) {
    try { availableCatalogs.value = await listCatalogApprovals() }
    catch (e) { formError.value = message(e, '读取目录版本失败') }
  }
}
function restoreInput(input: PreparePublicationInput) {
  const date = new Date(input.accept_until)
  const local = new Date(date.getTime() - date.getTimezoneOffset() * 60_000).toISOString().slice(0, 16)
  revision.value = input.catalog_revision; reason.value = input.reason; until.value = local
  expectedHead.value = input.expected_active_id; buildSHA.value = input.build_sha256
  catalog.value = null; consent.value = false; buildName.value = ''; buildIdentity.value = null
}
async function replaceCandidate() {
  if (!detail.value || !canReplace.value || busy.value || detailBusy.value || pending.value || journalError.value) return
  const original = detail.value
  start(); restoreInput(original.snapshot.request); sourceID.value = original.snapshot.id
  buildIdentity.value = { revision: original.snapshot.catalog.id, sha256: original.snapshot.catalog.public_sha256, environment: original.snapshot.catalog.request.environment }
  buildName.value = '原记录的构建清单'
  detailOpen.value = false; detail.value = null
  await readCatalog()
}
async function readCatalog() {
  if (busy.value || pending.value) return
  busy.value = true; formError.value = ''; catalog.value = null; consent.value = false
  try {
    const value = await getCatalogApproval(revision.value.trim())
    if (value.status !== 'exported') throw new Error('该目录尚未导出，请先在公开目录中完成导出')
    const current = await getPublicationHead(value.snapshot.request.environment)
    const bound = buildIdentity.value
    if (!bound || bound.revision !== value.snapshot.id || bound.sha256 !== value.public.sha256 || bound.environment !== value.snapshot.request.environment) { buildSHA.value = ''; buildName.value = ''; buildIdentity.value = null }
    catalog.value = value; revision.value = value.snapshot.id; expectedHead.value = current
  } catch (e) { formError.value = message(e, '读取目录失败') }
  finally { busy.value = false }
}
async function readBuild(event: Event) {
  const file = (event.target as HTMLInputElement).files?.[0]
  if (!file || !catalog.value || busy.value) return
  busy.value = true; formError.value = ''; buildSHA.value = ''; buildName.value = ''; consent.value = false
  try {
    if (file.size > 256 * 1024) throw new Error('构建清单超过支持的大小')
    const bytes = await file.arrayBuffer()
    const parsed = JSON.parse(new TextDecoder('utf-8', { fatal: true, ignoreBOM: true }).decode(bytes))
    const selected = catalog.value
    if (parsed.schema !== 'aster.website-release.v1' || parsed.catalog?.revision !== selected.snapshot.id || parsed.catalog?.sha256 !== selected.public.sha256 || parsed.catalog?.environment !== selected.snapshot.request.environment || parsed.catalog?.path !== `/catalog/${selected.snapshot.id}/plans.json` || !Array.isArray(parsed.files)) throw new Error('构建清单与当前目录不一致')
    buildSHA.value = [...new Uint8Array(await crypto.subtle.digest('SHA-256', bytes))].map(byte => byte.toString(16).padStart(2, '0')).join('')
    buildName.value = file.name; buildIdentity.value = { revision: selected.snapshot.id, sha256: selected.public.sha256, environment: selected.snapshot.request.environment }
  } catch (e) { formError.value = message(e, '读取构建清单失败') }
  finally { busy.value = false; (event.target as HTMLInputElement).value = '' }
}
async function save(password: string) {
  if (busy.value || journalError.value || !ready.value || !password) return
  formError.value = ''
  if (!pending.value) {
    try {
      const deadline = new Date(until.value)
      if (!Number.isFinite(deadline.getTime()) || deadline.getTime() <= Date.now()) throw new Error('请选择未来的受理截止时间')
      if (catalog.value?.snapshot.id !== revision.value.trim()) throw new Error('目录版本已改变，请重新读取')
      journal.prepare({ operation_id: `publication_${crypto.randomUUID()}`, catalog_revision: catalog.value!.snapshot.id, build_sha256: buildSHA.value, expected_active_id: expectedHead.value, accept_until: deadline.toISOString(), reason: reason.value.trim() })
    } catch (e) { formError.value = message(e, '请核对发布内容'); return }
  }
  busy.value = true
  let attempt: ReturnType<typeof journal.begin> | null = null
  try {
    attempt = journal.begin()
    const original = pending.value!
    const record = await preparePublication(original, password)
    const actual = record.snapshot.request
    if (Object.keys(original).some(key => actual[key as keyof PreparePublicationInput] !== original[key as keyof PreparePublicationInput])) throw new Error('返回记录与原发布请求不一致，请保留原记录核对')
    upsert(record); journal.clear(); open.value = false; confirmation.value = null; toast.success('发布核对请求已保存')
    await show(record.snapshot.id, false)
    writeLocation({ publication: record.snapshot.id }, 'replace')
  } catch (e) {
    if (e instanceof OperationsAPIError && e.number === 67716) { if (pending.value) restoreInput(pending.value); journal.clear(); confirmation.value = null; formError.value = message(e, '原请求未保存') + '。目录、说明和期限已保留，请重新读取目录、选择构建清单并确认受理范围' }
    else {
      const rejected = !!attempt && e instanceof OperationsAPIError && [400, 401, 403, 404, 409, 422].includes(e.status) && journal.reject(attempt)
      formError.value = message(e, '保存失败') + (rejected ? '。输入已保留，可修正后重试' : '。请重试原请求确认结果')
    }
  } finally { busy.value = false }
}
async function show(id: string, updateUrl = true) {
  const request = ++detailRequest
  detailOpen.value = true; detailID.value = id; detail.value = null; failures.value = []; detailHead.value = null; acceptUnknown.value = true; detailBusy.value = true; detailError.value = ''
  if (updateUrl) writeLocation({ publication: id })
  try {
    const value = await getPublication(id)
    const [events, current] = await Promise.all([listPublicationFailures(id), getPublicationHead(value.snapshot.catalog.request.environment)])
    if (request === detailRequest) { detail.value = value; failures.value = events; detailHead.value = current; heads.value[value.snapshot.catalog.request.environment] = current; acceptUnknown.value = false }
  }
  catch (e) { if (request === detailRequest) detailError.value = message(e, '读取发布记录失败') }
  finally { if (request === detailRequest) detailBusy.value = false }
}
function closeEditor() { open.value = false; confirmation.value = null; writeLocation() }
function closeDetail() {
  detailRequest++; detailOpen.value = false; detail.value = null; confirmation.value = null
  writeLocation()
}
async function restoreLocation() {
  const id = typeof route.query.publication === 'string' ? route.query.publication : ''
  if (id) {
    open.value = false
    if (!detailOpen.value || detailID.value !== id) await show(id, false)
    return
  }
  detailRequest++; detailOpen.value = false; detail.value = null
  if (route.query.publication_new === '1') {
    if (!open.value) await start(false)
  } else open.value = false
}
async function accept(password: string) {
  if (!detail.value || detailBusy.value || !password) return
  detailBusy.value = true; detailError.value = ''; acceptUnknown.value = true
  try {
    const record = await acceptPublication(detail.value.snapshot.id, password)
    confirmation.value = null
    detail.value = record; upsert(record); toast.success('官网核对通过并已记录受理范围')
    // A recovered old receipt need not still be the current channel head.
    await show(record.snapshot.id)
    await load()
  } catch (e) {
    // A failed response can follow a committed acceptance. Do not combine the
    // old prepared state with a fresh head and offer a new ID before read-back.
    failures.value = []; detailHead.value = null
    detailError.value = message(e, '核对结果未确认') + '。请重新读取原记录，核对实际状态与未完成尝试，或重试同一次核对'
  }
  finally { detailBusy.value = false }
}
async function copy(value: string) { try { await navigator.clipboard.writeText(value); toast.success('已复制') } catch { toast.error('复制失败，请检查剪贴板权限') } }
onMounted(() => { void load(); void restoreLocation() })
watch(() => [route.query.publication, route.query.publication_new], () => { void restoreLocation() })
</script>

<template>
  <section class="content">
    <div v-if="!props.embedded || !open" class="page-head"><h1>官网核对与受理</h1><AButton :disabled="!!journalError" @click="start()">{{ pending ? '继续原请求' : '新建核对' }}</AButton></div>
    <div v-if="!props.embedded || !open" class="publication-toolbar"><RouterLink to="/commercial/catalogs">公开套餐目录</RouterLink><AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton></div>
    <p v-if="journalError" role="alert" class="publication-error">{{ journalError }}</p>
    <p v-else-if="pending" class="publication-note">有一笔结果尚未确认的请求，请继续原请求。</p>
    <div v-if="!props.embedded || !open" class="table-wrap"><ALoadingState v-if="loading" label="正在读取发布核对记录" /><div v-else-if="loadError" class="publication-error" role="alert">{{ loadError }} <AButton variant="secondary" @click="load">重新读取</AButton></div>
      <table v-else-if="items.length" class="flat-data-table"><thead><tr><th>说明</th><th>环境</th><th>核对状态</th><th>受理截止</th><th>操作</th></tr></thead><tbody><tr v-for="item in items" :key="item.snapshot.id"><td>{{ item.snapshot.request.reason }}</td><td>{{ environmentLabel(item.snapshot.catalog.request.environment) }}</td><td>{{ statusLabel(item) }}</td><td>{{ dateLabel(item.snapshot.request.accept_until) }}</td><td><AButton variant="secondary" @click="show(item.snapshot.id)">查看</AButton></td></tr></tbody></table>
      <AEmpty v-else title="暂无官网核对记录" />
    </div>
    <component :is="props.embedded ? 'section' : AModal" v-if="!props.embedded || open" :open="open" title="准备官网核对" description="选择已导出的目录与实际构建清单，明确本次报价受理期限。" :close-disabled="busy" :class="{ 'publication-workspace': props.embedded }" @close="closeEditor">
      <h2 v-if="props.embedded">准备官网核对</h2>
      <form class="form" @submit.prevent="confirmation = 'prepare'">
        <p v-if="formError" class="publication-error" role="alert">{{ formError }}</p><ALoadingState v-if="busy" label="正在核对发布内容" />
        <template v-if="!pending">
          <div v-if="sourceID" class="publication-summary"><p>原记录保持不变 保存后将建立新核对请求</p><ACopyCode :value="sourceID" label="复制原发布记录" copied-label="已复制" @copy="copy(sourceID)" /></div>
          <div class="publication-catalog-row"><label class="field"><span>目录版本</span><ASelect v-if="props.embedded" v-model="revision" aria-label="目录版本" :options="catalogOptions" searchable required :disabled="busy" @update:model-value="catalog = null; consent = false" /><input v-else v-model="revision" aria-label="目录版本" :disabled="busy" required maxlength="64" @input="catalog = null; consent = false"></label><AButton variant="secondary" :disabled="busy || !revision" @click="readCatalog">读取目录</AButton></div>
          <p v-if="catalog" class="publication-note">当前基准 {{ expectedHead || '尚无已核对发布' }}</p>
          <template v-if="catalog"><p>目标环境 {{ environmentLabel(catalog.snapshot.request.environment) }}</p><PublicCatalogSummary :catalog="catalog.public.catalog" /><label class="field"><span>官网构建清单</span><input type="file" accept=".json,application/json" aria-label="官网构建清单" :disabled="busy" @change="readBuild"><small>选择本次生产构建输出的 website-release.json</small></label><p v-if="buildName" class="publication-note">已读取 {{ buildName }}</p><ACopyCode copied-label="已复制" v-if="buildSHA" :value="buildSHA" label="复制构建摘要" @copy="copy(buildSHA)" /></template>
          <label class="field"><span>受理截止时间（本地）</span><input v-model="until" type="datetime-local" aria-label="受理截止时间" :disabled="busy" required></label>
          <label class="field"><span>核对说明</span><textarea v-model="reason" aria-label="核对说明" :disabled="busy" maxlength="1000" required /></label>
          <label class="publication-consent"><input v-model="consent" type="checkbox" :disabled="busy || !catalog || !buildSHA" required><span>核对通过后，在截止时间前接受此目录公开年度套餐的原价格和年限，包括后续切换页面后的历史报价</span></label>
        </template>
        <div v-else class="publication-summary"><ACopyCode copied-label="已复制" :value="pending.catalog_revision" label="复制目录版本" @copy="copy(pending.catalog_revision)" /><p>受理截止 {{ dateLabel(pending.accept_until) }}</p><p>{{ pending.reason }}</p></div>
        <div class="form-actions"><AButton type="button" variant="secondary" :disabled="busy" @click="closeEditor">返回</AButton><AButton type="submit" :loading="busy" :disabled="!ready || !!journalError">{{ pending ? '重试原请求' : '保存核对请求' }}</AButton></div>
      </form>
    </component>
    <AModal :open="detailOpen" title="发布核对详情" :close-disabled="detailBusy" @close="closeDetail">
      <ALoadingState v-if="detailBusy" label="正在核对官网及发布记录" /><p v-if="detailError" role="alert" class="publication-error">{{ detailError }}</p>
      <AButton v-if="!detail && detailError" variant="secondary" :disabled="detailBusy" @click="show(detailID)">重新读取原记录</AButton>
      <template v-if="detail && detailCatalog">
        <p>{{ environmentLabel(detailCatalog.request.environment) }} · {{ acceptUnknown ? '本次结果待确认' : statusLabel(detail) }}</p><p>{{ detail.snapshot.request.reason }}</p>
        <ACopyCode copied-label="已复制" :value="detail.snapshot.id" label="复制发布记录" @copy="copy(detail.snapshot.id)" /><ACopyCode copied-label="已复制" :value="detailCatalog.id" label="复制目录版本" @copy="copy(detailCatalog.id)" /><ACopyCode copied-label="已复制" :value="detail.snapshot.request.build_sha256" label="复制构建摘要" @copy="copy(detail.snapshot.request.build_sha256)" />
        <p>受理截止 {{ dateLabel(detail.snapshot.request.accept_until) }}</p><p v-if="detail.evidence" class="publication-note">核对地址 {{ detail.evidence.origin }}<br>核对时间 {{ dateLabel(detail.evidence.observed_at) }}</p>
        <p class="publication-note">目录中的年度套餐按原价和已公开年限受理，免费及联系报价套餐不自动生成付费订单。核对操作读取已配置的站点，不上传或部署页面。</p>
        <section class="publication-history" aria-label="未完成核对记录"><h2>未完成核对记录</h2><p class="publication-note">显示最近 100 次未完成尝试 成功核对后仍保留原记录</p><p v-if="acceptUnknown" class="publication-note">请重新读取原记录以更新核对历史</p><ul v-else-if="failures.length"><li v-for="event in failures" :key="event.id"><p>{{ failureLabels[event.code] }}</p><p class="publication-note">{{ event.stage === 'verification' ? '官网核对' : '受理提交' }} · {{ dateLabel(event.created_at) }}</p></li></ul><p v-else class="publication-note">暂无记录</p></section>
        <div v-if="canReplace" class="publication-summary"><p>当前发布基准或期限已变化 可以保留原记录并重新准备</p><p class="publication-note">当前基准 {{ detailHead || '尚无已核对发布' }}</p><AButton variant="secondary" :disabled="detailBusy || !!pending || !!journalError" @click="replaceCandidate">以此记录准备新核对</AButton><p v-if="pending" class="publication-note">请先恢复页面顶部结果未知的原请求</p></div>
        <div class="form-actions"><AButton variant="secondary" :disabled="detailBusy" @click="show(detail.snapshot.id)">重新读取原记录</AButton><AButton v-if="detail.status === 'prepared'" :disabled="detailBusy" @click="confirmation = 'accept'">核对官网并批准受理</AButton></div>
      </template>
    </AModal>
    <ReauthActionModal :open="!!confirmation" :title="confirmation === 'accept' ? '确认官网核对' : '确认保存核对请求'" :busy="busy || detailBusy" :error="confirmation === 'accept' ? detailError : formError" @close="confirmation = null" @submit="confirmation === 'accept' ? accept($event) : save($event)" />
  </section>
</template>

<style scoped>
.publication-toolbar { display: flex; justify-content: space-between; align-items: center; gap: 12px; margin-bottom: 16px; }
.publication-note { color: var(--muted); font-size:var(--font-size-body); line-height: 1.7; overflow-wrap: anywhere; }
.publication-error { color: var(--danger); line-height: 1.6; overflow-wrap: anywhere; }
.publication-consent { display: flex; align-items: flex-start; gap: 10px; line-height: 1.7; }
.publication-consent input { flex: 0 0 auto; width: 16px; margin-top: 6px; }
.publication-catalog-row { display: grid; grid-template-columns: minmax(0, 1fr) max-content; align-items: end; gap: 14px; min-width: 0; }
.publication-summary { display: grid; gap: 12px; overflow-wrap: anywhere; }
.publication-history { margin-block: 20px; }
.publication-history h2 { font-size:var(--font-size-body); }
.publication-history ul { padding-left: 20px; }
.publication-history li { margin-block: 12px; }
.form-actions { flex-wrap: wrap; }
.publication-workspace { display: flex; min-height: 0; flex: 1; flex-direction: column; gap: 22px; }
.publication-workspace > h2 { margin: 0; }
.publication-workspace > .form { display: grid; min-height: 0; flex: 1; grid-template-columns: repeat(2, minmax(0, 1fr)); align-content: start; gap: 22px; }
.publication-workspace .publication-error,
.publication-workspace .publication-catalog-row,
.publication-workspace .publication-summary,
.publication-workspace .publication-consent,
.publication-workspace .form-actions { grid-column: 1 / -1; }
.publication-workspace .form-actions { align-self: end; }
@media (max-width: 900px) { .publication-workspace > .form { grid-template-columns: 1fr; } }
</style>
