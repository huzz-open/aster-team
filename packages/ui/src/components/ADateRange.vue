<script setup lang="ts">
import { computed } from 'vue'
import { VueDatePicker } from '@vuepic/vue-datepicker'
import { enUS, zhCN } from 'date-fns/locale'
import '@vuepic/vue-datepicker/dist/main.css'

const props = withDefaults(defineProps<{
  from?: string
  to?: string
  min?: string
  max?: string
  startLabel?: string
  endLabel?: string
  locale?: 'zh-CN' | 'en-US'
  disabled?: boolean
  maxRangeDays?: number
}>(), {
  from: '',
  to: '',
  min: '',
  max: '',
  startLabel: '开始日期',
  endLabel: '结束日期',
  locale: 'zh-CN',
  disabled: false,
  maxRangeDays: 0,
})

const emit = defineEmits<{
  'update:from': [value: string]
  'update:to': [value: string]
  change: [boundary: 'from' | 'to']
}>()

function parseLocalDate(value: string) {
  const parts = value.split('-').map(Number)
  if (parts.length !== 3 || parts.some(part => !Number.isFinite(part))) return null
  return new Date(parts[0]!, parts[1]! - 1, parts[2]!)
}

function formatLocalDate(value: Date) {
  return `${value.getFullYear()}-${String(value.getMonth() + 1).padStart(2, '0')}-${String(value.getDate()).padStart(2, '0')}`
}

const rangeValue = computed<Date[] | null>({
  get() {
    const from = parseLocalDate(props.from)
    const to = parseLocalDate(props.to)
    return from && to ? [from, to] : null
  },
  set(value) {
    if (value === null) {
      emit('update:from', '')
      emit('update:to', '')
      emit('change', 'to')
      return
    }
    if (!Array.isArray(value) || !(value[0] instanceof Date) || !(value[1] instanceof Date)) return
    emit('update:from', formatLocalDate(value[0]))
    emit('update:to', formatLocalDate(value[1]))
    emit('change', 'to')
  },
})

const calendarLocale = computed(() => props.locale === 'en-US' ? enUS : zhCN)
const maximumRangeDays = computed(() => Math.max(0, Math.floor(props.maxRangeDays)))
const dateLimits = computed(() => {
  const minDate = parseLocalDate(props.min)
  const maxDate = parseLocalDate(props.max)
  return { ...(minDate ? { minDate } : {}), ...(maxDate ? { maxDate } : {}) }
})
const rangeConfig = computed(() => ({
  partialRange: false,
  autoSwitchStartEnd: true,
  ...(maximumRangeDays.value > 0 ? { maxRange: maximumRangeDays.value - 1 } : {}),
}))
const inputLabel = computed(() => {
  const base = `${props.startLabel} → ${props.endLabel}`
  if (!maximumRangeDays.value) return base
  return props.locale === 'en-US' ? `${base}, up to ${maximumRangeDays.value} days` : `${base}，最多 ${maximumRangeDays.value} 天`
})
const clearLabel = computed(() => props.locale === 'en-US' ? 'Clear date range' : '清除日期范围')
const inputFormat = computed(() => ({
  month: props.locale === 'en-US' ? 'MMMM' : 'M月',
  year: props.locale === 'en-US' ? 'yyyy' : 'yyyy年',
  weekDay: 'EEEEE',
  input: (dates: Date[]) => dates.map(formatLocalDate).join('  →  '),
}))
</script>

<template>
  <VueDatePicker
    v-model="rangeValue"
    class="a-date-range"
    :locale="calendarLocale"
    :range="rangeConfig"
    :multi-calendars="{ count: 2, static: false }"
    v-bind="dateLimits"
    :disabled="disabled"
    :formats="inputFormat"
    :time-config="{ enableTimePicker: false }"
    :input-attrs="{ clearable: true, autocomplete: 'off' }"
    :ui="{ menu: 'a-date-range-menu' }"
    :aria-labels="{ input: inputLabel, clearInput: clearLabel }"
    :floating="{ placement: 'bottom-end', offset: 8, arrow: false }"
    :config="{ mobileBreakpoint: 720 }"
    year-first
    auto-apply
    teleport
  />
</template>

<style>
.a-date-range {
  width: 220px;
  min-width: 220px;
  max-width: 100%;
  --dp-font-family: var(--font-ui);
  --dp-border-radius: 12px;
  --dp-cell-border-radius: 7px;
  --dp-font-size:var(--font-size-body);
  --dp-cell-size: 34px;
  --dp-cell-padding: 4px;
  --dp-row-margin: 3px 0;
  --dp-calendar-header-cell-padding: 5px;
  --dp-month-year-row-height: 38px;
  --dp-month-year-row-button-size: 30px;
  --dp-button-icon-height: 16px;
  --dp-input-icon-padding: 12px;
  --dp-input-padding: 8px 38px 8px 12px;
  --dp-input-not-clearable-padding: 12px 38px 12px 12px;
  --dp-menu-padding: 10px 12px;
  --dp-multi-calendars-spacing: 20px;
}
.a-date-range .dp--input {
  min-height: var(--control-height);
  color: var(--text);
  border-color: var(--line-strong);
  border-radius: var(--control-radius);
  background: linear-gradient(180deg,var(--surface),color-mix(in srgb,var(--surface) 88%,var(--surface-2)));
  box-shadow: inset 0 1px 0 color-mix(in srgb,#fff 55%,transparent);
  font-variant-numeric: tabular-nums;
}
.a-date-range .dp--input:hover { border-color: color-mix(in srgb,var(--accent) 35%,var(--line-strong)); }
.a-date-range .dp--input-focus { border-color: var(--accent); box-shadow: var(--focus-ring); background: var(--surface); }
.a-date-range .dp--input-icon { inset-inline-start: auto; inset-inline-end: 0; }
.a-date-range .dp--input-icons { width: 16px; height: 16px; padding: 6px 11px; color: var(--muted); }
.a-date-range .dp--clear-btn { inset-inline-end: 0; width: 38px; height: 100%; justify-content: center; color: var(--muted); }
.a-date-range .dp--clear-btn:hover { color: var(--danger); }
.a-date-range .dp--input-wrap:has(.dp--clear-btn) .dp--input-icon { display: none; }
.a-date-range-menu.dp--theme-light,.a-date-range-menu.dp--theme-dark {
  --dp-background-color: var(--surface);
  --dp-text-color: var(--text);
  --dp-hover-color: color-mix(in srgb,var(--accent-soft) 76%,var(--surface));
  --dp-hover-text-color: var(--accent);
  --dp-hover-icon-color: var(--accent);
  --dp-primary-color: var(--accent);
  --dp-primary-disabled-color: color-mix(in srgb,var(--accent) 45%,var(--surface));
  --dp-primary-text-color: #fff;
  --dp-secondary-color: var(--muted);
  --dp-border-color: var(--line);
  --dp-menu-border-color: var(--line-strong);
  --dp-border-color-hover: var(--accent);
  --dp-border-color-focus: var(--accent);
  --dp-disabled-color: var(--surface-2);
  --dp-disabled-color-text: var(--muted);
  --dp-icon-color: var(--muted);
  --dp-highlight-color: var(--accent-soft);
  --dp-range-between-dates-background-color: color-mix(in srgb,var(--accent) 11%,var(--surface));
  --dp-range-between-dates-text-color: var(--text);
  --dp-range-between-border-color: transparent;
}
.dp--menu-wrapper { z-index: 220; }
.a-date-range-menu {
  overflow: hidden;
  border-color: color-mix(in srgb,var(--accent) 13%,var(--line-strong));
  border-radius: 17px;
  background: color-mix(in srgb,var(--surface) 97%,transparent);
  box-shadow: 0 24px 64px rgba(20,29,52,.18),0 4px 14px rgba(20,29,52,.08),inset 0 1px 0 color-mix(in srgb,#fff 70%,transparent);
  font-family: var(--font-ui);
  backdrop-filter: blur(18px);
  --dp-cell-size: 36px;
  --dp-cell-border-radius: 9px;
  --dp-row-margin: 2px 0;
  --dp-calendar-header-cell-padding: 5px;
  --dp-month-year-row-height: 40px;
  --dp-month-year-row-button-size: 30px;
  --dp-button-icon-height: 15px;
  --dp-menu-padding: 12px 14px 14px;
  --dp-multi-calendars-spacing: 18px;
  --dp-animation-duration: .16s;
  --dp-transition-length: 8px;
}
[data-theme="dark"] .a-date-range-menu {
  border-color: color-mix(in srgb,var(--accent) 18%,var(--line-strong));
  box-shadow: 0 28px 72px rgba(0,0,0,.5),0 4px 16px rgba(0,0,0,.28),inset 0 1px 0 rgba(255,255,255,.06);
}
.a-date-range-menu .dp--menu-inner { gap: 0; }
.a-date-range-menu .dp--calendar-next {
  margin-inline-start: 18px;
  padding-inline-start: 18px;
  border-inline-start: 1px solid var(--line);
}
.a-date-range-menu .dp--month-year-row { margin-bottom: 3px; font-weight: 750; }
.a-date-range-menu .dp--month-year-select-base {
  height: 32px;
  border: 1px solid transparent;
  border-radius: 9px;
  color: var(--text);
  font-size:var(--font-size-body);
  font-weight: 760;
  transition: color .14s ease,border-color .14s ease,background .14s ease;
}
.a-date-range-menu .dp--month-year-select-base:hover {
  color: var(--accent);
  border-color: color-mix(in srgb,var(--accent) 18%,var(--line));
  background: var(--accent-soft);
}
.a-date-range-menu .dp--inner-nav {
  width: 30px;
  height: 30px;
  flex: 0 0 30px;
  border: 1px solid transparent;
  border-radius: 9px;
  transition: color .14s ease,border-color .14s ease,background .14s ease;
}
.a-date-range-menu .dp--inner-nav:hover {
  color: var(--accent);
  border-color: color-mix(in srgb,var(--accent) 20%,var(--line));
  background: var(--accent-soft);
}
.a-date-range-menu .dp--calendar-header { color: var(--muted); font-size:var(--font-size-caption); font-weight: 720; }
.a-date-range-menu .dp--calendar-header-cell { border: 0; }
.a-date-range-menu .dp--calendar-header-separator {
  height: 1px;
  margin: 0 0 7px;
  background: linear-gradient(90deg,transparent,var(--line) 12%,var(--line) 88%,transparent);
}
.a-date-range-menu .dp--cell-inner {
  border-color: transparent;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  font-variant-numeric: tabular-nums;
  transition: color .14s ease,background .14s ease,border-color .14s ease,box-shadow .14s ease,transform .1s ease;
}
.a-date-range-menu .dp--cell-inner:focus-visible {
  outline: 0;
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}
.a-date-range-menu .dp--calendar-item:focus-visible { outline: 0; }
.a-date-range-menu .dp--calendar-item:focus-visible .dp--cell-inner {
  border-color: var(--accent);
  box-shadow: var(--focus-ring);
}
.a-date-range-menu .dp--cell-offset { color: color-mix(in srgb,var(--muted) 58%,transparent); }
.a-date-range-menu .dp--date-hoverable:hover {
  color: var(--accent);
  border-color: color-mix(in srgb,var(--accent) 12%,transparent);
  background: var(--accent-soft);
  transform: scale(.94);
}
.a-date-range-menu .dp--range-between {
  color: var(--text);
  background: color-mix(in srgb,var(--accent) 11%,var(--surface));
  box-shadow: inset 0 1px 0 color-mix(in srgb,var(--accent) 7%,transparent),inset 0 -1px 0 color-mix(in srgb,var(--accent) 7%,transparent);
}
.a-date-range-menu .dp--range-start,
.a-date-range-menu .dp--range-end,
.a-date-range-menu .dp--active {
  color: #fff;
  border-color: color-mix(in srgb,var(--accent) 76%,#fff);
  background: linear-gradient(145deg,color-mix(in srgb,var(--accent) 88%,#fff),var(--accent));
  font-weight: 780;
  box-shadow: 0 5px 13px color-mix(in srgb,var(--accent) 27%,transparent),inset 0 1px 0 rgba(255,255,255,.28);
}
.a-date-range-menu .dp--range-start { border-start-end-radius: 4px; border-end-end-radius: 4px; }
.a-date-range-menu .dp--range-end { border-start-start-radius: 4px; border-end-start-radius: 4px; }
.a-date-range-menu .dp--today:not(.dp--active) {
  color: var(--accent);
  border-color: color-mix(in srgb,var(--accent) 38%,transparent);
  background: color-mix(in srgb,var(--accent-soft) 56%,transparent);
  font-weight: 760;
  box-shadow: inset 0 0 0 1px color-mix(in srgb,var(--accent) 7%,transparent);
}
.a-date-range-menu .dp--cell-disabled { color: color-mix(in srgb,var(--muted) 52%,var(--surface)); background: transparent; opacity: 1; }
.a-date-range-menu .dp--overlay { background: color-mix(in srgb,var(--surface) 98%,transparent); }
.a-date-range-menu .dp--overlay-container { scrollbar-width: none; }
.a-date-range-menu .dp--overlay-container::-webkit-scrollbar { display: none; }
.a-date-range-menu .dp--overlay-cell {
  border: 1px solid transparent;
  border-radius: 9px;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  transition: color .14s ease,border-color .14s ease,background .14s ease;
}
.a-date-range-menu .dp--overlay-cell:hover { color: var(--accent); border-color: color-mix(in srgb,var(--accent) 18%,var(--line)); background: var(--accent-soft); }
.a-date-range-menu .dp--overlay-cell:focus-visible { outline: 0; border-color: var(--accent); box-shadow: var(--focus-ring); }
.a-date-range-menu .dp--overlay-col:focus-visible { outline: 0; }
.a-date-range-menu .dp--overlay-col:focus-visible .dp--overlay-cell { border-color: var(--accent); box-shadow: var(--focus-ring); }
.a-date-range-menu .dp--overlay-cell-active {
  color: #fff;
  border-color: color-mix(in srgb,var(--accent) 76%,#fff);
  background: linear-gradient(145deg,color-mix(in srgb,var(--accent) 88%,#fff),var(--accent));
  box-shadow: 0 4px 11px color-mix(in srgb,var(--accent) 24%,transparent),inset 0 1px 0 rgba(255,255,255,.28);
  font-weight: 750;
}
@media (max-width:720px) {
  .a-date-range-menu { max-width: calc(100vw - 24px); --dp-menu-padding: 10px; }
  .a-date-range-menu .dp--calendar-next { margin-inline-start: 0; padding-inline-start: 0; border-inline-start: 0; }
}
</style>
