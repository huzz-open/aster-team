<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, useId } from 'vue'
import ABrandMark from './ABrandMark.vue'
import AIcon from './AIcon.vue'
import AIconButton from './AIconButton.vue'
import type { ConsoleNavSection, ConsoleQuickLink } from '../console'

const props = withDefaults(defineProps<{
  product?: string
  subtitle: string
  statusLabel?: string
  statusTone?: 'positive' | 'warning' | 'danger' | 'neutral'
  sections: ConsoleNavSection[]
  userName?: string
  userMeta?: string
  locale?: 'zh-CN' | 'en-US'
  showLocale?: boolean
  quickLinks?: ConsoleQuickLink[]
  profileLinks?: Array<{ to: string; label: string; icon: string }>
  labels?: Partial<Record<'notice' | 'theme' | 'language' | 'profile' | 'logout' | 'collapse' | 'expand' | 'menu' | 'close' | 'allClear', string>>
}>(), { product: 'Aster Team', statusLabel: '', statusTone: 'neutral', userName: '', userMeta: '', locale: 'zh-CN', showLocale: false, quickLinks: () => [], profileLinks: () => [], labels: () => ({}) })

const emit = defineEmits<{ logout: []; localeChange: [value: 'zh-CN' | 'en-US'] }>()
const collapsed = ref(false)
const mobileOpen = ref(false)
const noticeOpen = ref(false)
const profileOpen = ref(false)
const theme = ref<'light' | 'dark'>('light')
const shellID = useId()
const noticeID = `console-notice-${shellID}`
const profileID = `console-profile-${shellID}`
const initials = computed(() => (props.userName || props.product).trim().slice(0, 2).toUpperCase())
const text = computed(() => ({
  notice: props.labels.notice || '系统通知', theme: props.labels.theme || '切换主题', language: props.labels.language || '语言',
  profile: props.labels.profile || '账户', logout: props.labels.logout || '退出登录', collapse: props.labels.collapse || '收起导航',
  expand: props.labels.expand || (props.locale === 'en-US' ? 'Expand sidebar' : '展开导航'),
  menu: props.labels.menu || (props.locale === 'en-US' ? 'Open navigation' : '打开导航'),
  close: props.labels.close || (props.locale === 'en-US' ? 'Close navigation' : '关闭导航'),
  allClear: props.labels.allClear || '当前没有需要处理的系统通知。',
}))
const themeActionLabel = computed(() => theme.value === 'dark'
  ? (props.locale === 'en-US' ? 'Switch to light mode' : '切换到浅色模式')
  : (props.locale === 'en-US' ? 'Switch to dark mode' : '切换到深色模式'))
const localeActionLabel = computed(() => props.locale === 'zh-CN' ? 'Switch to English' : '切换到中文')

function applyTheme(value: 'light' | 'dark') {
  theme.value = value
  document.documentElement.dataset.theme = value
  localStorage.setItem('aster-theme', value)
}
function toggleTheme() { applyTheme(theme.value === 'dark' ? 'light' : 'dark') }
function toggleCollapsed() { collapsed.value = !collapsed.value; localStorage.setItem('aster-sidebar-collapsed', String(collapsed.value)) }
function closeMobile() { mobileOpen.value = false; noticeOpen.value = false; profileOpen.value = false }
function handleDocumentPointerDown(event: PointerEvent) {
  if (!(event.target instanceof Element) || event.target.closest('.console-popover-wrap')) return
  noticeOpen.value = false
  profileOpen.value = false
}
function handleDocumentKeydown(event: KeyboardEvent) {
  if (event.key !== 'Escape') return
  closeMobile()
}

onMounted(() => {
  const stored = localStorage.getItem('aster-theme')
  applyTheme(stored === 'dark' || stored === 'light' ? stored : (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'))
  collapsed.value = localStorage.getItem('aster-sidebar-collapsed') === 'true'
  document.addEventListener('pointerdown', handleDocumentPointerDown)
  document.addEventListener('keydown', handleDocumentKeydown)
})
onBeforeUnmount(() => {
  document.removeEventListener('pointerdown', handleDocumentPointerDown)
  document.removeEventListener('keydown', handleDocumentKeydown)
})
</script>

<template>
  <div class="console-shell" :class="{ 'is-collapsed': collapsed, 'mobile-nav-open': mobileOpen }">
    <header class="console-header">
      <button class="console-icon-button mobile-menu" type="button" :aria-label="text.menu" :aria-expanded="mobileOpen" @click.stop="mobileOpen = !mobileOpen"><AIcon name="menu" /></button>
      <RouterLink class="console-brand" to="/" @click="closeMobile">
        <ABrandMark class="console-brand-mark" :size="36" label="" />
        <span class="console-brand-copy"><span class="console-brand-title"><strong>{{ product }}</strong><span v-if="statusLabel" class="console-brand-status" :class="`is-${statusTone}`">{{ statusLabel }}</span></span><small>{{ subtitle }}</small></span>
      </RouterLink>
      <nav v-if="quickLinks.length" class="console-quick-links" :aria-label="locale === 'en-US' ? 'Quick links' : '快捷入口'">
        <template v-for="item in quickLinks" :key="item.to || item.href">
          <a v-if="item.href" :href="item.href" :class="{ 'is-mobile-visible': item.keepVisibleOnMobile }" :aria-label="item.label" :title="item.label" target="_blank" rel="noreferrer"><AIcon :name="item.icon" :size="15" /><span>{{ item.label }}</span></a>
          <RouterLink v-else-if="item.to" :to="item.to" :class="{ 'is-mobile-visible': item.keepVisibleOnMobile }" :aria-label="item.label" :title="item.label"><AIcon :name="item.icon" :size="15" /><span>{{ item.label }}</span></RouterLink>
        </template>
      </nav>
      <div class="console-header-actions">
        <div class="console-popover-wrap">
          <AIconButton class="console-icon-button" icon="bell" :label="text.notice" aria-haspopup="dialog" :aria-controls="noticeID" :aria-expanded="noticeOpen" @click="noticeOpen = !noticeOpen; profileOpen = false" />
          <Transition name="a-popover"><div v-if="noticeOpen" :id="noticeID" class="console-popover notification-popover" role="dialog" :aria-label="text.notice"><strong>{{ text.notice }}</strong><p>{{ text.allClear }}</p></div></Transition>
        </div>
        <AIconButton class="console-icon-button" :icon="theme === 'dark' ? 'sun' : 'moon'" :label="themeActionLabel" @click="toggleTheme" />
        <button v-if="showLocale" class="locale-button" type="button" :title="localeActionLabel" :aria-label="localeActionLabel" @click="emit('localeChange', locale === 'zh-CN' ? 'en-US' : 'zh-CN')"><AIcon name="globe" /><span>{{ locale === 'zh-CN' ? 'EN' : '中' }}</span></button>
        <div class="console-popover-wrap">
          <button class="profile-button" type="button" :aria-label="text.profile" aria-haspopup="dialog" :aria-controls="profileID" :aria-expanded="profileOpen" @click="profileOpen = !profileOpen; noticeOpen = false"><span class="profile-avatar">{{ initials }}</span><span class="profile-copy"><strong>{{ userName || text.profile }}</strong><small>{{ userMeta }}</small></span><AIcon class="profile-chevron" :class="{ 'is-open': profileOpen }" name="chevron" :size="14" /></button>
          <Transition name="a-popover"><div v-if="profileOpen" :id="profileID" class="console-popover profile-popover" role="dialog" :aria-label="text.profile"><div class="profile-popover-head"><strong>{{ userName || text.profile }}</strong><small>{{ userMeta }}</small></div><RouterLink v-for="item in profileLinks" :key="item.to" :to="item.to" @click="profileOpen=false"><AIcon :name="item.icon" />{{ item.label }}</RouterLink><button type="button" @click="emit('logout')"><AIcon name="logout" />{{ text.logout }}</button></div></Transition>
        </div>
      </div>
    </header>

    <aside class="console-sidebar">
      <nav class="console-nav">
        <section v-for="(section, sectionIndex) in sections" :key="sectionIndex" class="console-nav-section">
          <div v-if="section.label" class="console-nav-label">{{ section.label }}</div>
          <template v-for="item in section.items" :key="item.to">
            <span v-if="item.disabled" class="console-nav-disabled" aria-disabled="true" :title="item.disabledLabel || item.label"><AIcon :name="item.icon" /><span>{{ item.label }}</span><AIcon class="nav-lock" name="lock" :size="13" /></span>
            <RouterLink v-else :to="item.to" :title="collapsed ? item.label : item.locked ? item.lockedLabel : undefined" @click="closeMobile"><AIcon :name="item.icon" /><span>{{ item.label }}</span><AIcon v-if="item.locked" class="nav-lock" name="lock" :size="13" /></RouterLink>
          </template>
        </section>
      </nav>
      <button class="sidebar-collapse" type="button" :aria-label="collapsed ? text.expand : text.collapse" :title="collapsed ? text.expand : text.collapse" @click="toggleCollapsed"><AIcon name="collapse" /><span>{{ collapsed ? text.expand : text.collapse }}</span></button>
    </aside>
    <button class="mobile-backdrop" type="button" :aria-label="text.close" @click="closeMobile"></button>
    <main class="console-main" @click="noticeOpen = false; profileOpen = false"><slot><RouterView /></slot></main>
  </div>
</template>
