<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AEmpty, ALoadingState, ASelect } from '@aster/ui'
import WorkflowStepper from '../components/WorkflowStepper.vue'
import { listCommercialPlans, type CommercialPlanRecord } from '../api/client'
import PublicCatalogsView from './PublicCatalogsView.vue'
import PublicationsView from './PublicationsView.vue'
import ReleaseCenterView from './ReleaseCenterView.vue'
import ReleaseArtifactsView from './ReleaseArtifactsView.vue'
import EnvironmentUpgradesView from './EnvironmentUpgradesView.vue'

const route = useRoute()
const router = useRouter()
const routeStep = () => {
  const step = Number(route.query.step)
  return Number.isInteger(step) && step >= 1 && step <= 6 ? step : 1
}
const current = ref(routeStep())
const steps = ['选择版本', '批准目录', '公开发布', '构建产物', '验签验收', '环境升级']
const views = [ReleaseCenterView, ReleaseArtifactsView, EnvironmentUpgradesView]
const activeView = computed(() => views[current.value - 4]!)
const plans = ref<CommercialPlanRecord[]>([])
const selectedPlanID = ref(typeof route.query.plan === 'string' ? route.query.plan : '')
const loading = ref(false)
const loadError = ref('')
const selectedPlan = computed(() => plans.value.find((plan) => plan.snapshot.plan_id === selectedPlanID.value))
const planOptions = computed(() => plans.value.map((plan) => ({
  value: plan.snapshot.plan_id,
  label: `${plan.snapshot.definition.name} · v${plan.snapshot.version}`,
  description: plan.snapshot.definition.code,
})))

async function loadPlans() {
  loading.value = true
  loadError.value = ''
  try {
    plans.value = await listCommercialPlans(100)
    if (selectedPlanID.value && !plans.value.some((plan) => plan.snapshot.plan_id === selectedPlanID.value)) selectedPlanID.value = ''
  } catch (value) {
    loadError.value = value instanceof Error ? value.message : '读取套餐版本失败'
  } finally {
    loading.value = false
  }
}

function writeLocation(method: 'push' | 'replace' = 'push', clearDetails = false) {
  const query = { ...route.query, step: String(current.value), plan: selectedPlanID.value || undefined,
    catalog: clearDetails ? undefined : route.query.catalog, catalog_new: clearDetails ? undefined : route.query.catalog_new,
    publication: clearDetails ? undefined : route.query.publication, publication_new: clearDetails ? undefined : route.query.publication_new,
    upgrade: clearDetails ? undefined : route.query.upgrade }
  if (route.query.step === query.step && route.query.plan === query.plan && (!clearDetails || !route.query.catalog && !route.query.catalog_new && !route.query.publication && !route.query.publication_new && !route.query.upgrade)) return
  void router[method]({ path: route.path, query })
}
function selectStep(step: number) {
  current.value = step
  writeLocation('push', true)
}

onMounted(loadPlans)
watch(() => [route.query.step, route.query.plan], () => {
  current.value = routeStep()
  selectedPlanID.value = typeof route.query.plan === 'string' ? route.query.plan : ''
})
watch(selectedPlanID, () => writeLocation('replace'))
</script>

<template>
  <section class="content release-workflow">
    <header class="page-head"><h1>发布交付</h1></header>
    <WorkflowStepper :steps="steps" :current="current" interactive vertical @select="selectStep" />
    <div class="release-viewport">
      <section v-if="current === 1" class="release-plan-picker">
        <header class="release-plan-head"><h2>选择套餐版本</h2><AButton variant="secondary" :disabled="loading" @click="loadPlans">刷新</AButton></header>
        <p v-if="loadError" class="release-error" role="alert">{{ loadError }}</p>
        <ALoadingState v-if="loading && !plans.length" label="正在读取套餐版本" />
        <template v-else-if="plans.length">
          <label class="field"><span>套餐版本</span><ASelect v-model="selectedPlanID" :options="planOptions" searchable aria-label="套餐版本" /></label>
          <dl v-if="selectedPlan" class="release-plan-summary">
            <div><dt>套餐</dt><dd>{{ selectedPlan.snapshot.definition.name }}</dd></div>
            <div><dt>版本</dt><dd>v{{ selectedPlan.snapshot.version }}</dd></div>
            <div><dt>代码</dt><dd>{{ selectedPlan.snapshot.definition.code }}</dd></div>
            <div><dt>授权版本</dt><dd>{{ selectedPlan.snapshot.definition.edition }}</dd></div>
          </dl>
        </template>
        <AEmpty v-else-if="!loadError" title="暂无套餐版本" />
      </section>
      <PublicCatalogsView v-else-if="current === 2" embedded :initial-plan-id="selectedPlanID" />
      <PublicationsView v-else-if="current === 3" embedded />
      <component :is="activeView" v-else />
    </div>
    <footer class="workflow-actions">
      <AButton variant="secondary" :disabled="current === 1" @click="selectStep(current - 1)">上一步</AButton>
      <AButton :disabled="current === steps.length || (current === 1 && !selectedPlanID)" @click="selectStep(current + 1)">下一步</AButton>
    </footer>
  </section>
</template>

<style scoped>
.release-workflow{display:flex;min-height:0;flex-direction:column}.page-head{margin-bottom:18px}.page-head h1{margin:0}.release-viewport{min-height:0;flex:1;margin-top:18px;border:1px solid var(--line);border-radius:14px;background:var(--surface);overflow:hidden}.release-viewport :deep(>.content){height:100%;min-height:0;overflow:auto;padding:16px}.release-viewport :deep(>.content>.page-head){justify-content:flex-end}.release-viewport :deep(>.content>.page-head h1){display:none}.release-viewport :deep(.catalog-toolbar a),.release-viewport :deep(.publication-toolbar a){display:none}.workflow-actions{display:flex;justify-content:space-between;padding-top:14px}.release-plan-picker{height:100%;display:flex;flex-direction:column;gap:24px;padding:28px}.release-plan-head{display:flex;align-items:center;justify-content:space-between}.release-plan-head h2{margin:0}.release-plan-picker>.field{max-width:950px}.release-plan-summary{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:20px;max-width:950px;margin:0;padding:24px;border:1px solid var(--line);border-radius:12px;background:var(--surface-2)}.release-plan-summary>div{display:grid;gap:6px}.release-plan-summary dt{color:var(--muted)}.release-plan-summary dd{margin:0;font-weight:700}.release-error{color:var(--danger)}
.release-workflow{display:grid;grid-template-columns:230px minmax(0,1fr);grid-template-rows:auto minmax(0,1fr) auto;gap:0 18px;align-content:stretch}
.release-workflow>.page-head{grid-column:1/-1}
.release-workflow>.workflow-stepper{grid-column:1;grid-row:2/4;align-self:stretch}
.release-workflow>.release-viewport{grid-column:2;grid-row:2;margin:0;overflow:visible}
.release-workflow>.release-viewport :deep(>.content){height:auto;overflow:visible}
.release-workflow>.workflow-actions{grid-column:2;grid-row:3}
@media(max-width:1100px){.release-workflow{grid-template-columns:1fr;grid-template-rows:auto auto minmax(0,1fr) auto}.release-workflow>.workflow-stepper{grid-column:1;grid-row:2}.release-workflow>.release-viewport{grid-column:1;grid-row:3;margin-top:16px}.release-workflow>.workflow-actions{grid-column:1;grid-row:4}}
.release-viewport :deep(>.content>.page-head a){display:none}
</style>
