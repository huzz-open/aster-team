<script setup lang="ts">
import { computed, onUnmounted, ref, watch } from 'vue'
import { ACopyCode, AIcon, ASelect } from '@aster/ui'
import type { ReleaseCatalog } from '../shared/release-catalog'
import { buildInstallCommand, type InstallChoices } from './install-command'

const props = defineProps<{ locale: 'zh' | 'en'; catalog: ReleaseCatalog; catalogLive: boolean }>()
const choices = ref<InstallChoices>({ version: 'latest', email: '', protocol: 'http', host: '' })
const command = computed(() => buildInstallCommand(choices.value, props.catalog.releases.map(item => item.tag), window.location.origin))
const copyError = ref(false)
const copiedCommand = ref(false)
let copiedTimer: number | undefined
const copyLabel = computed(() => props.locale === 'zh' ? '复制命令' : 'Copy command')
const copiedLabel = computed(() => props.locale === 'zh' ? '已复制' : 'Copied')
const versionChoices = computed(() => [
  { value: 'latest', label: `${props.locale === 'zh' ? '最新版本' : 'Latest'}${props.catalogLive ? ` (${props.catalog.latest})` : ''}` },
  ...props.catalog.releases.map(item => ({ value: item.tag, label: item.tag })),
])
const protocolChoices = computed(() => [
  { value: 'http', label: `HTTP (${props.locale === 'zh' ? '默认' : 'default'})` },
  { value: 'https', label: 'HTTPS' },
])

function clearCopiedState() {
  if (copiedTimer) window.clearTimeout(copiedTimer)
  copiedTimer = undefined
  copiedCommand.value = false
}
watch(command, clearCopiedState)
function copyWithSelection(value: string): boolean {
  const field = document.createElement('textarea')
  const previousFocus = document.activeElement
  field.value = value
  field.setAttribute('readonly', '')
  field.style.position = 'fixed'
  field.style.opacity = '0'
  field.style.width = '1px'
  field.style.height = '1px'
  document.body.appendChild(field)
  try {
    field.focus()
    field.select()
    return document.execCommand('copy')
  } finally {
    field.remove()
    if (previousFocus instanceof HTMLElement) previousFocus.focus({ preventScroll: true })
  }
}
async function copyCommand() {
  if (!command.value) return
  try {
    let copied = false
    if (window.isSecureContext && navigator.clipboard?.writeText) {
      try {
        await navigator.clipboard.writeText(command.value)
        copied = true
      } catch { /* Try selection-based copying when clipboard access is denied. */ }
    }
    if (!copied && !copyWithSelection(command.value)) throw new Error('Clipboard copy unavailable')
    copyError.value = false
    clearCopiedState()
    copiedCommand.value = true
    copiedTimer = window.setTimeout(clearCopiedState, 3000)
  } catch {
    copyError.value = true
    clearCopiedState()
  }
}

onUnmounted(clearCopiedState)
</script>

<template>
  <div class="download-experience">
    <details class="command-options" open>
      <summary class="options-heading"><strong>{{ locale === 'zh' ? '自定义安装命令' : 'Customize the command' }}</strong><span class="options-chevron" aria-hidden="true"></span></summary>
      <div class="options-content"><div class="option-grid">
        <label class="option-field"><span>{{ locale === 'zh' ? '版本' : 'Version' }}</span><ASelect :model-value="choices.version" :options="versionChoices" :aria-label="locale === 'zh' ? '版本' : 'Version'" data-testid="install-version" @update:model-value="choices.version = String($event)" /></label>
        <label class="option-field"><span>{{ locale === 'zh' ? '管理员邮箱' : 'Admin email' }} <em>{{ locale === 'zh' ? '可选' : 'optional' }}</em></span><input v-model="choices.email" data-testid="install-email" type="email" autocomplete="email" placeholder="admin@example.com" /></label>
        <label class="option-field"><span>{{ locale === 'zh' ? '访问协议' : 'Access protocol' }}</span><ASelect :model-value="choices.protocol" :options="protocolChoices" :aria-label="locale === 'zh' ? '访问协议' : 'Access protocol'" data-testid="install-protocol" @update:model-value="choices.protocol = String($event) === 'https' ? 'https' : 'http'" /></label>
        <label class="option-field"><span>{{ locale === 'zh' ? '服务器域名或 IP' : 'Server domain or IP' }} <em>{{ locale === 'zh' ? '可选' : 'optional' }}</em></span><input v-model="choices.host" data-testid="install-host" type="text" autocomplete="off" :placeholder="locale === 'zh' ? '留空则自动检测' : 'Auto-detect if left blank'" /></label>
      </div>
      </div>
    </details>

    <div class="catalog-install">
      <ACopyCode :value="command ?? (locale === 'zh' ? '请修正邮箱或地址' : 'Correct the email or address')" :disabled="!command" :label="copyLabel" :copied-label="copiedLabel" :copy-state="copiedCommand ? 'copied' : 'idle'" copied-icon="check" @copy="copyCommand" />
      <small v-if="copyError" role="alert">{{ locale === 'zh' ? '复制失败，请手动复制命令。' : 'Copy failed. Please copy the command manually.' }}</small>
      <div class="install-facts"><span class="release-status"><i class="status-dot" aria-hidden="true"></i>{{ catalogLive ? (locale === 'zh' ? '最新稳定版' : 'Latest stable') : (locale === 'zh' ? '可用版本' : 'Available release') }} <strong>{{ catalog.latest }}</strong></span><span><img class="linux-icon" src="/assets/linux-tux.png" width="20" height="20" alt="" />Linux x86-64 <b aria-hidden="true">·</b> systemd</span><span><AIcon name="shield" :size="19" />SHA-256 {{ locale === 'zh' ? '安装前校验' : 'checked before install' }}</span></div>
    </div>

  </div>
</template>

<style scoped src="./LatestDownload.css"></style>
