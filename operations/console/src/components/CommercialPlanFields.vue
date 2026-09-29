<script setup lang="ts">
import { ACheckbox, ASelect } from '@aster/ui'
import { BUSINESS_OPERATIONS, CAPABILITIES, FEATURE_SETS, effectiveFeatures, QUOTAS } from '../api/generated/product-capabilities'
import { formatAmount, toggleCapability, type PlanForm } from '../commercial/plan-form'

withDefaults(defineProps<{
  form: PlanForm
  codeLocked: boolean
  section?: 'all' | 'identity' | 'entitlements' | 'quotas' | 'offer'
}>(), { section: 'all' })
const quoteKinds = [{ value: 'free', label: '免费' }, { value: 'annual', label: '按年订阅' }, { value: 'contact', label: '联系报价' }]
const limitModes = [{ value: 'limited', label: '限制数量' }, { value: 'unlimited', label: '不设商业上限' }]
const expiryModes = [{ value: 'none', label: '不设到期' }, { value: 'fixed', label: '固定到期时间' }]
const taxes = [{ value: 'inclusive', label: '金额含税' }, { value: 'none', label: '无税费' }]
const timezones = [{ value: 'Asia/Shanghai', label: '北京时间' }, { value: 'UTC', label: 'UTC' }]
</script>

<template>
  <div v-if="section === 'all' || section === 'identity'" class="two-columns">
    <label class="field"><span>套餐代码</span><input v-model="form.code" aria-label="套餐代码" required maxlength="128" :readonly="codeLocked" placeholder="team-20"></label>
    <label class="field"><span>套餐名称</span><input v-model="form.name" aria-label="套餐名称" required maxlength="160"></label>
  </div>
  <label v-if="section === 'all' || section === 'identity'" class="field"><span>套餐说明</span><textarea v-model="form.description" aria-label="套餐说明" maxlength="4000" rows="2"></textarea></label>
  <div v-if="section === 'all' || section === 'identity'" class="two-columns">
    <label class="field"><span>授权版本</span><input v-model="form.edition" aria-label="授权版本" required maxlength="128" placeholder="team"></label>
    <label class="field"><span>最低客户版本</span><input v-model="form.minimumVersion" aria-label="最低客户版本" required maxlength="64" placeholder="主版本.次版本.修订版本"></label>
  </div>
  <fieldset v-if="section === 'all' || section === 'entitlements'" class="field"><legend>功能集合</legend>
    <ACheckbox v-for="set in FEATURE_SETS" :key="set.id" :model-value="form.featureSets.includes(set.id)" :disabled="form.kind === 'free' && !form.featureSets.includes(set.id)" :label="set.label" @update:model-value="form.featureSets = $event === true ? [...form.featureSets.filter(id => id !== set.id), set.id] : form.featureSets.filter(id => id !== set.id)" />
  </fieldset>
  <fieldset v-if="section === 'all' || section === 'entitlements'" class="field"><legend>授权功能</legend><div class="commercial-feature-grid">
    <ACheckbox v-for="feature in CAPABILITIES" :key="feature.id" :model-value="form.features.includes(feature.id)" :label="feature.label" @update:model-value="form.features = toggleCapability(form.features, feature.id, $event === true)" />
  </div><button v-if="form.kind === 'free'" type="button" class="btn secondary" @click="form.features = CAPABILITIES.filter(feature => feature.free_default).map(feature => feature.id); form.featureSets = []">填入免费默认功能</button></fieldset>
  <fieldset v-if="section === 'all' || section === 'entitlements'" class="field"><legend>业务能力</legend>
    <p v-for="operation in BUSINESS_OPERATIONS" :key="operation.id" class="commercial-hint">
      {{ operation.label }} · {{ operation.requires.every(id => effectiveFeatures({ features: form.features, feature_sets: form.featureSets }).includes(id)) ? '已包含所需功能' : `还需 ${operation.requires.filter(id => !effectiveFeatures({ features: form.features, feature_sets: form.featureSets }).includes(id)).map(id => CAPABILITIES.find(feature => feature.id === id)?.label ?? id).join(' + ')}` }}
    </p>
  </fieldset>
  <fieldset v-if="section === 'all' || section === 'quotas'" class="field"><legend>资源额度</legend>
    <div v-for="quota in form.quotas" :key="quota.id" class="commercial-quota-row">
      <span>{{ QUOTAS.find(item => item.id === quota.id)?.label }}</span>
      <ASelect v-model="quota.mode" :aria-label="`${quota.id} 限制方式`" :options="limitModes" required />
      <input v-if="quota.mode === 'limited'" v-model="quota.value" :aria-label="`${quota.id} 数量`" inputmode="numeric" pattern="[0-9]+" required placeholder="数量">
      <span v-else class="commercial-hint">{{ quota.mode === 'unlimited' ? '不设商业上限' : '尚未选择' }}</span>
    </div>
  </fieldset>
  <div v-if="section === 'all' || section === 'quotas'" class="two-columns">
    <label class="field"><span>允许换机次数</span><input v-model="form.transferLimit" aria-label="允许换机次数" inputmode="numeric" pattern="[0-9]+" required></label>
    <label class="field"><span>支持条款版本</span><input v-model="form.supportTermsVersion" aria-label="支持条款版本" required maxlength="128" placeholder="support-v1"></label>
  </div>
  <label v-if="section === 'all' || section === 'offer'" class="field"><span>报价方式</span><ASelect v-model="form.kind" aria-label="报价方式" :options="quoteKinds" required /></label>
  <template v-if="(section === 'all' || section === 'offer') && form.kind === 'annual'">
    <div class="two-columns">
      <label class="field"><span>货币代码</span><input v-model="form.currency" aria-label="货币代码" required pattern="[A-Z]{3}" maxlength="3" placeholder="CNY"></label>
      <label class="field"><span>年度金额（最小货币单位）</span><input v-model="form.annualAmountMinor" aria-label="年度金额" inputmode="numeric" pattern="[0-9]+" required></label>
    </div>
    <p v-if="form.currency && /^[0-9]+$/.test(form.annualAmountMinor)" class="commercial-hint">年度价格 {{ formatAmount(Number(form.annualAmountMinor), form.currency) }}</p>
    <div class="two-columns">
      <label class="field"><span>税费方式</span><ASelect v-model="form.taxMode" aria-label="税费方式" :options="taxes" required /></label>
      <label class="field"><span>合同日历时区</span><ASelect v-model="form.timezone" aria-label="合同日历时区" :options="timezones" required /></label>
    </div>
    <fieldset class="field"><legend>可售期限与折扣</legend>
      <div v-for="term in form.terms" :key="term.years" class="commercial-term-row">
        <ACheckbox v-model="term.enabled" :label="`${term.years} 年`" />
        <label v-if="term.enabled" class="commercial-percent"><input v-model="term.discountPercent" :aria-label="`${term.years} 年折扣比例`" inputmode="decimal" required placeholder="100"><span>%</span></label>
      </div>
    </fieldset>
  </template>
  <template v-else-if="(section === 'all' || section === 'offer') && form.kind === 'free'">
    <label class="field"><span>免费到期方式</span><ASelect v-model="form.expiryMode" aria-label="免费到期方式" :options="expiryModes" required /></label>
    <label v-if="form.expiryMode === 'fixed'" class="field"><span>到期时间（UTC）</span><input v-model="form.expiresAt" aria-label="免费到期时间" required placeholder="2027-01-01T00:00:00Z"></label>
  </template>
</template>

<style scoped>
.field { align-content: start; }
fieldset.field { border: 0; padding: 0; margin: 0; }
fieldset.field legend { margin-bottom: 10px; font-size:var(--font-size-body); font-weight: 650; }
.commercial-feature-grid { display: grid; gap: 10px; }
.commercial-quota-row { display: grid; grid-template-columns: 96px minmax(0, 1fr) minmax(80px, 1fr); align-items: center; gap: 10px; margin-top: 10px; }
.commercial-quota-row input, .commercial-percent input { width: 100%; min-width: 0; }
.commercial-term-row { display: grid; grid-template-columns: 100px minmax(0, 1fr); align-items: center; gap: 10px; min-height: 42px; }
.commercial-percent { display: flex; align-items: center; gap: 8px; }
.commercial-hint { color: var(--muted); font-size:var(--font-size-body); line-height: 1.6; margin: 0; }
@media (max-width: 520px) { .commercial-quota-row { grid-template-columns: minmax(0, 1fr) minmax(0, 1fr); } .commercial-quota-row > span:first-child { grid-column: 1 / -1; } }
</style>
