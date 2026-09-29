<script setup lang="ts">
import { computed, ref } from 'vue'
import { formatTokens } from '@aster/sdk'
import { locale, t } from '../i18n'

const props = defineProps<{ input: number; cacheRead: number; cacheWrite: number; output: number }>()
const active = ref<number | null>(null)
const segments = computed(() => {
  const values = [
    { label: t('uncached'), value: Number(props.input || 0), color: '#2f6df6' },
    { label: t('cacheRead'), value: Number(props.cacheRead || 0), color: '#1faeb0' },
    { label: t('cacheWrite'), value: Number(props.cacheWrite || 0), color: '#ef7722' },
    { label: t('output'), value: Number(props.output || 0), color: '#35a853' },
  ]
  const total = values.reduce((sum, item) => sum + item.value, 0)
  let offset = 0
  return values.map(item => {
    const percent = total ? item.value / total * 100 : 0
    const result = { ...item, percent, offset }
    offset += percent
    return result
  })
})
const total = computed(() => segments.value.reduce((sum, item) => sum + item.value, 0))
const selected = computed(() => active.value === null ? null : segments.value[active.value])
function percent(value: number) { return `${new Intl.NumberFormat(locale.value, { maximumFractionDigits: 2 }).format(value)}%` }
function centerNumber(value: number) {
  return Math.abs(value) >= 1_000_000
    ? new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }).format(value)
    : formatTokens(value, locale.value)
}
</script>

<template>
  <div class="donut-layout">
    <div class="donut-wrap">
      <svg viewBox="0 0 160 160" role="img" :aria-label="t('tokenComposition')">
        <circle class="donut-track" cx="80" cy="80" r="55" pathLength="100" />
        <circle v-for="(item,index) in segments" :key="item.label" class="donut-segment" :class="{active:active===index,dimmed:active!==null&&active!==index}"
          cx="80" cy="80" r="55" pathLength="100" :stroke="item.color" :stroke-dasharray="`${item.percent} ${100-item.percent}`"
          :stroke-dashoffset="-item.offset" tabindex="0" role="button" :aria-label="`${item.label} ${formatTokens(item.value, locale)}`"
          @mouseenter="active=index" @mouseleave="active=null" @focus="active=index" @blur="active=null" />
      </svg>
      <div class="donut-center" aria-live="polite"><strong :title="formatTokens(selected?.value ?? total, locale)">{{ centerNumber(selected?.value ?? total) }}</strong><span v-if="selected">{{ selected.label }}</span></div>
    </div>
    <div class="donut-legend">
      <button v-for="(item,index) in segments" :key="item.label" type="button" :class="{active:active===index}"
        @mouseenter="active=index" @mouseleave="active=null" @focus="active=index" @blur="active=null">
        <i :style="{background:item.color}" /><span>{{ item.label }}</span><strong>{{ formatTokens(item.value, locale) }}</strong><small>{{ percent(item.percent) }}</small>
      </button>
    </div>
  </div>
</template>

<style scoped>
.donut-layout{height:100%;min-height:0;display:grid;grid-template-columns:minmax(170px,.9fr) minmax(190px,1.1fr);align-items:center;gap:16px}.donut-wrap{position:relative;width:min(210px,88%);aspect-ratio:1;margin:auto}.donut-wrap svg{width:100%;height:100%;overflow:visible;transform:rotate(-90deg)}.donut-track,.donut-segment{fill:none;stroke-width:24}.donut-track{stroke:var(--surface-3)}.donut-segment{cursor:pointer;outline:none;transition:opacity .16s,stroke-width .16s}.donut-segment.active,.donut-segment:focus-visible{stroke-width:27}.donut-segment.dimmed{opacity:.38}.donut-center{position:absolute;inset:30%;display:grid;place-content:center;text-align:center;pointer-events:none}.donut-center strong{font-size:var(--font-size-display);letter-spacing:-.03em}.donut-center span{margin-top:4px;color:var(--muted);font-size:var(--font-size-caption)}.donut-legend{display:grid;align-content:center}.donut-legend button{display:grid;grid-template-columns:auto minmax(72px,1fr) auto 54px;align-items:center;gap:9px;width:100%;min-height:40px;padding:8px 5px;border:0;border-bottom:1px solid var(--line);border-radius:0;color:var(--text-soft);background:transparent;font-size:var(--font-size-body);text-align:left}.donut-legend button:last-child{border-bottom:0}.donut-legend button:hover,.donut-legend button:focus-visible,.donut-legend button.active{color:var(--text);background:var(--surface-2);outline:none}.donut-legend i{width:8px;height:8px;border-radius:50%}.donut-legend strong{font-size:var(--font-size-caption)}.donut-legend small{color:var(--muted);font-size:var(--font-size-caption);text-align:right}@media(max-width:960px){.donut-layout{grid-template-columns:minmax(170px,.7fr) 1fr}.donut-wrap{width:min(190px,85%)}}@media(max-width:560px){.donut-layout{grid-template-columns:1fr}.donut-wrap{width:170px}}
</style>
