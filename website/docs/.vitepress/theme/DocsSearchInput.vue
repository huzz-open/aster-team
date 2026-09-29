<script setup lang="ts">
import { computed, nextTick, ref } from 'vue'
import { useData } from 'vitepress'

const { lang } = useData()
const query = ref('')
const placeholder = computed(() => lang.value.startsWith('zh') ? '搜索文档' : 'Search documentation')

async function focusSearch() {
  if (typeof document === 'undefined') return
  document.querySelector<HTMLButtonElement>('.VPNavBarSearch .DocSearch-Button')?.click()
  await nextTick()
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const input = document.querySelector<HTMLInputElement>('#localsearch-input')
    if (input) {
      if (query.value) {
        input.value = query.value
        input.dispatchEvent(new Event('input', { bubbles: true }))
        query.value = ''
      }
      input.focus()
      return
    }
    await new Promise(resolve => window.setTimeout(resolve, 25))
  }
}
</script>

<template>
  <label class="docs-search-input">
    <span class="docs-search-input__icon" aria-hidden="true" />
    <input
      v-model="query"
      type="search"
      :placeholder="placeholder"
      :aria-label="placeholder"
      autocomplete="off"
      @focus="focusSearch"
      @input="focusSearch"
    >
    <kbd>Ctrl K</kbd>
  </label>
</template>
