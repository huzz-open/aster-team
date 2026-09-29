<script setup lang="ts">
import { computed } from 'vue'
import AButton from './AButton.vue'
import ASelect from './ASelect.vue'
import ASegmentedControl from './ASegmentedControl.vue'

const props = withDefaults(defineProps<{
  page: number
  pageSize?: number
  pageSizeOptions?: number[]
  inlinePageSizes?: boolean
  total?: number
  hasNext?: boolean
  loading?: boolean
  locale?: string
}>(), {
  pageSize: 50,
  pageSizeOptions: () => [20, 50, 100],
  inlinePageSizes: false,
  hasNext: false,
  loading: false,
  locale: 'zh-CN',
})

const emit = defineEmits<{
  'update:page': [page: number]
  'update:pageSize': [pageSize: number]
  change: [page: number]
  'page-size-change': [pageSize: number]
}>()

const normalizedPage = computed(() => Math.max(1, Math.trunc(props.page) || 1))
const pageCount = computed(() => props.total === undefined
  ? undefined
  : Math.max(1, Math.ceil(Math.max(0, props.total) / props.pageSize)))
const canPrevious = computed(() => normalizedPage.value > 1 && !props.loading)
const canNext = computed(() => !props.loading && (pageCount.value === undefined
  ? props.hasNext
  : normalizedPage.value < pageCount.value))
const isEnglish = computed(() => props.locale.toLowerCase().startsWith('en'))
const normalizedPageSizes = computed(() => [...new Set([props.pageSize, ...props.pageSizeOptions]
  .filter(value => Number.isSafeInteger(value) && value > 0))].sort((left, right) => left - right))
const pageSizeChoices = computed(() => normalizedPageSizes.value.map(size => ({
  value: size,
  label: String(size),
})))
const summary = computed(() => {
  if (pageCount.value === undefined) {
    return isEnglish.value ? `Page ${normalizedPage.value}` : `第 ${normalizedPage.value} 页`
  }
  return isEnglish.value
    ? `Page ${normalizedPage.value} of ${pageCount.value} · ${props.total?.toLocaleString(props.locale)} records`
    : `第 ${normalizedPage.value} / ${pageCount.value} 页 · 共 ${props.total?.toLocaleString(props.locale)} 条`
})

function move(nextPage: number) {
  if (nextPage === normalizedPage.value || nextPage < 1) return
  if (pageCount.value !== undefined && nextPage > pageCount.value) return
  emit('update:page', nextPage)
  emit('change', nextPage)
}

function resize(value: string | number) {
  const nextPageSize = Number(value)
  if (!Number.isSafeInteger(nextPageSize) || nextPageSize <= 0 || nextPageSize === props.pageSize) return
  emit('update:pageSize', nextPageSize)
  emit('page-size-change', nextPageSize)
  if (normalizedPage.value !== 1) emit('update:page', 1)
}
</script>

<template>
  <nav class="a-pagination" :aria-label="isEnglish ? 'Pagination' : '分页导航'">
    <span class="a-pagination-summary" aria-live="polite">{{ summary }}</span>
    <div v-if="total !== undefined" class="a-pagination-size">
      <span>{{ isEnglish ? 'Per page' : '每页' }}</span>
      <ASegmentedControl v-if="inlinePageSizes" :model-value="pageSize" :options="pageSizeChoices.map(option => ({ ...option, disabled: loading }))" :label="isEnglish ? 'Records per page' : '每页条数'" size="small" @update:model-value="resize" />
      <ASelect v-else class="a-pagination-size-select" :model-value="pageSize" :options="pageSizeChoices" :disabled="loading" :popup-min-width="50" align="center" :aria-label="isEnglish ? 'Records per page' : '每页条数'" @update:model-value="resize" />
      <span>{{ isEnglish ? 'rows' : '条' }}</span>
    </div>
    <div class="a-pagination-actions">
      <AButton variant="secondary" size="small" :disabled="!canPrevious" @click="move(normalizedPage - 1)">
        {{ isEnglish ? 'Previous' : '上一页' }}
      </AButton>
      <AButton variant="secondary" size="small" :disabled="!canNext" @click="move(normalizedPage + 1)">
        {{ isEnglish ? 'Next' : '下一页' }}
      </AButton>
    </div>
  </nav>
</template>

<style scoped>
.a-pagination{min-height:42px;display:flex;align-items:center;justify-content:flex-end;gap:11px;padding-top:10px}.a-pagination-summary{color:var(--muted);font-size:var(--font-size-caption);font-variant-numeric:tabular-nums;white-space:nowrap}.a-pagination-size{display:flex;align-items:center;gap:5px;color:var(--muted);font-size:var(--font-size-caption);white-space:nowrap}.a-pagination-size-select{width:50px;flex:0 0 50px}.a-pagination-size-select:deep(.a-select-trigger){min-height:32px;padding:5px 4px;gap:0;border-radius:8px;font-size:var(--font-size-body);font-variant-numeric:tabular-nums}.a-pagination-size-select:deep(.a-select-chevron){position:static;right:auto;width:11px;height:11px}.a-pagination-actions{display:flex;align-items:center;gap:8px}@media(max-width:560px){.a-pagination{align-items:stretch;flex-direction:column}.a-pagination-summary{text-align:center}.a-pagination-size{justify-content:center}.a-pagination-actions>.a-button{flex:1}}
</style>
