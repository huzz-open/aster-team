<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { formatMoney, formatTokens, request, sumMoneyAmounts, type AdminBillingOverview } from '@aster/sdk'
import { ADatePeriodRange, AIcon, AInfoTip, ALoadingState, useToast } from '@aster/ui'
import { locale } from '../i18n'

type Period = '7d' | '14d' | '30d' | '90d' | 'custom'

type UsageMetrics = {
  request_count: number
  raw_tokens: number
  billed_tokens: number
  uncovered_tokens: number
  uncached_input_tokens: number
  cached_input_tokens: number
  cache_write_tokens: number
  output_tokens: number
}

type TrendPoint = {
  date: string
  current: UsageMetrics
  previous_date: string
  previous: UsageMetrics
}

type UsageRanking = UsageMetrics & {
  user_id?: string
  display_name?: string
  email?: string
  model?: string
}

type UsageDimension = UsageMetrics & {
  value: string
}

type Overview = {
  period: Period
  days: number
  current: UsageMetrics
  previous: UsageMetrics
  trend: TrendPoint[]
  top_members: UsageRanking[]
  model_usage: UsageRanking[]
  processing_tiers: UsageDimension[]
  reasoning_efforts: UsageDimension[]
  granted_tokens: number
  used_tokens: number
  balance_tokens: number
  licensed_seats?: number
  period_active_users: number
  inactive_period_users: number
  active_keys: number
  users: number
  active_users: number
  enabled_runners: number
  online_runners: number
  upstream_accounts: number
  active_upstream_accounts: number
  models: number
  enabled_models: number
}

const now = new Date()
const localDate = (value: Date) => `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
const today = localDate(now)
const period = ref<Period>('14d')
const fromDate = ref(localDate(new Date(now.getFullYear(), now.getMonth(), now.getDate() - 13)))
const toDate = ref(today)
const data = ref<Overview | null>(null)
const billing = ref<AdminBillingOverview | null>(null)
const trendMode = ref<'cost' | 'tokens'>('cost')
const loading = ref(true)
const hoveredTrendIndex = ref<number | null>(null)
const trendChartSvg = ref<SVGSVGElement | null>(null)
const trendChartWidth = ref(820)
const trendChartHeight = ref(252)
const activeCompositionKey = ref<string | null>(null)
const selectedMemberId = ref<string | null>(null)
const selectedModelName = ref<string | null>(null)
const toast = useToast()
let loadRevision = 0
let trendChartResizeObserver: ResizeObserver | null = null

const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const periods = computed(() => [
  { value: '7d', label: tx('近 7 日', '7 days') },
  { value: '14d', label: tx('近 14 日', '14 days') },
  { value: '30d', label: tx('近 30 日', '30 days') },
  { value: '90d', label: tx('近 90 日', '90 days') },
])
const number = (value: number) => formatTokens(Number(value || 0), locale.value)
const compact = (value: number) => new Intl.NumberFormat(locale.value, {
  notation: 'compact', maximumFractionDigits: 2,
}).format(Number(value || 0))
const compositionCenterValue = computed(() => compact(selectedComposition.value?.value ?? composition.value.total))
const compositionCenterDensity = computed(() => {
  const length = Array.from(compositionCenterValue.value).length
  return { 'is-dense': length >= 7, 'is-very-dense': length >= 10 }
})
const ratio = (value: number, total: number) => total > 0 ? Math.min(100, Math.max(0, value * 100 / total)) : 0
const percent = (value: number) => `${value.toFixed(1)}%`
const chartDate = (value: string) => {
  const [yearText = '1970', monthText = '1', dayText = '1'] = value.split('-')
  const year = Number(yearText)
  const month = Number(monthText)
  const day = Number(dayText)
  return new Intl.DateTimeFormat(locale.value, {
    month: 'numeric',
    day: 'numeric',
    weekday: 'short',
  }).format(new Date(year, month - 1, day))
}

function niceMaximum(value: number) {
  if (value <= 0) return 1
  const magnitude = 10 ** Math.floor(Math.log10(value))
  const normalized = value / magnitude
  const ceiling = [1, 2, 2.5, 5, 10].find(candidate => normalized <= candidate) || 10
  return ceiling * magnitude
}

function chartScale(maximum: number) {
  if (locale.value === 'en-US') {
    if (maximum >= 1_000_000) return { divisor: 1_000_000, label: 'Token (M)' }
    if (maximum >= 1_000) return { divisor: 1_000, label: 'Token (K)' }
    return { divisor: 1, label: 'Token' }
  }
  if (maximum >= 10_000) return { divisor: 10_000, label: 'Token（万）' }
  return { divisor: 1, label: 'Token' }
}

function axisNumber(value: number) {
  return new Intl.NumberFormat(locale.value, { maximumFractionDigits: 1 }).format(value)
}

async function load() {
  if (period.value === 'custom' && (!fromDate.value || !toDate.value)) return
  const revision = ++loadRevision
  loading.value = true
  try {
    const query = new URLSearchParams()
    if (period.value === 'custom') {
      query.set('from', fromDate.value)
      query.set('to', toDate.value)
    } else query.set('period', period.value)
    const result = await request<Overview>(`/api/admin/overview?${query}`)
    const money = await request<AdminBillingOverview>('/api/admin/billing/overview').catch(() => null)
    if (revision === loadRevision) {
      data.value = result
      billing.value = money
      if (!money?.configured) trendMode.value = 'tokens'
    }
  } catch (value) {
    if (revision === loadRevision) toast.error(value instanceof Error ? value.message : tx('运行概览加载失败', 'Could not load operations overview'))
  } finally {
    if (revision === loadRevision) loading.value = false
  }
}

function comparison(current: number, previous: number, cost = false) {
  if (!previous) return current ? cost ? tx('上期无费用', 'No prior cost') : tx('上期无使用', 'No prior usage') : tx('与上期持平', 'No change')
  const change = (current - previous) * 100 / previous
  const sign = change > 0 ? '+' : ''
  return `${tx('较上期', 'vs previous')} ${sign}${change.toFixed(1)}%`
}

function comparisonTone(current: number, previous: number) {
  return current > previous ? 'positive' : current < previous ? 'negative' : 'neutral'
}

const configuredBilling = computed(() => billing.value?.configured ? billing.value : null)
const periodDates = computed(() => {
  const trend = data.value?.trend ?? []
  return { from: trend[0]?.date ?? '', to: trend.at(-1)?.date ?? '' }
})
const chargedEntries = computed(() => configuredBilling.value?.entries.filter(entry => entry.kind === 'charge') ?? [])
const costByDate = computed(() => {
  const amounts = new Map<string, string[]>()
  for (const entry of chargedEntries.value) {
    const date = entry.created_at.slice(0, 10)
    const values = amounts.get(date) ?? []
    values.push(entry.amount.startsWith('-') ? entry.amount.slice(1) : entry.amount)
    amounts.set(date, values)
  }
  return new Map([...amounts].map(([date, values]) => [date, sumMoneyAmounts(values)]))
})
const periodCost = computed(() => sumMoneyAmounts((data.value?.trend ?? []).map(row => costByDate.value.get(row.date) ?? '0')))
const previousCost = computed(() => sumMoneyAmounts((data.value?.trend ?? []).map(row => costByDate.value.get(row.previous_date) ?? '0')))
const costUtilization = computed(() => ratio(Number(configuredBilling.value?.debited ?? 0), Number(configuredBilling.value?.credited ?? 0)))
const money = (amount: string) => {
  const [whole = '0', fraction = ''] = amount.split('.')
  return formatMoney(`${whole}.${fraction.padEnd(2, '0')}`, configuredBilling.value?.currency)
}
const trendValue = (row: TrendPoint, which: 'current' | 'previous') => trendMode.value === 'cost'
  ? Number(costByDate.value.get(which === 'current' ? row.date : row.previous_date) ?? 0)
  : row[which].raw_tokens
const trendLabel = (row: TrendPoint, which: 'current' | 'previous') => trendMode.value === 'cost'
  ? money(costByDate.value.get(which === 'current' ? row.date : row.previous_date) ?? '0')
  : `${number(row[which].raw_tokens)} Token`
const activeMemberRate = computed(() => ratio(data.value?.period_active_users || 0, data.value?.users || 0))
const cacheHitRate = computed(() => {
  const current = data.value?.current
  return ratio(current?.cached_input_tokens || 0, (current?.uncached_input_tokens || 0) + (current?.cached_input_tokens || 0))
})
const cacheShare = computed(() => {
  const current = data.value?.current
  return ratio((current?.cached_input_tokens || 0) + (current?.cache_write_tokens || 0), current?.raw_tokens || 0)
})
const accountSharing = computed(() => {
  const accounts = data.value?.active_upstream_accounts || 0
  return accounts ? (data.value?.period_active_users || 0) / accounts : 0
})
const accountSharingDetail = computed(() => {
  const active = data.value?.period_active_users || 0
  const accounts = data.value?.active_upstream_accounts || 0
  return tx(`活跃 ${active} 人 · 启用 ${accounts} 个`, `${active} active · ${accounts} enabled`)
})

function toggleMember(id?: string) {
  if (!id) return
  selectedMemberId.value = selectedMemberId.value === id ? null : id
}

function toggleModel(name?: string) {
  if (!name) return
  selectedModelName.value = selectedModelName.value === name ? null : name
}

function sparkline(metric: 'cost' | 'request_count') {
  const values = (data.value?.trend || []).map(item => metric === 'cost'
    ? Number(costByDate.value.get(item.date) ?? 0)
    : Number(item.current.request_count || 0))
  const maximum = Math.max(1, ...values)
  const width = 100
  const height = 22
  return values.map((value, index) => {
    const x = values.length <= 1 ? width / 2 : index * width / (values.length - 1)
    const y = height - value * (height - 3) / maximum
    return `${x.toFixed(1)},${y.toFixed(1)}`
  }).join(' ')
}

const chart = computed(() => {
  const rows = data.value?.trend || []
  const width = trendChartWidth.value
  const height = trendChartHeight.value
  const left = 52
  const right = 18
  const top = 18
  const bottom = 36
  const plotWidth = width - left - right
  const plotHeight = height - top - bottom
  const maximum = niceMaximum(Math.max(1, ...rows.flatMap(row => [trendValue(row, 'current'), trendValue(row, 'previous')])))
  const scale = trendMode.value === 'cost'
    ? { divisor: 1, label: configuredBilling.value?.currency ?? tx('费用', 'Cost') }
    : chartScale(maximum)
  const x = (index: number) => rows.length <= 1 ? left + plotWidth / 2 : left + index * plotWidth / (rows.length - 1)
  const y = (value: number) => top + plotHeight - value * plotHeight / maximum
  const points = (which: 'current' | 'previous') => rows.map((row, index) => `${x(index).toFixed(1)},${y(trendValue(row, which)).toFixed(1)}`).join(' ')
  const currentPoints = points('current')
  const area = rows.length ? `${left},${top + plotHeight} ${currentPoints} ${x(rows.length - 1)},${top + plotHeight}` : ''
  const grid = Array.from({ length: 5 }, (_, index) => {
    const value = maximum * (4 - index) / 4
    return { y: top + index * plotHeight / 4, value }
  })
  const labelStep = Math.max(1, Math.ceil(rows.length / 14))
  const labels = rows.map((row, index) => ({ row, index, x: x(index) })).filter(({ index }) => index % labelStep === 0 || index === rows.length - 1)
  const hitAreas = rows.map((row, index) => {
    const currentX = x(index)
    const previousX = index > 0 ? x(index - 1) : left
    const nextX = index < rows.length - 1 ? x(index + 1) : width - right
    const start = index === 0 ? left : (previousX + currentX) / 2
    const end = index === rows.length - 1 ? width - right : (currentX + nextX) / 2
    return { row, index, x: start, width: Math.max(1, end - start) }
  })
  return { width, height, left, right, top, bottom, plotHeight, currentPoints, previousPoints: points('previous'), area, grid, labels, rows, hitAreas, scale, x, y }
})

const hoveredTrend = computed(() => {
  const index = hoveredTrendIndex.value
  if (index === null) return null
  const row = chart.value.rows[index]
  if (!row) return null
  const x = chart.value.x(index)
  const peak = Math.max(trendValue(row, 'current'), trendValue(row, 'previous'))
  const y = chart.value.y(peak)
  return {
    row,
    x,
    y,
    style: {
      left: `${x * 100 / chart.value.width}%`,
      top: `${Math.max(12, y * 100 / chart.value.height)}%`,
    },
    alignment: index <= 2 ? 'align-start' : index >= chart.value.rows.length - 3 ? 'align-end' : 'align-center',
    placement: y > chart.value.height * 0.58 ? 'place-above' : 'place-below',
  }
})

const composition = computed(() => {
  const current = data.value?.current
  const rows = [
    { key: 'input', label: tx('输入', 'Input'), value: current?.uncached_input_tokens || 0, color: '#4f63f5' },
    { key: 'cache-read', label: tx('缓存命中', 'Cache hit'), value: current?.cached_input_tokens || 0, color: '#20b8b0' },
    { key: 'cache-write', label: tx('缓存写入', 'Cache write'), value: current?.cache_write_tokens || 0, color: '#8a68ee' },
    { key: 'output', label: tx('输出', 'Output'), value: current?.output_tokens || 0, color: '#f2bd3d' },
  ]
  const total = rows.reduce((sum, row) => sum + row.value, 0)
  let start = 0
  const segments = rows.map(row => {
    const share = total ? row.value * 100 / total : 0
    const offset = -start
    start += share
    return { ...row, share, offset }
  })
  return { rows: segments, total }
})

const selectedComposition = computed(() => composition.value.rows.find(row => row.key === activeCompositionKey.value) || null)

const currentCharges = computed(() => chargedEntries.value.filter(entry => {
  const date = entry.created_at.slice(0, 10)
  return date >= periodDates.value.from && date <= periodDates.value.to
}))
const memberCosts = computed(() => {
  const totals = new Map<string, string[]>()
  for (const entry of currentCharges.value) {
    const values = totals.get(entry.identity_id) ?? []
    values.push(entry.amount.startsWith('-') ? entry.amount.slice(1) : entry.amount)
    totals.set(entry.identity_id, values)
  }
  return (configuredBilling.value?.members ?? []).map(member => ({
    ...member, cost: sumMoneyAmounts(totals.get(member.identity_id) ?? []),
    requests: totals.get(member.identity_id)?.length ?? 0,
  })).filter(member => member.requests > 0).sort((a, b) => Number(b.cost) - Number(a.cost)).slice(0, 5)
})
const topMemberMaximum = computed(() => Math.max(1, ...memberCosts.value.map(item => Number(item.cost))))
const modelCosts = computed(() => {
  const totals = new Map<string, string[]>()
  for (const entry of currentCharges.value) {
    const model = typeof entry.details.public_model === 'string' ? entry.details.public_model : tx('未知模型', 'Unknown model')
    const values = totals.get(model) ?? []
    values.push(entry.amount.startsWith('-') ? entry.amount.slice(1) : entry.amount)
    totals.set(model, values)
  }
  return [...totals].map(([model, values]) => ({ model, cost: sumMoneyAmounts(values), requests: values.length }))
    .sort((a, b) => Number(b.cost) - Number(a.cost))
})
const modelRows = computed(() => {
  const source = modelCosts.value
  if (source.length <= 4) return source
  const other = source.slice(3)
  return [...source.slice(0, 3), {
    model: tx('其他', 'Other'), cost: sumMoneyAmounts(other.map(item => item.cost)),
    requests: other.reduce((count, item) => count + item.requests, 0),
  }]
})
function executionValueLabel(value?: string) {
  if (!value || value === 'model_default') return tx('模型默认', 'Model default')
  const labels: Record<string, [string, string]> = {
    auto: ['自动', 'Auto'], standard: ['标准', 'Standard'], flex: ['灵活', 'Flex'], fast: ['快速', 'Fast'], ultrafast: ['极速', 'Ultra fast'],
    none: ['无推理', 'None'], low: ['低', 'Low'], medium: ['中', 'Medium'], high: ['高', 'High'], xhigh: ['极高', 'Extra high'], max: ['最大', 'Max'],
  }
  const label = labels[value]
  return label ? tx(label[0], label[1]) : value
}
function primaryDimension(items: UsageDimension[]) {
  const item = [...items].sort((left, right) => right.raw_tokens - left.raw_tokens)[0]
  const total = items.reduce((sum, row) => sum + row.raw_tokens, 0)
  return item ? { label: executionValueLabel(item.value), percent: ratio(item.raw_tokens, total) } : null
}
const primaryProcessingTier = computed(() => primaryDimension(data.value?.processing_tiers || []))
const primaryReasoningEffort = computed(() => primaryDimension(data.value?.reasoning_efforts || []))

watch(trendChartSvg, (chartElement) => {
  trendChartResizeObserver?.disconnect()
  trendChartResizeObserver = null
  if (!chartElement) return
  const updateChartSize = (width: number, height: number) => {
    if (width > 0) trendChartWidth.value = Math.round(width)
    if (height > 0) trendChartHeight.value = Math.max(252, Math.round(height))
  }
  const bounds = chartElement.getBoundingClientRect()
  updateChartSize(bounds.width, bounds.height)

  if (typeof ResizeObserver !== 'undefined') {
    trendChartResizeObserver = new ResizeObserver(([entry]) => {
      if (entry) updateChartSize(entry.contentRect.width, entry.contentRect.height)
    })
    trendChartResizeObserver.observe(chartElement)
  }
}, { flush: 'post' })

onMounted(load)
onBeforeUnmount(() => trendChartResizeObserver?.disconnect())
</script>

<template>
  <div class="content overview-page">
    <header class="overview-head">
      <div><h1>{{ tx('运行概览', 'Operations overview') }}</h1></div>
      <div class="overview-actions">
        <ADatePeriodRange class="overview-period" v-model="period" v-model:from="fromDate" v-model:to="toDate" :options="periods" :label="tx('统计周期', 'Reporting period')" :max="today" :max-range-days="366" :start-label="tx('开始日期', 'Start date')" :end-label="tx('结束日期', 'End date')" :locale="locale" @change="load" />
        <button type="button" class="refresh-button" :disabled="loading" @click="load"><AIcon name="sync" :size="15" />{{ tx('刷新', 'Refresh') }}</button>
      </div>
    </header>

    <ALoadingState v-if="loading && !data" :label="tx('正在读取运行概览…', 'Loading operations overview…')" />
    <div v-else-if="data" class="dashboard-body" :class="{ refreshing: loading }">
      <section class="kpi-grid" :aria-label="tx('核心指标', 'Core metrics')">
        <article class="kpi-card tone-indigo">
          <span class="kpi-icon"><AIcon name="payment" /></span><span class="kpi-label">{{ tx('本期费用', 'Period cost') }} <AInfoTip :text="tx('平台费用按已确认用量和配置价格计算，仅供参考；实际结算以模型官方账单为准。', 'Platform costs are estimates based on confirmed usage and configured prices. The provider’s official bill is authoritative.')" /></span>
          <strong>{{ money(periodCost) }}</strong><small v-if="configuredBilling" :class="comparisonTone(Number(periodCost), Number(previousCost))">{{ comparison(Number(periodCost), Number(previousCost), true) }}</small><small v-else>{{ tx('请先配置费用结算', 'Configure billing first') }}</small>
          <svg class="sparkline" viewBox="0 0 100 24" preserveAspectRatio="none"><polyline :points="sparkline('cost')" /></svg>
        </article>
        <article class="kpi-card tone-blue">
          <span class="kpi-icon"><AIcon name="trend-up" /></span><span class="kpi-label">{{ tx('API 请求', 'API requests') }}</span>
          <strong>{{ number(data.current.request_count) }}</strong><small :class="comparisonTone(data.current.request_count, data.previous.request_count)">{{ comparison(data.current.request_count, data.previous.request_count) }}</small>
          <svg class="sparkline" viewBox="0 0 100 24" preserveAspectRatio="none"><polyline :points="sparkline('request_count')" /></svg>
        </article>
        <article class="kpi-card tone-teal">
          <span class="kpi-icon"><AIcon name="pie" /></span><span class="kpi-label">{{ tx('费用使用率', 'Budget utilization') }}</span>
          <strong>{{ configuredBilling ? percent(costUtilization) : '—' }}</strong><small>{{ tx('累计费用 / 累计发放', 'Total cost / credited') }}</small>
          <span class="metric-progress"><i :style="{ width: `${costUtilization}%` }" /></span>
        </article>
        <article class="kpi-card tone-green">
          <span class="kpi-icon"><AIcon name="users" /></span><span class="kpi-label">{{ tx('活跃成员率', 'Active member rate') }}</span>
          <strong>{{ percent(activeMemberRate) }}</strong><small>{{ data.period_active_users }} / {{ data.users }} {{ tx('人', 'members') }}</small>
          <span class="metric-progress"><i :style="{ width: `${activeMemberRate}%` }" /></span>
        </article>
        <article class="kpi-card tone-cyan">
          <span class="kpi-icon"><AIcon name="target" /></span><span class="kpi-label">{{ tx('缓存命中率', 'Cache hit rate') }}</span>
          <strong>{{ percent(cacheHitRate) }}</strong>
          <span class="metric-progress"><i :style="{ width: `${cacheHitRate}%` }" /></span>
        </article>
        <article class="kpi-card tone-violet">
          <span class="kpi-icon"><AIcon name="share" /></span><span class="kpi-label">{{ tx('账号共享效率', 'Account sharing') }}</span>
          <strong>{{ accountSharing.toFixed(1) }} <em>{{ tx('人 / 账号', 'members / account') }}</em></strong>
          <small>{{ accountSharingDetail }}</small>
          <span class="metric-progress"><i :style="{ width: `${ratio(data.active_upstream_accounts, data.upstream_accounts)}%` }" /></span>
        </article>
      </section>

      <section class="main-grid">
        <article class="dashboard-panel trend-panel">
          <header class="panel-head"><div><h2>{{ trendMode === 'cost' ? tx('费用趋势', 'Cost trend') : tx('Token 使用趋势', 'Token usage trend') }}</h2></div><div class="trend-controls"><div class="chart-modes" :aria-label="tx('趋势指标', 'Trend metric')"><button type="button" :class="{ active: trendMode === 'cost' }" :disabled="!configuredBilling" @click="trendMode = 'cost'">{{ tx('费用', 'Cost') }}</button><button type="button" :class="{ active: trendMode === 'tokens' }" @click="trendMode = 'tokens'">Token</button></div><div class="chart-legend"><span class="current">{{ tx('本期', 'Current') }}</span><span class="previous">{{ tx('上期', 'Previous') }}</span></div></div></header>
          <div class="trend-chart" @pointerleave="hoveredTrendIndex = null">
            <svg ref="trendChartSvg" :viewBox="`0 0 ${chart.width} ${chart.height}`" role="img" :aria-label="trendMode === 'cost' ? tx('费用趋势图', 'Cost trend chart') : tx('Token 使用趋势图', 'Token usage trend chart')">
              <defs><linearGradient id="overview-area" x1="0" y1="0" x2="0" y2="1"><stop offset="0" stop-color="#4f63f5" stop-opacity=".24"/><stop offset="1" stop-color="#4f63f5" stop-opacity=".02"/></linearGradient></defs>
              <text class="axis-unit" :x="chart.left" y="11">{{ chart.scale.label }}</text>
              <g v-for="line in chart.grid" :key="line.y"><line class="grid-line" :x1="chart.left" :x2="chart.width-chart.right" :y1="line.y" :y2="line.y"/><text class="axis-label" :x="chart.left-10" :y="line.y+4" text-anchor="end">{{ axisNumber(line.value / chart.scale.divisor) }}</text></g>
              <polygon v-if="chart.area" class="chart-area" :points="chart.area" />
              <polyline v-if="chart.previousPoints" class="previous-line" :points="chart.previousPoints" />
              <polyline v-if="chart.currentPoints" class="current-line" :points="chart.currentPoints" />
              <g v-for="(row, index) in chart.rows" :key="`point-${row.date}`">
                <circle class="chart-point previous" :cx="chart.x(index)" :cy="chart.y(trendValue(row, 'previous'))" r="2.7" />
                <circle class="chart-point current" :cx="chart.x(index)" :cy="chart.y(trendValue(row, 'current'))" r="3.1" />
              </g>
              <g v-if="hoveredTrend">
                <line class="hover-guide" :x1="hoveredTrend.x" :x2="hoveredTrend.x" :y1="chart.top" :y2="chart.top + chart.plotHeight" />
                <circle class="hover-point previous" :cx="hoveredTrend.x" :cy="chart.y(trendValue(hoveredTrend.row, 'previous'))" r="4" />
                <circle class="hover-point current" :cx="hoveredTrend.x" :cy="chart.y(trendValue(hoveredTrend.row, 'current'))" r="4.5" />
              </g>
              <g v-for="label in chart.labels" :key="label.row.date"><text class="axis-label" :x="label.x" :y="chart.height-10" text-anchor="middle">{{ label.row.date.slice(5).replace('-', '/') }}</text></g>
              <rect v-for="area in chart.hitAreas" :key="area.row.date" class="chart-hit-area" :x="area.x" :y="chart.top" :width="area.width" :height="chart.plotHeight" tabindex="0" :aria-label="`${area.row.date}, ${trendLabel(area.row, 'current')}`" @pointerenter="hoveredTrendIndex = area.index" @focus="hoveredTrendIndex = area.index" @blur="hoveredTrendIndex = null" />
            </svg>
            <Transition name="chart-tooltip">
              <div v-if="hoveredTrend" class="trend-tooltip" :class="[hoveredTrend.alignment, hoveredTrend.placement]" :style="hoveredTrend.style" role="status">
                <strong>{{ chartDate(hoveredTrend.row.date) }}</strong>
                <span><i class="current" />{{ tx('本期', 'Current') }}<b>{{ trendLabel(hoveredTrend.row, 'current') }}</b></span>
                <span><i class="previous" />{{ tx('上期', 'Previous') }}<b>{{ trendLabel(hoveredTrend.row, 'previous') }}</b></span>
                <span><i class="requests" />{{ tx('API 请求', 'API requests') }}<b>{{ number(hoveredTrend.row.current.request_count) }}</b></span>
              </div>
            </Transition>
          </div>
        </article>

        <article class="dashboard-panel composition-panel">
          <header class="panel-head"><div><h2>{{ tx('Token 构成', 'Token composition') }}</h2><p>{{ tx('输入、缓存与输出占比', 'Input, cache, and output share') }}</p></div></header>
          <div class="composition-body">
            <div class="donut" @pointerleave="activeCompositionKey = null">
              <svg viewBox="0 0 120 120" role="img" :aria-label="tx('Token 构成占比', 'Token composition share')">
                <circle class="donut-track" cx="60" cy="60" r="45" pathLength="100" />
                <circle v-for="item in composition.rows" :key="item.key" class="donut-segment" :class="{ active: activeCompositionKey === item.key, dimmed: activeCompositionKey && activeCompositionKey !== item.key }" cx="60" cy="60" r="45" pathLength="100" :stroke="item.color" :stroke-dasharray="`${item.share} ${100 - item.share}`" :stroke-dashoffset="item.offset" tabindex="0" @pointerenter="activeCompositionKey = item.key" @focus="activeCompositionKey = item.key" @blur="activeCompositionKey = null" />
              </svg>
              <div><strong :class="compositionCenterDensity" :title="number(selectedComposition?.value ?? composition.total)">{{ compositionCenterValue }}</strong><span>{{ selectedComposition?.label || 'Token' }}</span></div>
            </div>
            <div class="composition-legend">
              <button v-for="item in composition.rows" :key="item.key" type="button" :class="{ active: activeCompositionKey === item.key, dimmed: activeCompositionKey && activeCompositionKey !== item.key }" @pointerenter="activeCompositionKey = item.key" @pointerleave="activeCompositionKey = null" @focus="activeCompositionKey = item.key" @blur="activeCompositionKey = null">
                <span><i :style="{ background: item.color }" />{{ item.label }}</span>
                <b>{{ activeCompositionKey === item.key ? `${compact(item.value)} Token` : `${item.share.toFixed(0)}%` }}</b>
              </button>
            </div>
          </div>
          <div class="cache-summary"><AIcon name="cache" :size="20" /><span>{{ tx('缓存占比', 'Cache share') }}</span><strong>{{ cacheShare.toFixed(0) }}%</strong></div>
        </article>
      </section>

      <section class="bottom-grid">
        <article class="dashboard-panel ranking-panel">
          <header class="panel-head"><div><h2>{{ tx('成员费用 Top 5', 'Top 5 member costs') }}</h2></div><span>{{ configuredBilling?.currency ?? '—' }}</span></header>
          <ol v-if="memberCosts.length" class="ranking-list">
            <li v-for="(item, index) in memberCosts" :key="item.identity_id" :class="{ selected: selectedMemberId === item.identity_id }">
              <button type="button" class="ranking-row" :aria-expanded="selectedMemberId === item.identity_id" @click="toggleMember(item.identity_id)">
                <b>{{ index + 1 }}</b><span class="ranking-name" :title="item.email">{{ item.name || item.email }}</span><span class="ranking-bar"><i :style="{ width: `${ratio(Number(item.cost), topMemberMaximum)}%` }" /></span><strong :title="money(item.cost)">{{ money(item.cost) }}</strong><AIcon name="chevron" :size="14" />
              </button>
              <Transition name="ranking-detail"><div v-if="selectedMemberId === item.identity_id" class="ranking-detail"><div><span>{{ tx('API 请求', 'API requests') }}</span><strong>{{ number(item.requests) }}</strong></div><div><span>{{ tx('本期费用', 'Period cost') }}</span><strong>{{ money(item.cost) }}</strong></div></div></Transition>
            </li>
          </ol>
          <p v-else class="empty-copy">{{ tx('当前周期暂无成员费用', 'No member costs in this period') }}</p>
          <div class="idle-note"><AIcon name="users" :size="17" /><span>{{ data.inactive_period_users }} {{ tx(`名成员近 ${data.days} 日未使用`, `members had no usage in the last ${data.days} days`) }}</span></div>
        </article>

        <article class="dashboard-panel ranking-panel">
          <header class="panel-head"><div><h2>{{ tx('模型费用分布', 'Model cost distribution') }}</h2></div><span>{{ tx('费用占比', 'Cost share') }}</span></header>
          <div v-if="modelRows.length" class="model-list">
            <div v-for="item in modelRows" :key="item.model" class="model-item" :class="{ selected: selectedModelName === item.model }">
              <button type="button" class="model-row" :aria-expanded="selectedModelName === item.model" @click="toggleModel(item.model)">
                <span class="ranking-name" :title="item.model">{{ item.model }}</span><span class="ranking-bar"><i :style="{ width: `${ratio(Number(item.cost), Number(periodCost))}%` }" /></span><strong>{{ ratio(Number(item.cost), Number(periodCost)).toFixed(0) }}%</strong><AIcon name="chevron" :size="14" />
              </button>
              <Transition name="ranking-detail"><div v-if="selectedModelName === item.model" class="ranking-detail"><div><span>{{ tx('API 请求', 'API requests') }}</span><strong>{{ number(item.requests) }}</strong></div><div><span>{{ tx('本期费用', 'Period cost') }}</span><strong>{{ money(item.cost) }}</strong></div></div></Transition>
            </div>
          </div>
          <p v-else class="empty-copy">{{ tx('当前周期暂无模型费用', 'No model costs in this period') }}</p>
          <dl class="execution-summary">
            <div><dt><span class="inline-actions"><span>{{ tx('主要处理速度', 'Primary speed') }}</span><AInfoTip :text="tx('速度。选择 ChatGPT 在聊天、子智能体和压缩中的运行速度。', 'Speed. Choose how fast ChatGPT runs in chats, subagents, and compactions.')" /></span></dt><dd>{{ primaryProcessingTier ? `${primaryProcessingTier.label} · ${primaryProcessingTier.percent.toFixed(0)}%` : '—' }}</dd></div>
            <div><dt>{{ tx('主要推理强度', 'Primary effort') }}</dt><dd>{{ primaryReasoningEffort ? `${primaryReasoningEffort.label} · ${primaryReasoningEffort.percent.toFixed(0)}%` : '—' }}</dd></div>
          </dl>
        </article>

        <article class="dashboard-panel resources-panel">
          <header class="panel-head"><div><h2>{{ tx('运行资源', 'Runtime resources') }}</h2></div></header>
          <dl class="resource-list">
            <div><dt><AIcon name="server" :size="17" />Runner</dt><dd><i :class="{ healthy: data.online_runners === data.enabled_runners && data.enabled_runners > 0 }" />{{ data.online_runners }} / {{ data.enabled_runners }} {{ tx('在线', 'online') }}</dd></div>
            <div><dt><AIcon name="cloud" :size="17" />{{ tx('订阅/账号', 'Subscriptions & accounts') }}</dt><dd><i :class="{ healthy: data.active_upstream_accounts > 0 }" />{{ data.active_upstream_accounts }} / {{ data.upstream_accounts }} {{ tx('启用', 'active') }}</dd></div>
            <div><dt><AIcon name="model" :size="17" />{{ tx('模型', 'Models') }}</dt><dd><i :class="{ healthy: data.enabled_models > 0 }" />{{ data.enabled_models }} / {{ data.models }} {{ tx('开放', 'enabled') }}</dd></div>
            <div><dt><AIcon name="key" :size="17" />{{ tx('有效 API Key', 'Active API keys') }}</dt><dd>{{ data.active_keys }}</dd></div>
          </dl>
        </article>
      </section>
    </div>
  </div>
</template>

<style scoped>
.overview-page{max-width:var(--page-max-width);padding:var(--page-padding-top) var(--page-padding-inline) var(--page-padding-bottom)}
.overview-head{display:flex;align-items:flex-start;justify-content:space-between;gap:30px;margin-bottom:16px}
.overview-head h1{margin:0;font-size:var(--font-size-display);line-height:var(--page-title-line-height);letter-spacing:var(--page-title-letter-spacing)}
.overview-head p{margin:7px 0 0;color:var(--text-soft);font-size:var(--font-size-body)}
.overview-actions{display:flex;align-items:center;gap:14px}
.refresh-button{height:42px;display:inline-flex;align-items:center;gap:8px;padding:0 17px;border:1px solid var(--line);border-radius:10px;background:var(--surface);color:var(--text);font-size:var(--font-size-body);font-weight:750;cursor:pointer;box-shadow:var(--shadow-soft);transition:border-color .16s ease,color .16s ease,background .16s ease}
.refresh-button:hover{border-color:var(--accent);color:var(--accent);background:var(--accent-soft)}
.refresh-button:disabled{opacity:.62;cursor:wait}
.refresh-button:disabled :deep(.a-icon){animation:overview-spin .8s linear infinite}
.dashboard-body{container:overview-dashboard / inline-size;display:grid;gap:12px;transition:opacity .16s}
.dashboard-body.refreshing{opacity:.7}
.kpi-grid{display:grid;grid-template-columns:repeat(6,minmax(0,1fr));gap:10px}
.kpi-card{--metric-color:#4f63f5;position:relative;min-width:0;min-height:152px;padding:18px 17px 14px 62px;border:1px solid var(--line);border-radius:12px;background:var(--surface);box-shadow:var(--shadow-soft);overflow:hidden}
.kpi-card.tone-indigo{--metric-color:#5b50ee}
.kpi-card.tone-teal{--metric-color:#18aaa4}
.kpi-card.tone-green{--metric-color:#25a76f}
.kpi-card.tone-cyan{--metric-color:#258ddf}
.kpi-card.tone-violet{--metric-color:#7559e8}
.kpi-icon{position:absolute;left:17px;top:17px;width:36px;height:36px;display:grid;place-items:center;border-radius:11px;background:color-mix(in srgb,var(--metric-color) 11%,var(--surface));color:var(--metric-color)}
.kpi-icon :deep(.a-icon){width:21px;height:21px}
.kpi-label{display:block;overflow:hidden;color:var(--text);font-size:var(--font-size-body);font-weight:800;white-space:nowrap;text-overflow:ellipsis}
.kpi-card>strong{display:block;margin-top:12px;color:var(--text);font-size:var(--font-size-display);line-height:1;font-variant-numeric:tabular-nums;white-space:nowrap;letter-spacing:-.02em}
.kpi-card>strong em{font-size:var(--font-size-body);font-style:normal;font-weight:700;letter-spacing:0}
.kpi-card>small{display:block;min-height:18px;margin-top:8px;overflow:hidden;color:var(--muted);font-size:var(--font-size-body);white-space:nowrap;text-overflow:ellipsis}
.kpi-card>small.positive{color:var(--positive);font-weight:800}
.kpi-card>small.negative{color:var(--danger);font-weight:800}
.sparkline{position:absolute;left:17px;right:17px;bottom:9px;width:calc(100% - 34px);height:31px;overflow:visible}
.sparkline polyline{fill:none;stroke:var(--metric-color);stroke-width:2.2;stroke-linecap:round;stroke-linejoin:round;vector-effect:non-scaling-stroke}
.metric-progress{position:absolute;left:17px;right:17px;bottom:17px;height:6px;overflow:hidden;border-radius:99px;background:var(--surface-3)}
.metric-progress i{display:block;height:100%;border-radius:inherit;background:var(--metric-color);transition:width .3s ease}
.main-grid{display:grid;grid-template-columns:minmax(0,1.89fr) minmax(340px,1fr);gap:12px}
.bottom-grid{display:grid;grid-template-columns:1.03fr 1fr 1.09fr;gap:12px}
.dashboard-panel{min-width:0;border:1px solid var(--line);border-radius:12px;background:var(--surface);box-shadow:var(--shadow-soft)}
.panel-head{display:flex;align-items:flex-start;justify-content:space-between;gap:20px;padding:19px 20px 0}
.panel-head h2{margin:0;color:var(--text);font-size:var(--font-size-title);line-height:22px}
.panel-head p{margin:5px 0 0;color:var(--muted);font-size:var(--font-size-body)}
.panel-head>span{color:var(--muted);font-size:var(--font-size-caption);white-space:nowrap}
.chart-legend{display:flex;align-items:center;gap:24px;padding-top:3px;color:var(--text-soft);font-size:var(--font-size-body);font-weight:750}
.trend-controls{display:flex;align-items:center;gap:18px;flex-wrap:wrap}
.chart-modes{display:flex;gap:3px;padding:3px;border:1px solid var(--line);border-radius:9px;background:var(--surface-2)}
.chart-modes button{padding:5px 10px;border:0;border-radius:6px;background:transparent;color:var(--text-soft);font:inherit;font-size:var(--font-size-body);cursor:pointer}
.chart-modes button.active{background:var(--surface);color:var(--accent);box-shadow:var(--shadow-soft)}
.chart-modes button:disabled{opacity:.5;cursor:not-allowed}
.chart-legend span{display:inline-flex;align-items:center;gap:8px;white-space:nowrap}
.chart-legend span::before{width:27px;height:3px;border-radius:99px;background:#4f63f5;content:""}
.chart-legend .previous::before{height:0;border-top:2px dashed #9ba6b8;background:transparent}
.trend-chart{position:relative;padding:8px 14px 7px 8px}
.trend-chart svg{display:block;width:100%;height:250px;overflow:visible}
.grid-line{stroke:var(--line);stroke-width:1;stroke-dasharray:3 3}
.axis-label,.axis-unit{fill:var(--muted);font-size:var(--font-size-caption)}
.axis-unit{font-weight:650}
.chart-area{fill:url(#overview-area)}
.current-line,.previous-line{fill:none;vector-effect:non-scaling-stroke;stroke-linecap:round;stroke-linejoin:round}
.current-line{stroke:#4f63f5;stroke-width:2.7}
.previous-line{stroke:#9ba6b8;stroke-width:2;stroke-dasharray:5 5}
.chart-point{fill:var(--surface);stroke-width:1.8;vector-effect:non-scaling-stroke;pointer-events:none}
.chart-point.current{stroke:#4f63f5}
.chart-point.previous{stroke:#9ba6b8}
.hover-guide{stroke:#8e9aaf;stroke-width:1;stroke-dasharray:2 2;vector-effect:non-scaling-stroke;pointer-events:none}
.hover-point{fill:var(--surface);stroke-width:2.3;vector-effect:non-scaling-stroke;pointer-events:none}
.hover-point.current{stroke:#4f63f5}
.hover-point.previous{stroke:#9ba6b8}
.chart-hit-area{fill:transparent;stroke:transparent;cursor:pointer}
.chart-hit-area:focus{outline:none}
.chart-hit-area:focus-visible{stroke:color-mix(in srgb,var(--accent) 30%,transparent);stroke-width:2}
.trend-tooltip{position:absolute;z-index:3;min-width:184px;padding:12px 14px;border:1px solid var(--line-strong);border-radius:10px;background:color-mix(in srgb,var(--surface) 97%,transparent);box-shadow:0 12px 32px rgba(21,31,56,.18),0 2px 8px rgba(21,31,56,.08);backdrop-filter:blur(12px);pointer-events:none}
.trend-tooltip.align-center{transform:translate(-50%,12px)}
.trend-tooltip.align-start{transform:translate(0,12px)}
.trend-tooltip.align-end{transform:translate(-100%,12px)}
.trend-tooltip.place-above.align-center{transform:translate(-50%,calc(-100% - 12px))}
.trend-tooltip.place-above.align-start{transform:translate(0,calc(-100% - 12px))}
.trend-tooltip.place-above.align-end{transform:translate(-100%,calc(-100% - 12px))}
.trend-tooltip>strong{display:block;margin-bottom:8px;color:var(--text);font-size:var(--font-size-body)}
.trend-tooltip>span{display:grid;grid-template-columns:8px minmax(0,1fr) auto;align-items:center;gap:7px;min-height:23px;color:var(--text-soft);font-size:var(--font-size-caption);white-space:nowrap}
.trend-tooltip span>i{width:7px;height:7px;border-radius:50%;background:#4f63f5}
.trend-tooltip span>i.previous{background:#9ba6b8}
.trend-tooltip span>i.requests{background:#20b8b0}
.trend-tooltip span>b{color:var(--text);font-variant-numeric:tabular-nums}
.chart-tooltip-enter-active,.chart-tooltip-leave-active{transition:opacity .12s ease}
.chart-tooltip-enter-from,.chart-tooltip-leave-to{opacity:0}
.composition-panel{display:flex;flex-direction:column}
.composition-body{display:grid;grid-template-columns:minmax(180px,1fr) minmax(160px,.92fr);align-items:center;gap:28px;flex:1;padding:4px 22px 6px}
.donut{position:relative;width:min(194px,100%);aspect-ratio:1;margin:auto;display:grid;place-items:center}
.donut>svg{grid-area:1/1;width:100%;height:100%;overflow:visible}
.donut-track,.donut-segment{fill:none;transform:rotate(-90deg);transform-origin:60px 60px}
.donut-track{stroke:var(--surface-3);stroke-width:25}
.donut-segment{stroke-width:25;cursor:pointer;transition:stroke-width .16s ease,opacity .16s ease,filter .16s ease}
.donut-segment:hover,.donut-segment:focus,.donut-segment.active{stroke-width:32;filter:drop-shadow(0 4px 5px rgba(41,54,92,.2));outline:none}
.donut-segment.dimmed{opacity:.35}
.donut>div{z-index:1;grid-area:1/1;display:grid;width:98px;max-width:100%;text-align:center;pointer-events:none}
.donut strong{display:block;max-width:100%;font-size:var(--font-size-title);line-height:1;letter-spacing:-.02em;white-space:nowrap}
.donut strong.is-dense{font-size:var(--font-size-title)}
.donut strong.is-very-dense{font-size:var(--font-size-title)}
.donut span{margin-top:4px;overflow:hidden;color:var(--muted);font-size:var(--font-size-body);text-overflow:ellipsis;white-space:nowrap}
.composition-legend{display:grid;gap:4px;margin:0}
.composition-legend button{display:grid;grid-template-columns:minmax(0,1fr) auto;align-items:center;gap:16px;width:100%;min-height:42px;padding:5px 8px;border:1px solid transparent;border-radius:9px;background:transparent;color:var(--text-soft);font-size:var(--font-size-body);text-align:left;cursor:pointer;transition:background .14s ease,border-color .14s ease,opacity .14s ease}
.composition-legend button>span{display:flex;align-items:center;gap:10px;min-width:0}
.composition-legend button i{width:11px;height:11px;flex:0 0 auto;border-radius:50%}
.composition-legend button>b{min-width:72px;color:var(--text);font-size:var(--font-size-body);font-variant-numeric:tabular-nums;text-align:right;white-space:nowrap}
.composition-legend button:hover,.composition-legend button:focus-visible,.composition-legend button.active{border-color:color-mix(in srgb,var(--accent) 18%,var(--line));background:var(--surface-2);outline:none}
.composition-legend button.dimmed{opacity:.42}
.cache-summary{min-height:48px;display:flex;align-items:center;justify-content:center;gap:11px;margin:0 18px 12px;padding:8px 10px;border:1px solid color-mix(in srgb,var(--positive) 42%,var(--line));border-radius:9px;background:color-mix(in srgb,var(--positive) 7%,var(--surface));color:var(--positive);line-height:1}
.cache-summary span{font-size:var(--font-size-body);font-weight:800;line-height:1.2}
.cache-summary strong{font-size:var(--font-size-title);line-height:1}
.ranking-panel,.resources-panel{min-height:246px}
.ranking-list{display:grid;gap:2px;margin:8px 12px 6px;padding:0;list-style:none}
.ranking-list>li,.model-item{min-width:0;border:1px solid transparent;border-radius:9px;transition:border-color .14s ease,background .14s ease}
.ranking-list>li:hover,.model-item:hover{background:color-mix(in srgb,var(--surface-2) 72%,transparent)}
.ranking-list>li.selected,.model-item.selected{border-color:color-mix(in srgb,var(--accent) 20%,var(--line));background:color-mix(in srgb,var(--accent-soft) 52%,var(--surface))}
.ranking-row,.model-row{display:grid;grid-template-columns:20px minmax(78px,.52fr) minmax(90px,1.25fr) auto 16px;align-items:center;gap:11px;width:100%;min-width:0;min-height:29px;padding:3px 7px;border:0;border-radius:9px;background:transparent;color:inherit;text-align:left;cursor:pointer}
.ranking-row>b{font-size:var(--font-size-body)}
.ranking-name{overflow:hidden;color:var(--text-soft);font-size:var(--font-size-body);white-space:nowrap;text-overflow:ellipsis}
.ranking-bar{height:9px;overflow:hidden;border-radius:99px;background:var(--surface-3)}
.ranking-bar i{display:block;height:100%;border-radius:inherit;background:linear-gradient(90deg,#5368f5,#3d8bf0);transition:width .3s ease}
.ranking-row>strong,.model-row>strong{min-width:45px;color:var(--text);font-size:var(--font-size-body);text-align:right;font-variant-numeric:tabular-nums}
.ranking-row>.a-icon,.model-row>.a-icon{color:var(--muted);transition:transform .16s ease,color .16s ease}
.selected>.ranking-row>.a-icon,.selected>.model-row>.a-icon{color:var(--accent);transform:rotate(90deg)}
.ranking-detail{max-height:100px;display:grid;grid-template-columns:repeat(4,minmax(0,1fr));gap:1px;margin:0 7px 7px;overflow:hidden;border:1px solid var(--line);border-radius:8px;background:var(--surface)}
.ranking-detail>div{display:grid;gap:3px;min-width:0;padding:8px 9px;border-right:1px solid var(--line)}
.ranking-detail>div:last-child{border-right:0}
.ranking-detail span{overflow:hidden;color:var(--muted);font-size:var(--font-size-caption);text-overflow:ellipsis;white-space:nowrap}
.ranking-detail strong{overflow:hidden;color:var(--text);font-size:var(--font-size-body);font-variant-numeric:tabular-nums;text-overflow:ellipsis;white-space:nowrap}
.ranking-detail-enter-active,.ranking-detail-leave-active{transition:max-height .18s ease,opacity .14s ease,margin .18s ease}
.ranking-detail-enter-from,.ranking-detail-leave-to{max-height:0;margin-block:0;opacity:0}
.idle-note{min-height:38px;display:flex;align-items:center;gap:9px;margin:0 12px 10px;padding:8px 12px;border:1px solid var(--line);border-radius:9px;background:var(--surface-2);color:var(--text-soft);font-size:var(--font-size-body)}
.model-list{display:grid;gap:8px;margin:14px}
.model-row{grid-template-columns:minmax(100px,.7fr) minmax(100px,1.25fr) 40px 16px}
.execution-summary{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:8px;margin:auto 14px 12px;padding-top:10px;border-top:1px solid var(--line)}
.execution-summary>div{min-width:0;display:flex;align-items:center;justify-content:space-between;gap:8px;padding:7px 9px;border-radius:8px;background:var(--surface-2)}
.execution-summary dt,.execution-summary dd{min-width:0;margin:0;font-size:var(--font-size-caption);white-space:nowrap}
.execution-summary dt{overflow:hidden;color:var(--text-soft);text-overflow:ellipsis}.execution-summary dd{color:var(--text);font-weight:750;font-variant-numeric:tabular-nums}
.empty-copy{display:grid;place-items:center;min-height:145px;margin:0;color:var(--muted);font-size:var(--font-size-body)}
.resource-list{margin:10px 18px}
.resource-list>div{display:flex;align-items:center;justify-content:space-between;gap:16px;min-height:45px;border-top:1px solid var(--line)}
.resource-list dt,.resource-list dd{display:flex;align-items:center;gap:9px;margin:0;font-size:var(--font-size-body)}
.resource-list dt{color:var(--text-soft);font-weight:700}
.resource-list dd{font-variant-numeric:tabular-nums}
.resource-list dd>i{width:8px;height:8px;border-radius:50%;background:var(--warning)}
.resource-list dd>i.healthy{background:var(--positive);box-shadow:0 0 0 4px color-mix(in srgb,var(--positive) 10%,transparent)}
@keyframes overview-spin{to{transform:rotate(360deg)}}
@media(max-width:1280px){.kpi-grid{grid-template-columns:repeat(3,minmax(0,1fr))}.main-grid{grid-template-columns:minmax(0,1.6fr) minmax(300px,.85fr)}}
@media(max-width:1080px){.overview-head{align-items:stretch;flex-direction:column}.overview-actions{justify-content:space-between}.main-grid{grid-template-columns:1fr}.composition-body{grid-template-columns:210px 1fr}.ranking-panel,.resources-panel{min-height:auto}}
@container overview-dashboard (max-width:1000px){.bottom-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.resources-panel{grid-column:1/-1;min-height:auto}.resources-panel .resource-list{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));column-gap:20px}.composition-body{grid-template-columns:minmax(112px,.82fr) minmax(0,1fr);gap:10px;padding-inline:12px}.donut{width:min(150px,100%)}.composition-legend button{gap:6px;padding-inline:4px}.composition-legend button>b{min-width:50px}}
@container overview-dashboard (max-width:760px){.bottom-grid{grid-template-columns:1fr}.resources-panel{grid-column:auto}.resources-panel .resource-list{grid-template-columns:repeat(2,minmax(0,1fr))}}
@media(max-width:720px){.overview-actions{align-items:stretch;flex-direction:column}.refresh-button{justify-content:center}.kpi-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.composition-body{grid-template-columns:1fr}.donut{width:170px}.panel-head{padding-inline:16px}.chart-legend{gap:12px;font-size:var(--font-size-caption)}}
@media(max-width:720px){.ranking-detail{grid-template-columns:repeat(2,minmax(0,1fr))}.ranking-detail>div:nth-child(2){border-right:0}.ranking-detail>div:nth-child(-n+2){border-bottom:1px solid var(--line)}}
@media(max-width:480px){.kpi-grid{grid-template-columns:1fr}.kpi-card{min-height:138px}.overview-head p{font-size:var(--font-size-body)}.ranking-row{grid-template-columns:18px minmax(72px,.65fr) minmax(64px,1fr) auto 14px}}
.overview-period{min-width:0}.refresh-button{height:40px}
.overview-page{min-height:calc(100vh - var(--header-height));min-height:calc(100dvh - var(--header-height));display:flex;flex-direction:column}
.dashboard-body{min-height:0;flex:1;grid-template-rows:auto minmax(330px,1fr) auto}
.main-grid,.bottom-grid{min-height:0}
.trend-panel{min-height:0;display:grid;grid-template-rows:auto minmax(0,1fr)}
.trend-chart{min-height:0;height:100%}.trend-chart svg{height:100%;min-height:252px}
.bottom-grid>.dashboard-panel{height:100%}
@media(max-height:980px){.overview-page{min-height:0;display:block}.dashboard-body{grid-template-rows:auto}.trend-chart{height:auto}.trend-chart svg{height:250px}}
@media(max-width:1080px){.overview-page{min-height:0;display:block}.dashboard-body{grid-template-rows:auto}.trend-chart{height:auto}.trend-chart svg{height:250px}.bottom-grid>.dashboard-panel{height:auto}}
</style>
