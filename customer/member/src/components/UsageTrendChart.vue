<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { AFloatingPanel } from '@aster/ui'
import { formatTokens } from '@aster/sdk'
import { locale, t } from '../i18n'

export type UsageTrendPoint = {
  date: string
  request_count: number
  raw_tokens: number
  billed_tokens: number
  uncached_input_tokens: number
  cached_input_tokens: number
  cache_write_tokens: number
  output_tokens: number
}

const props = withDefaults(defineProps<{ items: UsageTrendPoint[]; previousItems?: UsageTrendPoint[]; metric: 'raw' | 'billed' }>(), { previousItems: () => [] })
const activeIndex = ref<number | null>(null)
const activeAnchor = ref<HTMLElement | null>(null)
const points = computed(() => props.items.map(item => {
  const input = Number(item.uncached_input_tokens || 0)
  const cacheRead = Number(item.cached_input_tokens || 0)
  const cacheWrite = Number(item.cache_write_tokens || 0)
  const output = Number(item.output_tokens || 0)
  const total = props.metric === 'raw' ? Number(item.raw_tokens || input + cacheRead + cacheWrite + output) : Number(item.billed_tokens || 0)
  return { ...item, input, cacheRead, cacheWrite, output, total }
}))
const previousTotals = computed(() => props.previousItems.map(item => props.metric === 'raw' ? Number(item.raw_tokens || 0) : Number(item.billed_tokens || 0)))
const hasPreviousUsage = computed(() => previousTotals.value.some(value => value > 0))
const maximum = computed(() => Math.max(...points.value.map(item => item.total), ...previousTotals.value, 1))
const ticks = computed(() => [1, .75, .5, .25, 0].map(ratio => Math.round(maximum.value * ratio)))
const comparisonPoints = computed(() => {
  const count = previousTotals.value.length
  if (!count || !hasPreviousUsage.value) return ''
  return previousTotals.value.map((value, index) => {
    const x = count === 1 ? 500 : (index + .5) / count * 1000
    const y = 8 + (1 - value / maximum.value) * 184
    return `${x},${y}`
  }).join(' ')
})
const activePoint = computed(() => activeIndex.value === null ? null : points.value[activeIndex.value] || null)
function shortNumber(value: number) {
  return new Intl.NumberFormat(locale.value, { notation: 'compact', maximumFractionDigits: 1 }).format(value)
}
function dateLabel(value: string) { return value.slice(5).replace('-', '/') }
function showDateLabel(index: number) {
  const count = points.value.length
  if (count <= 14) return true
  const step = count <= 31 ? 3 : Math.ceil(count / 10)
  return index === 0 || index === count - 1 || index % step === 0
}
const barGap = computed(() => points.value.length <= 7 ? 12 : points.value.length <= 14 ? 7 : points.value.length <= 31 ? 3 : 1)
function activate(index: number, event: Event) { activeIndex.value = index; activeAnchor.value = event.currentTarget as HTMLElement }
function deactivate(event: Event) {
  if (event.type === 'mouseleave' && document.activeElement === event.currentTarget) return
  activeIndex.value = null
  activeAnchor.value = null
}
watch(() => props.items, () => { activeIndex.value = null; activeAnchor.value = null })
</script>

<template>
  <div class="trend-figure">
    <div v-if="points.length" class="chart-body">
      <div class="y-axis" aria-hidden="true"><span v-for="tick in ticks" :key="tick">{{ shortNumber(tick) }}</span></div>
      <div class="plot-area">
        <div class="grid-lines" aria-hidden="true"><i v-for="tick in ticks" :key="tick" /></div>
        <svg v-if="comparisonPoints" class="comparison-line" viewBox="0 0 1000 212" preserveAspectRatio="none" aria-hidden="true"><polyline :points="comparisonPoints" /></svg>
        <div class="bars" role="list" :aria-label="t('usageTrend')" :style="{ gridTemplateColumns:`repeat(${points.length},minmax(0,1fr))`, gap:`${barGap}px` }">
          <button v-for="(item,index) in points" :key="`${item.date}-${index}`" type="button" class="bar-item"
            :class="{ active:activeIndex===index, 'edge-left':index===0, 'edge-right':index===points.length-1 }"
            role="listitem" @mouseenter="activate(index, $event)" @mouseleave="deactivate" @focus="activate(index, $event)" @blur="deactivate">
            <span class="bar-space">
              <span v-if="item.total" class="bar-stack" :style="{height:`${Math.max(3,item.total/maximum*100)}%`}">
                <template v-if="metric==='raw'">
                  <i class="segment input" :style="{height:`${item.total ? item.input/item.total*100 : 0}%`}" />
                  <i class="segment cache-read" :style="{height:`${item.total ? item.cacheRead/item.total*100 : 0}%`}" />
                  <i class="segment cache-write" :style="{height:`${item.total ? item.cacheWrite/item.total*100 : 0}%`}" />
                  <i class="segment output" :style="{height:`${item.total ? item.output/item.total*100 : 0}%`}" />
                </template>
                <i v-else class="segment billed" />
              </span>
            </span>
            <span class="date-label">{{ showDateLabel(index) ? dateLabel(item.date) : '' }}</span>
          </button>
        </div>
      </div>
    </div>
    <AFloatingPanel :open="!!activePoint" :anchor="activeAnchor" :width="205"><span v-if="activePoint" class="chart-tooltip"><strong>{{ activePoint.date }}</strong><span>{{ metric === 'raw' ? t('rawTokens') : t('settledTokens') }}<b>{{ formatTokens(activePoint.total, locale) }}</b></span><span>{{ t('uncached') }}<b>{{ formatTokens(activePoint.input, locale) }}</b></span><span>{{ t('cacheRead') }}<b>{{ formatTokens(activePoint.cacheRead, locale) }}</b></span><span>{{ t('cacheWrite') }}<b>{{ formatTokens(activePoint.cacheWrite, locale) }}</b></span><span>{{ t('output') }}<b>{{ formatTokens(activePoint.output, locale) }}</b></span><span>{{ t('requestCount') }}<b>{{ formatTokens(Number(activePoint.request_count || 0), locale) }}</b></span></span></AFloatingPanel>
    <div v-if="!points.length" class="chart-empty">{{ t('noUsageInPeriod') }}</div>
  </div>
</template>

<style scoped>
.trend-figure{height:100%;min-height:0;display:grid}.chart-body{--x-axis-height:22px;height:100%;min-height:245px;display:grid;grid-template-columns:48px minmax(0,1fr);gap:8px;padding-top:10px}.y-axis{display:flex;flex-direction:column;justify-content:space-between;padding:1px 0 var(--x-axis-height);color:var(--muted);font-size:var(--font-size-caption);text-align:right}.plot-area{position:relative;min-width:0}.grid-lines{position:absolute;inset:0 0 var(--x-axis-height);display:flex;flex-direction:column;justify-content:space-between;pointer-events:none}.grid-lines i{display:block;border-top:1px dashed var(--line)}.comparison-line{position:absolute;z-index:1;inset:0 0 var(--x-axis-height);width:100%;height:calc(100% - var(--x-axis-height));overflow:visible;pointer-events:none}.comparison-line polyline{fill:none;stroke:#6d87db;stroke-width:2;stroke-dasharray:6 6;vector-effect:non-scaling-stroke}.bars{position:absolute;z-index:2;inset:0;display:grid;align-items:stretch}.bar-item{position:relative;min-width:0;width:100%;display:grid;grid-template-rows:1fr var(--x-axis-height);padding:0;border:0;color:var(--muted);background:transparent;cursor:pointer;outline:none}.bar-space{display:flex;align-items:flex-end;justify-content:center;min-height:0}.bar-stack{width:min(100%,40px);min-height:3px;display:flex;flex-direction:column-reverse;border-radius:4px 4px 1px 1px;overflow:hidden;box-shadow:0 0 0 1px color-mix(in srgb,var(--surface) 45%,transparent)}.bar-item:hover .bar-stack,.bar-item:focus-visible .bar-stack,.bar-item.active .bar-stack{filter:saturate(1.15)}.segment{display:block;width:100%;min-height:1px}.segment.input{background:#2f6df6}.segment.cache-read{background:#1faeb0}.segment.cache-write{background:#ef7722}.segment.output{background:#35a853}.segment.billed{height:100%;background:#4f6ef7}.date-label{overflow:visible;font-size:var(--font-size-caption);text-align:center;white-space:nowrap}.bar-item.edge-left .date-label{text-align:left}.bar-item.edge-right .date-label{text-align:right}.bar-item:focus-visible .date-label{color:var(--accent);font-weight:700}.chart-tooltip{display:grid;gap:5px;font-size:var(--font-size-caption);text-align:left}.chart-tooltip>strong{margin-bottom:2px;font-size:var(--font-size-caption)}.chart-tooltip>span{display:flex;justify-content:space-between;gap:12px;color:#c9cfdd}.chart-tooltip b{color:#fff}.chart-empty{min-height:230px;display:grid;place-items:center;color:var(--muted);font-size:var(--font-size-body)}@media(max-width:680px){.chart-body{height:230px;grid-template-columns:34px 1fr}.date-label{font-size:var(--font-size-caption)}}
</style>
