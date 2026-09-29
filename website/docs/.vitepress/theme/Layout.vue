<script setup lang="ts">
import DefaultTheme from 'vitepress/theme'
import { useData } from 'vitepress'
import { computed } from 'vue'
import DocsSearchInput from './DocsSearchInput.vue'

const { page } = useData()
const isReferencePage = computed(() => /\/(api|models)\//.test(page.value.relativePath))
</script>

<template>
  <DefaultTheme.Layout :class="{ 'docs-reference': isReferencePage }">
    <template #nav-bar-content-before>
      <DocsSearchInput />
    </template>
  </DefaultTheme.Layout>
</template>
