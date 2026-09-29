<script setup lang="ts">
import type { CommercialPlanDefinition } from '../api/client'
import { CAPABILITIES, FEATURE_SETS, effectiveFeatures, QUOTAS } from '../api/generated/product-capabilities'
import { formatAmount, termAmount } from '../commercial/plan-form'
defineProps<{ definition: CommercialPlanDefinition }>()
</script>

<template>
  <div class="commercial-summary">
    <p v-if="definition.description">{{ definition.description }}</p>
    <dl>
      <dt>授权版本</dt><dd>{{ definition.edition }}</dd>
      <dt>最低客户版本</dt><dd>{{ definition.minimum_version }}</dd>
      <dt>支持条款</dt><dd>{{ definition.support_terms_version }}</dd>
      <dt>允许换机</dt><dd>{{ definition.transfer_limit }} 次</dd>
      <dt>功能集合</dt><dd>{{ (definition.entitlements.feature_sets ?? []).map(id => FEATURE_SETS.find(item => item.id === id)?.label ?? id).join(' · ') || '仅逐项授权' }}</dd>
        <dt>授权功能</dt><dd>{{ effectiveFeatures(definition.entitlements).map(id => CAPABILITIES.find(item => item.id === id)?.label ?? id).join(' · ') || '无授权功能' }}</dd>
      <template v-for="quota in definition.entitlements.quotas" :key="quota.id">
        <dt>{{ QUOTAS.find(item => item.id === quota.id)?.label ?? quota.id }}</dt><dd>{{ quota.limit.mode === 'unlimited' ? '不设商业上限' : quota.limit.value === 0 ? '禁止使用' : quota.limit.value }}</dd>
      </template>
      <template v-if="definition.offer.kind === 'annual'">
        <dt>年度价格</dt><dd>{{ formatAmount(definition.offer.annual_amount_minor, definition.offer.currency) }}</dd>
        <dt>税费</dt><dd>{{ definition.offer.tax_mode === 'inclusive' ? '金额含税' : '无税费' }}</dd>
        <dt>合同日历</dt><dd>{{ definition.offer.term_timezone === 'UTC' ? 'UTC' : '北京时间' }} · 自然年</dd>
        <template v-for="term in definition.offer.terms" :key="term.years"><dt>{{ term.years }} 年总价</dt><dd>{{ formatAmount(termAmount(definition.offer.annual_amount_minor, term.years, term.discount_basis_points), definition.offer.currency) }} · {{ term.discount_basis_points / 100 }}%</dd></template>
      </template>
      <template v-else-if="definition.offer.kind === 'free'"><dt>免费有效期</dt><dd>{{ definition.offer.expiry.mode === 'none' ? '不设到期' : definition.offer.expiry.expires_at }}</dd></template>
      <template v-else><dt>报价方式</dt><dd>联系报价</dd></template>
    </dl>
  </div>
</template>

<style scoped>
.commercial-summary p { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.6; }
.commercial-summary dl { display: grid; grid-template-columns: minmax(90px, 1fr) minmax(0, 2fr); gap: 12px 16px; margin: 16px 0; font-size:var(--font-size-body); }
.commercial-summary dt { color: var(--muted); }
.commercial-summary dd { margin: 0; overflow-wrap: anywhere; }
</style>
