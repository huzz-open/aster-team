<script setup lang="ts">
import type { PublicCatalog } from '../api/client'
import { CAPABILITIES, FEATURE_SETS, effectiveFeatures, QUOTAS } from '../api/generated/product-capabilities'
import { formatAmount } from '../commercial/plan-form'
defineProps<{ catalog: PublicCatalog }>()
</script>

<template>
  <div class="catalog-summary">
    <p>目标环境 {{ catalog.environment === 'local' ? '本地验证' : '生产' }} · {{ catalog.plans.length }} 个公开套餐</p>
    <p v-if="!catalog.plans.length">此目录不展示套餐，官网应显示联系入口。</p>
    <article v-for="(plan, index) in catalog.plans" :key="plan.plan_id">
      <h3>{{ index + 1 }} · {{ plan.name }}</h3><p>{{ plan.description }}</p>
      <dl>
        <dt>套餐版本</dt><dd>{{ plan.plan_id }} · v{{ plan.version }}</dd>
        <dt>授权版本</dt><dd>{{ plan.edition }}</dd>
        <dt>最低客户版本</dt><dd>{{ plan.minimum_version }}</dd>
        <dt>支持条款</dt><dd>{{ plan.support_terms_version }}</dd>
        <dt>功能集合</dt><dd>{{ (plan.entitlements.feature_sets ?? []).map(id => FEATURE_SETS.find(item => item.id === id)?.label ?? id).join(' · ') || '仅逐项授权' }}</dd>
        <dt>授权功能</dt><dd>{{ effectiveFeatures(plan.entitlements).map(id => CAPABILITIES.find(item => item.id === id)?.label ?? id).join(' · ') || '无授权功能' }}</dd>
        <template v-for="quota in plan.entitlements.quotas" :key="quota.id"><dt>{{ QUOTAS.find(item => item.id === quota.id)?.label ?? quota.id }}</dt><dd>{{ quota.limit.mode === 'unlimited' ? '不设商业上限' : quota.limit.value === 0 ? '禁止使用' : quota.limit.value }}</dd></template>
        <template v-if="plan.offer.kind === 'fixed_price'">
          <dt>年度价格</dt><dd>{{ formatAmount(plan.offer.annual_amount_minor, plan.offer.currency) }}</dd>
          <dt>税费</dt><dd>{{ plan.offer.tax_mode === 'inclusive' ? '金额含税' : '无税费' }}</dd>
          <dt>合同日历</dt><dd>{{ plan.offer.term_timezone === 'UTC' ? 'UTC' : '北京时间' }} · 自然年</dd>
          <template v-for="term in plan.offer.terms" :key="term.years"><dt>{{ term.years }} 年总价</dt><dd>{{ formatAmount(term.total_amount_minor, plan.offer.currency) }} · {{ term.discount_basis_points / 100 }}%</dd></template>
        </template>
        <template v-else-if="plan.offer.kind === 'free'"><dt>免费有效期</dt><dd>{{ plan.offer.expiry.mode === 'none' ? '不设到期' : plan.offer.expiry.expires_at }}</dd></template>
        <template v-else><dt>报价方式</dt><dd>联系报价</dd></template>
      </dl>
    </article>
  </div>
</template>

<style scoped>
.catalog-summary p { white-space: pre-wrap; overflow-wrap: anywhere; line-height: 1.6; }
.catalog-summary article { padding: 16px 0; border-top: 1px solid var(--line); }
.catalog-summary h3 { margin: 0 0 12px; font-size:var(--font-size-body); overflow-wrap: anywhere; }
.catalog-summary dl { display: grid; grid-template-columns: minmax(92px, 1fr) minmax(0, 2fr); gap: 12px 16px; margin: 16px 0; font-size:var(--font-size-body); }
.catalog-summary dt { color: var(--muted); }
.catalog-summary dd { margin: 0; overflow-wrap: anywhere; }
</style>
