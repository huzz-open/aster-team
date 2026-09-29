<script setup lang="ts">
import { computed, inject, onMounted, ref } from 'vue'
import { AButton, ACopyCode, AFilePicker, AIconButton, AInfoTip, ALoadingState, AModal, useToast } from '@aster/ui'
import { copyText, request } from '@aster/sdk'
import { locale } from '../i18n'
import { refreshLicenseProfileKey } from '../license-status'

type LicenseDocument = {
  schema: 'aster.admin-license-view.v1'
  protocol_schema: 'aster.license.v1' | 'aster.license.v2'
  license_id: string
  serial: string
  customer_ref: string | null
  key_id: string
  edition: string
  plan_id: string | null
  features: string[]
  quotas: { member_seats: number | null; runners: number | null; upstream_accounts: number | null; api_keys_per_member: number | null }
  binding: 'unbound' | 'installation'
  minimum_version: string
  transfer_sequence: number
  issued_at: string
  not_before: string
  expires_at: string | null
}
type LicenseStatus = {
  state: string
  license: LicenseDocument | null
  scheduled_license: LicenseDocument | null
  scheduled_state: 'none' | 'waiting' | 'ready' | 'version_too_old' | 'expired' | 'invalid' | 'unavailable'
  seat_usage: { occupied: number; licensed: number | null }
  online_runners: number
}
type LicensePreview = { activation: 'active' | 'scheduled'; license: LicenseDocument }

const requestCommand = 'pwd\nsudo aster-team-cli license request'
const fallbackInstallCommand = 'sudo aster-team-cli license install --source ./license.json'
const status = ref<LicenseStatus | null>(null)
const loadError = ref('')
const licenseFile = ref<File | null>(null)
const toast = useToast()
const refreshLicenseProfile = inject(refreshLicenseProfileKey, async () => {})
const loading = ref(false)
const installing = ref(false)
const previewing = ref(false)
const preview = ref<LicensePreview | null>(null)
const pendingSource = ref('')
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh

async function load() {
  loading.value = true
  status.value = null
  loadError.value = ''
  try {
    status.value = await request<LicenseStatus>('/api/admin/license')
    return status.value
  } catch (value) {
    loadError.value = value instanceof Error ? value.message : tx('授权状态读取失败', 'Could not load license status')
    return null
  }
  finally {
    await refreshLicenseProfile()
    loading.value = false
  }
}

async function copyCommand(value: string) {
  try {
    await copyText(value)
    toast.success(tx('命令已复制', 'Command copied'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('复制失败', 'Could not copy command'))
  }
}

async function copyLicenseIdentity() {
  if (!status.value?.license) return
  const { serial, license_id: licenseID } = status.value.license
  try {
    await copyText(`${serial} · ${licenseID}`)
    toast.success(tx('授权标识已复制', 'License identifiers copied'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('复制失败', 'Could not copy identifiers'))
  }
}

async function previewLicense() {
  if (installing.value || previewing.value || loading.value) return
  const file = licenseFile.value
  if (!file) {
    toast.error(tx('请先选择签名许可证 JSON 文件', 'Select the signed license JSON file first'))
    return
  }
  if (file.size === 0 || file.size > 64 * 1024) {
    toast.error(tx('许可证文件必须大于 0 字节且不超过 64 KiB', 'The license file must be between 1 byte and 64 KiB'))
    return
  }
  previewing.value = true
  try {
    const source = await file.text()
    const parsed = JSON.parse(source)
    if (!parsed || typeof parsed !== 'object' || Array.isArray(parsed)) throw new Error(tx('许可证 JSON 格式无效', 'The license JSON is invalid'))
    pendingSource.value = source
    preview.value = await request<LicensePreview>('/api/admin/license/preview', { method: 'POST', body: source })
  } catch (value) {
    pendingSource.value = ''
    preview.value = null
    toast.error(value instanceof Error ? value.message : tx('许可证校验失败', 'Could not verify the license'))
  } finally {
    previewing.value = false
  }
}

async function copyLicenseValue(value: string, label: string) {
  try {
    await copyText(value)
    toast.success(tx(`${label}已复制`, `${label} copied`))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('复制失败', 'Could not copy'))
  }
}

function closePreview() {
  if (installing.value) return
  preview.value = null
  pendingSource.value = ''
}

async function installLicense() {
  if (installing.value || loading.value || !preview.value || !pendingSource.value) return
  const intendedActivation = preview.value.activation
  installing.value = true
  try {
    const result = await request<{ state: string; activation: 'active' | 'scheduled' }>('/api/admin/license', { method: 'POST', body: pendingSource.value })
    licenseFile.value = null
    preview.value = null
    pendingSource.value = ''
    const current = await load()
    if (!current) {
      toast.warning(tx('许可证已保存 暂时无法确认当前授权状态 请重新读取', 'License saved; current status could not be confirmed. Reload the status.'))
    } else if (result.activation === 'scheduled' && current.scheduled_license) {
      toast.success(tx('续期授权已保存 将在签名生效时间自动切换', 'The renewal license is saved and will switch at its signed start time'))
    } else if (result.state === 'active' && current.state === 'active') {
      toast.success(intendedActivation === 'active' ? tx('已立即切换到新授权', 'Switched to the new license') : tx('许可证已更新 当前授权有效', 'License updated; the current license is active'))
    } else {
      toast.warning(tx('许可证已保存 请核对当前状态和有效期', 'License saved; check the current status and validity period'))
    }
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('许可证安装失败', 'Could not install the license'))
    await load()
  } finally {
    installing.value = false
  }
}

function format(value?: string | null) { return value ? new Date(value).toLocaleString(locale.value) : '—' }
function formatDate(value?: string | null) { return value ? new Date(value).toLocaleDateString(locale.value) : '—' }
function abbreviateIdentifier(value: string) {
  if (value.length <= 24) return value
  const timestampStart = value.search(/_\d{8}T/)
  const prefix = timestampStart > 0 ? value.slice(0, timestampStart) : value.split('_')[0]
  const suffix = value.split('_').at(-1) || value
  return `${prefix}_…${suffix.slice(-8)}`
}
function stateLabel(value?: string) {
  if (value === 'active') return tx('有效', 'Active')
  if (value === 'expired') return tx('已到期', 'Expired')
  if (value === 'missing') return tx('未安装', 'Missing')
  return tx('不可用', 'Unavailable')
}
function quotaLabel(value: number | null | undefined) {
  if (value === undefined) return '—'
  return value === null ? tx('不限', 'unlimited') : String(value)
}
function editionLabel(license?: LicenseDocument | null) {
  if (!license) return '—'
  if (license.edition === 'free') return tx('免费版', 'Free')
  if (license.edition === 'commercial') return tx('商业版', 'Commercial')
  return license.edition
}
function planLabel(license?: LicenseDocument | null) {
  return license?.plan_id || editionLabel(license)
}
function expiryRemainingLabel(value?: string | null) {
  if (!value) return tx('长期有效', 'No expiry')
  const remaining = new Date(value).getTime() - Date.now()
  if (remaining <= 0) return tx('已到期', 'Expired')
  const days = Math.ceil(remaining / 86_400_000)
  return tx(`剩余 ${days} 天`, `${days} days remaining`)
}
function expiryProgress(license?: LicenseDocument | null) {
  if (!license?.expires_at) return 100
  const start = new Date(license.not_before).getTime()
  const end = new Date(license.expires_at).getTime()
  if (!Number.isFinite(start) || !Number.isFinite(end) || end <= start) return 100
  return Math.min(100, Math.max(0, ((Date.now() - start) / (end - start)) * 100))
}
function quotaChange(current: number | null, next: number | null) {
  if (current === next) return 'same'
  if (next === null) return 'up'
  if (current === null) return 'down'
  return next > current ? 'up' : 'down'
}
const previewDifference = computed(() => {
  const next = preview.value?.license
  const current = status.value?.license
  if (!next || !current) return { kind: 'new', added: [] as string[], removed: [] as string[] }
  const added = next.features.filter(feature => !current.features.includes(feature))
  const removed = current.features.filter(feature => !next.features.includes(feature))
  const changes = [
    quotaChange(current.quotas.member_seats, next.quotas.member_seats),
    quotaChange(current.quotas.runners, next.quotas.runners),
    quotaChange(current.quotas.upstream_accounts, next.quotas.upstream_accounts),
    quotaChange(current.quotas.api_keys_per_member, next.quotas.api_keys_per_member),
  ]
  const improves = added.length > 0 || changes.includes('up')
  const reduces = removed.length > 0 || changes.includes('down')
  return { kind: improves && reduces ? 'mixed' : improves ? 'upgrade' : reduces ? 'downgrade' : 'same', added, removed }
})
function changeLabel(kind: string) {
  if (kind === 'upgrade') return tx('权益提升', 'Entitlements increase')
  if (kind === 'downgrade') return tx('权益减少', 'Entitlements decrease')
  if (kind === 'mixed') return tx('权益有增有减', 'Entitlements change')
  if (kind === 'new') return tx('首次启用', 'First activation')
  return tx('权益不变', 'Same entitlements')
}
onMounted(load)
</script>

<template>
  <div class="content license-page">
    <header class="page-head license-head">
      <div>
        <h1>{{ tx('产品授权', 'Product license') }}</h1>
      </div>
    </header>

    <div class="license-shell">

      <ALoadingState v-if="(loading || installing) && !status && !loadError" :label="installing ? tx('正在校验并导入许可证…', 'Verifying and importing license…') : tx('正在读取授权状态…', 'Loading license status…')" />

      <section v-if="loadError" class="card section recovery-card">
        <div role="alert">
          <h2>{{ tx('暂时无法读取授权状态', 'License status is unavailable') }}</h2>
          <p>{{ loadError }}</p>
          <p>{{ tx('可以重新读取状态或重试导入原许可证。无需清空数据或删除授权历史。', 'Reload the status or retry importing the original license. Do not clear data or license history.') }}</p>
        </div>
        <AButton variant="secondary" :loading="loading" :disabled="installing" @click="load">{{ tx('重新读取状态', 'Reload status') }}</AButton>
        <div class="upload-panel">
          <AFilePicker v-model="licenseFile" :label="tx('选择恢复许可证', 'Select recovery license')" :empty-label="tx('选择或拖入 license.json', 'Choose or drop license.json')" :hint="tx('仅支持 JSON，最大 64 KiB', 'JSON only, up to 64 KiB')" accept="application/json,.json" :loading="installing" />
          <AButton icon="upload" :loading="previewing" :disabled="!licenseFile || loading || installing" @click="previewLicense">{{ tx('校验许可证', 'Verify license') }}</AButton>
        </div>
      </section>

      <template v-if="status">
        <div class="license-dashboard" :class="{ 'is-unlicensed': !status.license }">
          <div class="license-main-column">
            <div class="license-overview-row">
              <section class="card section current-license-card" :class="{ 'is-active': status.state === 'active', 'is-empty': !status.license }">
                <h2>{{ tx('当前授权', 'Current license') }}</h2>
                <template v-if="status.license">
                  <div class="current-license-body">
                    <div class="license-state-icon" :class="{ 'is-inactive': status.state !== 'active' }" aria-hidden="true">
                      <svg v-if="status.state === 'active'" viewBox="0 0 24 24"><path d="m5 12.5 4.2 4.2L19 7" /></svg>
                      <span v-else>!</span>
                    </div>
                    <div class="current-license-copy">
                      <h3>{{ status.state === 'active' ? editionLabel(status.license) : stateLabel(status.state) }}</h3>
                      <dl class="current-license-meta">
                        <div><dt>{{ tx('套餐', 'Plan') }}</dt><dd class="license-identity" :title="planLabel(status.license)"><code>{{ planLabel(status.license) }}</code><AIconButton icon="copy" size="small" :label="tx('复制套餐标识', 'Copy plan ID')" @click="copyLicenseValue(planLabel(status.license), tx('套餐标识', 'Plan ID'))" /></dd></div>
                        <div><dt>{{ tx('授权编号', 'License') }}</dt><dd class="license-identity" :title="`${status.license.serial} · ${status.license.license_id}`"><code>{{ status.license.license_id }}</code><AIconButton icon="copy" size="small" :label="tx('复制完整授权标识', 'Copy full license identifiers')" @click="copyLicenseIdentity" /></dd></div>
                      </dl>
                    </div>
                  </div>
                </template>
                <div v-else class="empty-license-state">
                  <div class="license-state-icon is-inactive" aria-hidden="true">!</div>
                  <div>
                    <h3>{{ stateLabel(status.state) }}</h3>
                    <p>{{ tx('当前没有生效的产品授权。导入免费或商业许可证后即可开始使用。', 'No product license is active. Import a free or commercial license to get started.') }}</p>
                  </div>
                </div>
              </section>

              <section v-if="status.license" class="card section license-period-card" :class="{ 'is-expired': status.state === 'expired' }">
                <h2>{{ tx('授权周期', 'License period') }}</h2>
                <div class="period-summary">
                  <span class="period-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M7 3v3m10-3v3M4.5 9h15M6 5h12a2 2 0 0 1 2 2v12H4V7a2 2 0 0 1 2-2Zm2 8h3m-3 3h6" /></svg></span>
                  <div><strong>{{ expiryRemainingLabel(status.license.expires_at) }}</strong><span>{{ tx('到期', 'Expires') }} {{ format(status.license.expires_at) }}</span></div>
                </div>
                <div v-if="status.license.expires_at" class="license-period-track" :style="{ '--license-progress': `${expiryProgress(status.license)}%` }">
                  <div class="period-line"><span></span></div>
                  <div class="period-labels">
                    <span>{{ tx('生效', 'Started') }}<b>{{ formatDate(status.license.not_before) }}</b></span>
                    <span class="is-today">{{ tx('今天', 'Today') }}</span>
                    <span>{{ tx('到期', 'Expires') }}<b>{{ formatDate(status.license.expires_at) }}</b></span>
                  </div>
                </div>
              </section>
            </div>

            <section v-if="status.license" class="card section entitlement-card">
              <h2>{{ tx('授权权益', 'Entitlements') }}</h2>
              <div class="entitlement-grid">
                <div class="entitlement-item is-members">
                  <span class="entitlement-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><path d="M8.5 11a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Zm7-1a2.8 2.8 0 1 0 0-5.6M2.5 20v-2.2c0-2.7 2.4-4.8 6-4.8s6 2.1 6 4.8V20Zm13-6.2c3.6 0 6 1.8 6 4.4V20" /></svg></span>
                  <div><span>{{ tx('成员席位', 'Member seats') }}</span><strong>{{ status.seat_usage.occupied }} <small>/ {{ quotaLabel(status.license.quotas.member_seats) }}</small></strong></div>
                </div>
                <div class="entitlement-item is-runners">
                  <span class="entitlement-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><rect x="3" y="4" width="18" height="7" rx="2" /><rect x="3" y="13" width="18" height="7" rx="2" /><path d="M7 7.5h.01M7 16.5h.01M11 7.5h7M11 16.5h7" /></svg></span>
                  <div><span>Runner</span><strong>{{ status.online_runners }} <small>/ {{ quotaLabel(status.license.quotas.runners) }}</small></strong></div>
                </div>
                <div class="entitlement-item is-accounts">
                  <span class="entitlement-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><ellipse cx="12" cy="5" rx="7" ry="3" /><path d="M5 5v6c0 1.7 3.1 3 7 3s7-1.3 7-3V5M5 11v6c0 1.7 3.1 3 7 3s7-1.3 7-3v-6" /></svg></span>
                  <div><span>{{ tx('订阅/账号', 'Subscriptions') }}</span><strong>{{ quotaLabel(status.license.quotas.upstream_accounts) }}</strong></div>
                </div>
                <div class="entitlement-item is-keys">
                  <span class="entitlement-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><circle cx="8.5" cy="15.5" r="4.5" /><path d="m12 12 7.5-7.5M16 8l2 2m-4-4 2 2" /></svg></span>
                  <div><span>{{ tx('每人 Key', 'Keys / member') }}</span><strong>{{ quotaLabel(status.license.quotas.api_keys_per_member) }}</strong></div>
                </div>
              </div>
            </section>

            <div class="license-secondary-grid" :class="{ 'has-scheduled': !!status.scheduled_license }">
              <section v-if="status.scheduled_license" class="card section scheduled-card">
                <div class="scheduled-heading">
                  <span class="scheduled-icon" aria-hidden="true"><svg viewBox="0 0 24 24"><circle cx="12" cy="12" r="8.5" /><path d="M12 7v5l3.5 2" /></svg></span>
                  <div>
                    <span class="status-kicker">{{ tx('下一份授权', 'Next license') }}</span>
                    <h2>{{ status.scheduled_state === 'expired' ? tx('已过期且未生效', 'Expired before activation') : tx('已安排自动生效', 'Scheduled for activation') }}</h2>
                    <p class="scheduled-plan" :title="planLabel(status.scheduled_license)">{{ editionLabel(status.scheduled_license) }} · {{ abbreviateIdentifier(planLabel(status.scheduled_license)) }}</p>
                    <code class="scheduled-license-id" :title="status.scheduled_license.license_id">{{ abbreviateIdentifier(status.scheduled_license.license_id) }}</code>
                  </div>
                </div>
                <p v-if="status.scheduled_state === 'expired'">{{ tx('这份许可证已错过有效期，请重新导入新的许可证。', 'This license has passed its validity window. Import a new license.') }}</p>
                <p v-else-if="status.scheduled_state === 'version_too_old'">{{ tx('当前版本低于授权要求，升级软件后会按计划生效。', 'This Aster version is below the license requirement. Upgrade Aster before activation.') }}</p>
                <div class="activation-time"><span>{{ tx('生效时间', 'Activates') }}</span><strong>{{ format(status.scheduled_license.not_before) }}</strong></div>
                <div class="scheduled-entitlements">
                  <div><span>{{ tx('成员席位', 'Member seats') }}</span><strong>{{ quotaLabel(status.scheduled_license.quotas.member_seats) }}</strong></div>
                  <div><span>Runner</span><strong>{{ quotaLabel(status.scheduled_license.quotas.runners) }}</strong></div>
                  <div><span>{{ tx('订阅/账号', 'Subscriptions') }}</span><strong>{{ quotaLabel(status.scheduled_license.quotas.upstream_accounts) }}</strong></div>
                  <div><span>{{ tx('每人 Key', 'Keys / member') }}</span><strong>{{ quotaLabel(status.scheduled_license.quotas.api_keys_per_member) }}</strong></div>
                </div>
              </section>

              <section class="card section update-card">
                <div>
                  <span class="status-kicker">{{ status.license ? tx('授权更新', 'License update') : tx('授权激活', 'License activation') }}</span>
                  <div class="update-title">
                    <h2>{{ status.license ? tx('导入新授权', 'Import a new license') : tx('导入授权', 'Import a license') }}</h2>
                    <AInfoTip :text="tx('查看文件要求和服务器导入命令', 'View file requirements and the server import command')" :label="tx('导入说明', 'Import help')" tone="surface" :width="430" interactive>
                      <div class="license-import-tip">
                        <strong>{{ tx('导入说明', 'Import help') }}</strong>
                        <p>{{ tx('许可证文件仅支持 JSON，大小不超过 64 KiB。也可以把许可证保存为当前目录下的 license.json 后，在服务器执行：', 'License files must be JSON and no larger than 64 KiB. You can also save the license as license.json in the current directory and run:') }}</p>
                        <ACopyCode :value="fallbackInstallCommand" :label="tx('复制', 'Copy')" :copied-label="tx('已复制', 'Copied')" @copy="copyCommand(fallbackInstallCommand)" />
                      </div>
                    </AInfoTip>
                  </div>
                </div>
                <div class="upload-panel update-upload">
                  <AFilePicker v-model="licenseFile" :label="tx('选择新许可证', 'Select new license')" :empty-label="tx('选择或拖入 license.json', 'Choose or drop license.json')" accept="application/json,.json" :loading="previewing || installing" />
                  <AButton icon="upload" :loading="previewing" :disabled="!licenseFile || installing" @click="previewLicense">{{ status.license ? tx('校验并对比', 'Verify and compare') : tx('校验并安装', 'Verify and install') }}</AButton>
                </div>
              </section>
            </div>

          </div>

          <aside class="card section license-details-card" :class="{ 'license-setup-card': !status.license }">
            <template v-if="status.license">
              <h2>{{ tx('许可证详情', 'License details') }}</h2>
              <div class="feature-block">
                <span>{{ tx('功能权益', 'Features') }}</span>
                <div class="feature-tags"><b v-for="feature in status.license.features" :key="feature">{{ feature }}</b><b v-if="!status.license.features.length">—</b></div>
              </div>
              <dl class="license-detail-list">
                <div><dt>{{ tx('许可证', 'License') }}</dt><dd class="code"><span :title="status.license.license_id">{{ status.license.license_id }}</span><AIconButton icon="copy" size="small" :label="tx('复制许可证编号', 'Copy license ID')" @click="copyLicenseValue(status.license.license_id, tx('许可证编号', 'License ID'))" /></dd></div>
                <div><dt>{{ tx('序列号', 'Serial') }}</dt><dd class="code"><span :title="status.license.serial">{{ status.license.serial }}</span><AIconButton icon="copy" size="small" :label="tx('复制序列号', 'Copy serial')" @click="copyLicenseValue(status.license.serial, tx('序列号', 'Serial'))" /></dd></div>
                <div><dt>{{ tx('签名密钥', 'Signing key') }}</dt><dd class="code">{{ status.license.key_id }}</dd></div>
                <div><dt>{{ tx('签发时间', 'Issued') }}</dt><dd>{{ format(status.license.issued_at) }}</dd></div>
                <div><dt>{{ tx('生效时间', 'Valid from') }}</dt><dd>{{ format(status.license.not_before) }}</dd></div>
                <div><dt>{{ tx('到期时间', 'Expires') }}</dt><dd>{{ format(status.license.expires_at) }}</dd></div>
                <div><dt>{{ tx('最低版本', 'Minimum version') }}</dt><dd>{{ status.license.minimum_version }}</dd></div>
                <div><dt>{{ tx('换机序号', 'Transfer sequence') }}</dt><dd>{{ status.license.transfer_sequence }}</dd></div>
              </dl>
              <div v-if="status.state !== 'active'" class="notice overview-warning error">{{ tx('许可证当前不可用于业务请求，请导入新的有效许可证。', 'The license cannot currently authorize business requests. Import a new valid license.') }}</div>
            </template>
            <template v-else>
              <h2>{{ tx('获取授权', 'Get a license') }}</h2>
              <p class="license-setup-intro">{{ tx('免费版与商业版使用同一个导入入口。商业授权需要先生成当前服务器的机器申请。', 'Free and commercial licenses use the same import control. A commercial license first requires a machine request from this server.') }}</p>
              <div class="license-setup-options">
                <section>
                  <span class="setup-option-index">01</span>
                  <div><h3>{{ tx('免费授权', 'Free license') }}</h3><p>{{ tx('直接导入已签名的免费许可证，无需机器绑定。', 'Import a signed free license directly. No machine binding is required.') }}</p></div>
                </section>
                <section>
                  <span class="setup-option-index">02</span>
                  <div><h3>{{ tx('商业授权', 'Commercial license') }}</h3><p>{{ tx('生成机器申请并交给交付方，收到许可证后在左侧导入。', 'Generate a machine request for delivery, then import the issued license on the left.') }}</p></div>
                </section>
              </div>
              <ACopyCode :value="requestCommand" layout="block" :label="tx('复制申请命令', 'Copy request command')" :copied-label="tx('已复制', 'Copied')" @copy="copyCommand(requestCommand)" />
              <div v-if="status.scheduled_state === 'unavailable'" class="notice overview-warning error">{{ tx('预存许可证无法读取，请重新导入许可证。', 'The staged license cannot be read. Import the license again.') }}</div>
            </template>
          </aside>
        </div>

        <section v-if="status.license && !status.scheduled_license && status.scheduled_state === 'unavailable'" class="card section">
          <div class="notice error">{{ tx('预存许可证无法读取 当前许可证未受影响 请重新导入下一份许可证', 'The scheduled license cannot be read The current license is unaffected Import the next license again') }}</div>
        </section>

      </template>
    </div>

    <AModal :open="!!preview" :title="preview?.activation === 'active' ? tx('确认切换授权', 'Confirm license switch') : tx('确认续期授权', 'Confirm scheduled renewal')" :description="preview ? `${planLabel(preview.license)} · ${editionLabel(preview.license)}` : ''" :close-label="tx('取消', 'Cancel')" :close-disabled="installing" @close="closePreview">
      <div v-if="preview" class="license-preview">
        <div class="preview-result" :class="[`is-${preview.activation}`, `is-${previewDifference.kind}`]">
          <span>{{ changeLabel(previewDifference.kind) }}</span>
          <strong v-if="preview.activation === 'active'">{{ tx('确认后立即替换当前授权', 'Replaces the current license immediately after confirmation') }}</strong>
          <strong v-else>{{ tx(`将在 ${format(preview.license.not_before)} 自动切换`, `Switches automatically at ${format(preview.license.not_before)}`) }}</strong>
          <p v-if="preview.activation === 'scheduled'">{{ tx('许可证签名约定的生效时间尚未到达，因此不能提前启用。当前授权在此之前保持不变。', 'The signed start time has not arrived, so this license cannot be activated early. The current license remains unchanged until then.') }}</p>
          <p v-else-if="previewDifference.kind === 'downgrade' || previewDifference.kind === 'mixed'">{{ tx('新授权包含减少的权益，请确认当前资源仍符合新额度。系统不会自动删除数据。', 'Some entitlements are reduced. Confirm that current resources fit the new limits. Aster never deletes data automatically.') }}</p>
          <p v-else>{{ tx('签名、机器绑定、版本和有效期均已通过校验。', 'Signature, machine binding, version, and validity checks passed.') }}</p>
        </div>
        <div class="preview-comparison">
          <div class="comparison-head"><span>{{ tx('权益', 'Entitlement') }}</span><span>{{ tx('当前', 'Current') }}</span><span>{{ tx('新授权', 'New license') }}</span></div>
          <div><span>{{ tx('成员席位', 'Member seats') }}</span><strong>{{ quotaLabel(status?.license?.quotas.member_seats) }}</strong><strong>{{ quotaLabel(preview.license.quotas.member_seats) }}</strong></div>
          <div><span>Runner</span><strong>{{ quotaLabel(status?.license?.quotas.runners) }}</strong><strong>{{ quotaLabel(preview.license.quotas.runners) }}</strong></div>
          <div><span>{{ tx('订阅/账号', 'Subscriptions') }}</span><strong>{{ quotaLabel(status?.license?.quotas.upstream_accounts) }}</strong><strong>{{ quotaLabel(preview.license.quotas.upstream_accounts) }}</strong></div>
          <div><span>{{ tx('每人 Key', 'Keys / member') }}</span><strong>{{ quotaLabel(status?.license?.quotas.api_keys_per_member) }}</strong><strong>{{ quotaLabel(preview.license.quotas.api_keys_per_member) }}</strong></div>
          <div><span>{{ tx('到期时间', 'Expires') }}</span><strong>{{ format(status?.license?.expires_at) }}</strong><strong>{{ format(preview.license.expires_at) }}</strong></div>
        </div>
        <div v-if="previewDifference.added.length || previewDifference.removed.length" class="feature-difference">
          <p v-if="previewDifference.added.length"><span>+</span> {{ tx('新增功能', 'Added features') }}：{{ previewDifference.added.join(locale === 'zh-CN' ? '、' : ', ') }}</p>
          <p v-if="previewDifference.removed.length"><span>−</span> {{ tx('移除功能', 'Removed features') }}：{{ previewDifference.removed.join(locale === 'zh-CN' ? '、' : ', ') }}</p>
        </div>
        <div class="form-actions">
          <AButton variant="secondary" :disabled="installing" @click="closePreview">{{ tx('取消', 'Cancel') }}</AButton>
          <AButton :loading="installing" @click="installLicense">{{ preview.activation === 'active' ? tx('立即切换', 'Switch now') : tx('保存续期授权', 'Save renewal') }}</AButton>
        </div>
      </div>
    </AModal>
  </div>
</template>

<style scoped>
.license-page{min-height:calc(100vh - var(--header-height,64px))}.license-shell{width:min(100%,900px);margin:0 auto}.license-head{align-items:flex-start}.license-head p{margin-top:6px}.status-card{display:block;min-width:0;padding:18px 20px;background:linear-gradient(135deg,var(--surface),var(--surface-2))}.status-card.has-license{display:grid;grid-template-columns:minmax(220px,.72fr) minmax(0,1.28fr);align-items:center;gap:22px}.status-card.is-active{border-color:color-mix(in srgb,var(--positive) 30%,var(--line))}.status-summary{display:flex;align-items:center;min-width:0;gap:14px}.status-copy{min-width:0}.status-mark{display:grid;place-items:center;flex:0 0 40px;width:40px;height:40px;border-radius:12px;background:var(--warning-soft);color:var(--warning);font-size:var(--font-size-title);font-weight:900}.status-card.is-active .status-mark{background:var(--positive-soft);color:var(--positive)}.status-kicker{color:var(--muted);font-size:var(--font-size-caption)}.status-card h2{margin:2px 0 3px;font-size:var(--font-size-title)}.status-card p{font-size:var(--font-size-body)}.license-identity{display:flex;align-items:center;min-width:0;gap:5px;color:var(--text-soft);font-size:var(--font-size-caption);white-space:nowrap}.license-identity code{min-width:0;overflow:hidden;text-overflow:ellipsis}.license-identity .a-icon-button{flex:0 0 auto}.status-update{min-width:0;padding-left:22px;border-left:1px solid var(--line)}.status-update h3{margin:0;font-size:var(--font-size-body)}.status-update .upload-panel{margin-top:8px}.workflow-card{padding:22px}.workflow-head{margin-bottom:4px}.workflow-head p{margin-top:6px;color:var(--text-soft);line-height:1.65}.license-steps{display:grid;gap:0;margin:0;padding:0;list-style:none}.license-steps>li{display:grid;grid-template-columns:36px minmax(0,1fr);gap:14px;padding:20px 0;border-bottom:1px solid var(--line)}.license-steps>li:last-child{border-bottom:0}.step-number{display:grid;place-items:center;width:30px;height:30px;border-radius:50%;background:var(--accent-soft);color:var(--accent);font-weight:800}.step-body h3{margin:3px 0 7px;font-size:var(--font-size-body)}.step-body>p{max-width:760px;color:var(--text-soft);font-size:var(--font-size-body);line-height:1.7}.a-copy-code{margin-top:12px}.upload-panel{display:grid;grid-template-columns:minmax(0,1fr) auto;align-items:start;gap:12px;margin-top:13px}.cli-fallback{margin-top:4px;padding-top:16px;border-top:1px solid var(--line)}.cli-fallback summary{cursor:pointer;color:var(--text-soft);font-size:var(--font-size-body);font-weight:700}.cli-fallback p{margin-top:10px;font-size:var(--font-size-body)}.license-overview-grid,.license-detail-list{margin:0}.license-overview-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:0 24px}.license-overview-item{display:flex;align-items:center;justify-content:space-between;gap:20px;min-height:46px;border-bottom:1px solid var(--line);margin:0}.license-overview-item dt,.license-overview-item dd,.license-detail-list dt,.license-detail-list dd{margin:0}.license-overview-item dt,.license-detail-list dt{color:var(--muted)}.license-overview-item dd{font-weight:700}.usage-value{display:flex;align-items:baseline;gap:5px}.usage-value strong{font-size:var(--font-size-title)}.usage-value span{color:var(--muted);font-weight:500}.overview-warning{margin-top:12px}.license-detail-list>div{display:grid;grid-template-columns:112px minmax(0,1fr);align-items:center;gap:18px;min-height:46px;border-bottom:1px solid var(--line)}.license-overview-item:nth-last-child(-n+2),.license-detail-list>div:last-child{border-bottom:0}.license-detail-list dd{min-width:0}.license-detail-list .code{overflow-wrap:anywhere}@media(max-width:820px){.status-card.has-license{grid-template-columns:1fr;align-items:stretch}.status-update{padding-top:16px;padding-left:0;border-top:1px solid var(--line);border-left:0}}@media(max-width:760px){.workflow-card{padding:17px}.license-steps>li{grid-template-columns:30px minmax(0,1fr);gap:10px}.step-number{width:26px;height:26px}.upload-panel{grid-template-columns:1fr}.upload-panel>.a-button{width:100%}.a-copy-code{margin-top:12px}.license-overview-grid{grid-template-columns:1fr}.license-overview-item:nth-last-child(2){border-bottom:1px solid var(--line)}.license-detail-list>div{grid-template-columns:1fr;align-content:center;gap:4px;padding:8px 0}}
.license-shell{margin:0}.recovery-card{display:grid;gap:16px}.recovery-card h2{margin:0 0 8px;font-size:var(--font-size-title)}.recovery-card p{margin:8px 0;color:var(--text-soft);overflow-wrap:anywhere}.recovery-card>.a-button{justify-self:start}.recovery-card .upload-panel{margin:0}
.license-shell{width:100%;margin:0}.license-primary-grid{display:grid;grid-template-columns:minmax(0,1.35fr) minmax(360px,.85fr);gap:16px}.status-card{display:grid;align-content:start;gap:22px;min-height:280px;padding:24px;background:linear-gradient(145deg,var(--surface),color-mix(in srgb,var(--positive-soft) 28%,var(--surface)))}.license-card-head{display:flex;align-items:flex-start;justify-content:space-between;gap:20px}.license-state-pill{padding:5px 9px;border-radius:999px;background:var(--positive-soft);color:var(--positive);font-size:var(--font-size-caption);font-weight:800}.status-card h2,.update-card h2,.scheduled-card h2{margin:3px 0 5px;font-size:var(--font-size-title)}.status-card p,.update-card p,.scheduled-card p{margin:0;color:var(--text-soft);font-size:var(--font-size-body);line-height:1.6}.entitlement-grid{display:grid;grid-template-columns:repeat(4,minmax(0,1fr));border:1px solid var(--line);border-radius:12px;overflow:hidden}.entitlement-grid>div{display:grid;gap:7px;min-width:0;padding:14px;border-left:1px solid var(--line)}.entitlement-grid>div:first-child{border-left:0}.entitlement-grid span,.scheduled-entitlements span{color:var(--muted);font-size:var(--font-size-caption)}.entitlement-grid strong{font-size:var(--font-size-title)}.entitlement-grid small{color:var(--muted);font-size:var(--font-size-body);font-weight:500}.license-card-foot{display:flex;align-items:center;justify-content:space-between;gap:20px;margin-top:auto;color:var(--muted);font-size:var(--font-size-body)}.update-card{display:flex;flex-direction:column;justify-content:space-between;min-width:0;min-height:280px;padding:24px}.update-upload{grid-template-columns:1fr}.update-upload>.a-button{width:100%}.scheduled-card{padding:24px;border-color:color-mix(in srgb,var(--accent) 28%,var(--line));background:linear-gradient(135deg,var(--surface),color-mix(in srgb,var(--accent-soft) 34%,var(--surface)))}.scheduled-layout{display:grid;grid-template-columns:minmax(280px,.8fr) minmax(0,1.2fr);gap:36px}.scheduled-copy{display:flex;flex-direction:column;align-items:flex-start}.activation-time{display:grid;gap:5px;margin-top:auto;padding-top:22px}.activation-time span{color:var(--muted);font-size:var(--font-size-caption)}.activation-time strong{font-size:var(--font-size-title)}.scheduled-entitlements{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));overflow:hidden;border:1px solid var(--line);border-radius:12px;background:var(--surface)}.scheduled-entitlements>div{display:grid;align-content:center;gap:6px;min-height:78px;padding:13px;border-right:1px solid var(--line);border-bottom:1px solid var(--line)}.scheduled-entitlements>div:nth-child(3n){border-right:0}.scheduled-entitlements>div:nth-last-child(-n+3){border-bottom:0}.scheduled-entitlements strong{min-width:0;overflow-wrap:anywhere}.license-preview{display:grid;gap:18px}.preview-result{display:grid;gap:7px;padding:15px 17px;border:1px solid var(--line);border-radius:12px;background:var(--surface-2)}.preview-result>span{justify-self:start;padding:4px 8px;border-radius:999px;background:var(--accent-soft);color:var(--accent);font-size:var(--font-size-caption);font-weight:800}.preview-result>strong{font-size:var(--font-size-body)}.preview-result>p{margin:0;color:var(--text-soft);line-height:1.6}.preview-result.is-downgrade,.preview-result.is-mixed{border-color:color-mix(in srgb,var(--warning) 45%,var(--line))}.preview-result.is-downgrade>span,.preview-result.is-mixed>span{background:var(--warning-soft);color:var(--warning)}.preview-comparison{overflow:hidden;border:1px solid var(--line);border-radius:12px}.preview-comparison>div{display:grid;grid-template-columns:minmax(120px,1fr) minmax(110px,.8fr) minmax(110px,.8fr);gap:12px;min-height:42px;padding:10px 14px;border-top:1px solid var(--line)}.preview-comparison>div:first-child{border-top:0}.preview-comparison>div>*:not(:first-child){text-align:right}.preview-comparison .comparison-head{background:var(--surface-2);color:var(--muted);font-size:var(--font-size-caption);font-weight:700}.preview-comparison strong{min-width:0;overflow-wrap:anywhere}.feature-difference{display:grid;gap:6px}.feature-difference p{margin:0;color:var(--text-soft)}.feature-difference span{color:var(--accent);font-weight:900}.license-preview .form-actions{margin-top:0}.license-detail-list{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));column-gap:30px}.license-detail-list>div:nth-last-child(2){border-bottom:0}
@media(max-width:1120px){.license-primary-grid{grid-template-columns:1fr}.status-card,.update-card{min-height:auto}.scheduled-layout{grid-template-columns:1fr;gap:22px}.activation-time{margin-top:0}.license-detail-list{grid-template-columns:1fr}.license-detail-list>div:nth-last-child(2){border-bottom:1px solid var(--line)}}
@media(max-width:720px){.entitlement-grid{grid-template-columns:repeat(2,minmax(0,1fr))}.entitlement-grid>div:nth-child(3){border-left:0}.entitlement-grid>div:nth-child(-n+2){border-bottom:1px solid var(--line)}.license-card-foot{align-items:flex-start;flex-direction:column;gap:8px}.scheduled-entitlements{grid-template-columns:repeat(2,minmax(0,1fr))}.scheduled-entitlements>div:nth-child(3n){border-right:1px solid var(--line)}.scheduled-entitlements>div:nth-child(2n){border-right:0}.scheduled-entitlements>div:nth-last-child(-n+3){border-bottom:1px solid var(--line)}.scheduled-entitlements>div:nth-last-child(-n+2){border-bottom:0}.preview-comparison>div{grid-template-columns:minmax(92px,1fr) minmax(72px,.75fr) minmax(72px,.75fr);padding-inline:10px}.license-preview .form-actions{display:grid;grid-template-columns:1fr}.license-preview .form-actions>.a-button{width:100%}}
.license-plan-ref{display:grid;gap:2px;max-width:100%;margin-top:7px!important}.license-plan-ref span{color:var(--muted);font-size:var(--font-size-caption)}.license-plan-ref code{max-width:100%;overflow-wrap:anywhere;color:var(--text-soft);font-size:var(--font-size-caption);line-height:1.45}.scheduled-license-id{max-width:100%;margin-top:8px;overflow:hidden;text-overflow:ellipsis;color:var(--muted);font-size:var(--font-size-caption);white-space:nowrap}
.update-card{justify-content:flex-start;gap:18px;min-height:0}.update-title{display:flex;align-items:center;gap:6px}.update-title h2{margin-bottom:3px}.status-kicker,.entitlement-grid span,.scheduled-entitlements span,.activation-time span{font-size:var(--font-size-body)}.license-identity,.license-card-foot,.preview-result>span,.preview-comparison .comparison-head,.license-plan-ref span,.license-plan-ref code,.scheduled-license-id{font-size:var(--font-size-body)}.step-body>p,.cli-fallback summary,.cli-fallback p{font-size:var(--font-size-body)}
.license-primary-grid{display:block}.status-card{gap:0;min-height:0;background:var(--surface)}.current-license-label{display:block;margin-bottom:18px}.current-license-summary{display:grid;grid-template-columns:minmax(220px,.9fr) minmax(280px,1.15fr) minmax(300px,1fr);align-items:stretch}.license-state-block,.license-plan-block,.license-expiry-block{min-width:0;padding:8px 28px}.license-state-block{display:flex;align-items:center;gap:18px;padding-left:4px}.license-plan-block,.license-expiry-block{border-left:1px solid var(--line)}.license-state-block .status-mark{width:58px;height:58px;flex-basis:58px;border-radius:50%;font-size:var(--font-size-display)}.license-state-block h2,.license-plan-block h2{margin:0 0 8px;font-size:var(--font-size-title)}.license-state-block .license-state-pill{display:inline-flex;font-size:var(--font-size-body)}.license-plan-line{display:flex;align-items:baseline;gap:12px;color:var(--muted);font-size:var(--font-size-body)}.license-plan-line strong{min-width:0;overflow-wrap:anywhere;color:var(--text);font-size:var(--font-size-body)}.license-plan-block .license-identity{margin-top:9px}.license-expiry-block{display:grid;align-content:center;gap:7px}.license-expiry-block>span{color:var(--muted);font-size:var(--font-size-body)}.license-expiry-block>strong{font-size:var(--font-size-title);line-height:1.2}.license-expiry-block>em{color:var(--positive);font-size:var(--font-size-body);font-style:normal;font-weight:750}.license-expiry-block>em.is-expired{color:var(--danger)}.status-card .entitlement-grid{margin-top:24px}.empty-license-state{display:flex;align-items:center;gap:16px}.empty-license-state h2{margin:0 0 5px}.license-secondary-grid{display:grid;grid-template-columns:1fr;gap:16px;margin-top:16px}.license-secondary-grid.has-scheduled{grid-template-columns:minmax(0,1.45fr) minmax(360px,.85fr)}.license-secondary-grid .scheduled-card,.license-secondary-grid .update-card{margin:0}.license-secondary-grid .scheduled-layout{grid-template-columns:minmax(240px,.82fr) minmax(0,1.18fr);gap:24px}.scheduled-plan{color:var(--text)!important;font-size:var(--font-size-body)!important;font-weight:700}.license-secondary-grid .scheduled-entitlements{grid-template-columns:repeat(2,minmax(0,1fr))}.license-secondary-grid .scheduled-entitlements>div:nth-child(2n){border-right:0}.license-secondary-grid .scheduled-entitlements>div:nth-child(3){border-bottom:0}.license-secondary-grid .update-card{align-self:stretch}
@media(max-width:1180px){.current-license-summary{grid-template-columns:repeat(2,minmax(0,1fr))}.license-expiry-block{grid-column:1/-1;margin-top:18px;padding-top:22px;padding-left:4px;border-top:1px solid var(--line);border-left:0}.license-secondary-grid.has-scheduled{grid-template-columns:1fr}}
@media(max-width:760px){.current-license-summary{grid-template-columns:1fr}.license-state-block,.license-plan-block,.license-expiry-block{padding:18px 4px}.license-plan-block,.license-expiry-block{border-top:1px solid var(--line);border-left:0}.license-expiry-block{grid-column:auto;margin-top:0}.license-secondary-grid .scheduled-layout{grid-template-columns:1fr}}
.license-secondary-grid .scheduled-entitlements>div{border-right:1px solid var(--line);border-bottom:1px solid var(--line)}.license-secondary-grid .scheduled-entitlements>div:nth-child(2n){border-right:0}.license-secondary-grid .scheduled-entitlements>div:nth-child(-n+2){border-bottom:1px solid var(--line)}.license-secondary-grid .scheduled-entitlements>div:nth-child(n+3){border-bottom:0}

/* Confirmed wide license workspace */
.license-page {
  display: flex;
  flex-direction: column;
}
.license-page > .license-head {
  flex: 0 0 auto;
}
.license-shell {
  display: flex;
  flex: 1 1 auto;
  flex-direction: column;
  min-height: 0;
}
.license-dashboard {
  display: grid;
  grid-template-columns: minmax(0, 1.9fr) minmax(340px, .92fr);
  align-items: start;
  gap: 16px;
}
.license-main-column {
  display: grid;
  min-width: 0;
  gap: 16px;
}
.license-overview-row {
  display: grid;
  grid-template-columns: minmax(0, 1.08fr) minmax(0, .92fr);
  gap: 16px;
}
.license-overview-row > :only-child {
  grid-column: 1 / -1;
}
.current-license-card,
.license-period-card,
.entitlement-card,
.license-details-card {
  margin: 0;
  padding: 22px;
}
.current-license-card,
.license-period-card {
  min-height: 242px;
}
.current-license-card > h2,
.license-period-card > h2,
.entitlement-card > h2,
.license-details-card > h2 {
  margin: 0;
  color: var(--text);
  font-size:var(--font-size-title);
  line-height: 1.3;
}
.current-license-card.is-active {
  border-color: color-mix(in srgb, var(--positive) 22%, var(--line));
  background: linear-gradient(145deg, var(--surface) 55%, color-mix(in srgb, var(--positive-soft) 34%, var(--surface)));
}
.current-license-card.is-empty {
  min-height: 220px;
  border-color: color-mix(in srgb, var(--warning) 20%, var(--line));
  background: linear-gradient(145deg, var(--surface) 58%, color-mix(in srgb, var(--warning-soft) 36%, var(--surface)));
}
.current-license-body {
  display: flex;
  align-items: center;
  gap: 20px;
  min-width: 0;
  margin-top: 30px;
}
.license-state-icon,
.period-icon,
.entitlement-icon,
.scheduled-icon {
  display: grid;
  place-items: center;
  flex: 0 0 auto;
  border-radius: 50%;
}
.license-state-icon {
  width: 68px;
  height: 68px;
  background: color-mix(in srgb, var(--positive-soft) 78%, white);
  color: var(--positive);
}
.license-state-icon svg {
  width: 37px;
  height: 37px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 2.6;
}
.license-state-icon.is-inactive {
  background: var(--warning-soft);
  color: var(--warning);
  font-size:var(--font-size-display);
  font-weight: 900;
}
.current-license-copy {
  min-width: 0;
  flex: 1;
}
.current-license-copy h3 {
  margin: 0 0 18px;
  font-size:var(--font-size-display);
  line-height: 1.15;
}
.empty-license-state {
  max-width: 660px;
  margin-top: 30px;
}
.empty-license-state h3 {
  margin: 0 0 8px;
  font-size:var(--font-size-display);
}
.empty-license-state p {
  margin: 0;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  line-height: 1.65;
}
.current-license-meta {
  display: grid;
  gap: 10px;
  margin: 0;
}
.current-license-meta > div {
  display: grid;
  grid-template-columns: 84px minmax(0, 1fr);
  align-items: center;
  gap: 10px;
}
.current-license-meta dt,
.current-license-meta dd {
  margin: 0;
}
.current-license-meta dt {
  color: var(--muted);
  font-size:var(--font-size-body);
}
.current-license-meta dd {
  min-width: 0;
  overflow: hidden;
  color: var(--text);
  font-size:var(--font-size-body);
  font-weight: 700;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.current-license-meta .license-identity {
  font-size:var(--font-size-body);
}
.license-period-card {
  background: linear-gradient(145deg, var(--surface) 52%, color-mix(in srgb, #eaf9f2 64%, var(--surface)));
}
.period-summary {
  display: flex;
  align-items: center;
  gap: 18px;
  margin-top: 24px;
}
.period-icon {
  width: 58px;
  height: 58px;
  background: color-mix(in srgb, var(--positive-soft) 82%, white);
  color: var(--positive);
}
.period-icon svg,
.entitlement-icon svg,
.scheduled-icon svg {
  width: 28px;
  height: 28px;
  fill: none;
  stroke: currentColor;
  stroke-linecap: round;
  stroke-linejoin: round;
  stroke-width: 1.8;
}
.period-summary > div {
  display: grid;
  gap: 4px;
  min-width: 0;
}
.period-summary strong {
  color: var(--text);
  font-size:var(--font-size-display);
  line-height: 1;
  letter-spacing: -.025em;
}
.period-summary span {
  color: var(--text-soft);
  font-size:var(--font-size-body);
}
.license-period-card.is-expired .period-icon,
.license-period-card.is-expired .period-summary strong {
  color: var(--danger);
}
.license-period-track {
  margin-top: 30px;
}
.period-line {
  position: relative;
  height: 4px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--positive) 24%, var(--line));
}
.period-line::before {
  position: absolute;
  inset: 0 auto 0 0;
  width: var(--license-progress);
  border-radius: inherit;
  background: var(--positive);
  content: '';
}
.period-line::after {
  position: absolute;
  top: 50%;
  left: var(--license-progress);
  width: 12px;
  height: 12px;
  border: 3px solid var(--surface);
  border-radius: 50%;
  background: var(--positive);
  box-shadow: 0 0 0 2px var(--positive);
  content: '';
  transform: translate(-50%, -50%);
}
.period-labels {
  display: grid;
  grid-template-columns: 1fr auto 1fr;
  align-items: start;
  gap: 10px;
  margin-top: 12px;
  color: var(--muted);
  font-size:var(--font-size-caption);
}
.period-labels > span {
  display: grid;
  gap: 2px;
}
.period-labels > span:last-child {
  justify-items: end;
  text-align: right;
}
.period-labels b {
  color: var(--text-soft);
  font-size:var(--font-size-caption);
  font-weight: 600;
}
.period-labels .is-today {
  color: var(--positive);
  font-size:var(--font-size-body);
  font-weight: 800;
}
.entitlement-card {
  padding-bottom: 20px;
}
.entitlement-card .entitlement-grid {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin-top: 18px;
  border: 0;
  border-radius: 0;
  overflow: visible;
}
.entitlement-card .entitlement-item {
  display: flex;
  align-items: center;
  gap: 13px;
  min-width: 0;
  padding: 8px 16px;
  border: 0;
  border-left: 1px solid var(--line);
}
.entitlement-card .entitlement-item:first-child {
  padding-left: 0;
  border-left: 0;
}
.entitlement-icon {
  width: 46px;
  height: 46px;
}
.entitlement-item.is-members .entitlement-icon {
  background: #edf2ff;
  color: #3867e8;
}
.entitlement-item.is-runners .entitlement-icon {
  background: #f1edff;
  color: #7048e8;
}
.entitlement-item.is-accounts .entitlement-icon {
  background: #e8f8f1;
  color: #12a66b;
}
.entitlement-item.is-keys .entitlement-icon {
  background: #fff2df;
  color: #ed8a16;
}
.entitlement-item > div {
  display: grid;
  gap: 4px;
  min-width: 0;
}
.entitlement-card .entitlement-item span {
  color: var(--muted);
  font-size:var(--font-size-body);
}
.entitlement-card .entitlement-item strong {
  color: var(--text);
  font-size:var(--font-size-title);
  line-height: 1.1;
}
.entitlement-card .entitlement-item small {
  color: var(--muted);
  font-size:var(--font-size-body);
  font-weight: 600;
}
.license-secondary-grid,
.license-secondary-grid.has-scheduled {
  grid-template-columns: repeat(2, minmax(0, 1fr));
  margin: 0;
}
.license-secondary-grid:not(.has-scheduled) {
  grid-template-columns: minmax(0, 1fr);
}
.license-secondary-grid:not(.has-scheduled) .update-card {
  display: grid;
  grid-template-columns: minmax(210px, .36fr) minmax(0, 1fr);
  align-items: center;
  gap: 28px;
  width: 100%;
  min-height: 0;
}
.scheduled-card,
.update-card {
  min-height: 244px;
  padding: 22px;
}
.scheduled-card {
  border-color: color-mix(in srgb, #7666e8 25%, var(--line));
  background: linear-gradient(145deg, var(--surface) 54%, #f6f3ff);
}
.scheduled-heading {
  display: flex;
  align-items: center;
  gap: 14px;
}
.scheduled-icon {
  width: 52px;
  height: 52px;
  background: #f0ecff;
  color: #6853e8;
}
.scheduled-heading h2 {
  margin: 2px 0 3px;
}
.scheduled-card .activation-time {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 14px;
  margin-top: 18px;
  padding: 13px 0;
  border-top: 1px solid var(--line);
}
.scheduled-card .activation-time strong {
  font-size:var(--font-size-body);
}
.scheduled-card .scheduled-entitlements {
  grid-template-columns: repeat(4, minmax(0, 1fr));
  margin-top: 2px;
  border: 0;
  background: transparent;
}
.license-secondary-grid .scheduled-entitlements > div {
  min-height: 56px;
  padding: 8px 10px;
  border-top: 0;
  border-right: 1px solid var(--line);
  border-bottom: 0;
}
.license-secondary-grid .scheduled-entitlements > div:first-child {
  padding-left: 0;
}
.license-secondary-grid .scheduled-entitlements > div:last-child {
  border-right: 0;
}
.update-card {
  justify-content: space-between;
  gap: 20px;
}
.update-upload {
  grid-template-columns: 1fr;
  margin-top: 0;
}
.license-secondary-grid:not(.has-scheduled) .update-upload {
  grid-template-columns: minmax(0, 1fr) auto;
  align-items: start;
}
.license-secondary-grid:not(.has-scheduled) .update-upload > .a-button {
  width: auto;
  min-height: var(--control-height);
}
.license-import-tip {
  display: grid;
  gap: 12px;
  min-width: 0;
  padding: 2px;
}
.license-import-tip > strong {
  font-size:var(--font-size-body);
}
.license-import-tip > p {
  margin: 0;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  line-height: 1.65;
}
.license-import-tip :deep(.a-copy-code) {
  margin-top: 0;
}
.license-details-card {
  position: sticky;
  top: calc(var(--header-height, 64px) + 18px);
  min-width: 0;
  background: linear-gradient(160deg, var(--surface) 68%, color-mix(in srgb, var(--accent-soft) 22%, var(--surface)));
}
.feature-block {
  display: grid;
  gap: 12px;
  margin-top: 27px;
  padding-bottom: 22px;
  border-bottom: 1px solid var(--line);
}
.feature-block > span {
  color: var(--muted);
  font-size:var(--font-size-body);
}
.feature-tags {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
.feature-tags b {
  padding: 8px 12px;
  border-radius: 9px;
  background: color-mix(in srgb, var(--accent-soft) 72%, white);
  color: var(--accent);
  font-size:var(--font-size-body);
}
.license-details-card .license-detail-list {
  display: block;
  margin-top: 6px;
}
.license-details-card .license-detail-list > div {
  display: grid;
  grid-template-columns: 112px minmax(0, 1fr);
  align-items: center;
  gap: 14px;
  min-height: 60px;
  border-bottom: 1px solid var(--line);
}
.license-details-card .license-detail-list > div:last-child {
  border-bottom: 0;
}
.license-details-card .license-detail-list dt {
  color: var(--muted);
  font-size:var(--font-size-body);
}
.license-details-card .license-detail-list dd {
  min-width: 0;
  color: var(--text);
  font-size:var(--font-size-body);
  font-weight: 650;
  overflow-wrap: anywhere;
}
.license-details-card .license-detail-list dd.code {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 6px;
}
.license-details-card .license-detail-list dd.code > span {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.license-setup-card {
  display: grid;
  align-content: start;
  gap: 20px;
  min-height: 0;
  background: linear-gradient(160deg, var(--surface) 62%, color-mix(in srgb, var(--accent-soft) 28%, var(--surface)));
}
.license-setup-intro {
  margin: 0;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  line-height: 1.7;
}
.license-setup-options {
  display: grid;
  border-top: 1px solid var(--line);
}
.license-setup-options section {
  display: grid;
  grid-template-columns: 34px minmax(0, 1fr);
  gap: 12px;
  padding: 17px 0;
  border-bottom: 1px solid var(--line);
}
.setup-option-index {
  color: var(--accent);
  font-size:var(--font-size-body);
  font-weight: 800;
  letter-spacing: .06em;
}
.license-setup-options h3 {
  margin: 0 0 5px;
  font-size:var(--font-size-body);
}
.license-setup-options p {
  margin: 0;
  color: var(--text-soft);
  font-size:var(--font-size-body);
  line-height: 1.55;
}
.license-setup-card :deep(.a-copy-code) {
  margin-top: 0;
}
@media (max-width: 1420px) {
  .license-dashboard {
    grid-template-columns: minmax(0, 1fr) 340px;
  }
  .license-overview-row,
  .license-secondary-grid,
  .license-secondary-grid.has-scheduled {
    grid-template-columns: 1fr;
  }
  .current-license-card,
  .license-period-card,
  .scheduled-card,
  .update-card {
    min-height: 0;
  }
  .entitlement-card .entitlement-grid {
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 16px 0;
  }
  .entitlement-card .entitlement-item:nth-child(3) {
    padding-left: 0;
    border-left: 0;
  }
}
@media (max-width: 1040px) {
  .license-dashboard {
    grid-template-columns: 1fr;
  }
  .license-details-card {
    position: static;
  }
}
@media (max-width: 680px) {
  .current-license-card,
  .license-period-card,
  .entitlement-card,
  .license-details-card,
  .scheduled-card,
  .update-card {
    padding: 18px;
  }
  .current-license-body {
    align-items: flex-start;
    margin-top: 22px;
  }
  .license-state-icon {
    width: 54px;
    height: 54px;
  }
  .current-license-copy h3 {
    font-size:var(--font-size-title);
  }
  .entitlement-card .entitlement-grid {
    grid-template-columns: 1fr;
  }
  .entitlement-card .entitlement-item,
  .entitlement-card .entitlement-item:nth-child(3) {
    padding: 10px 0;
    border-top: 1px solid var(--line);
    border-left: 0;
  }
  .entitlement-card .entitlement-item:first-child {
    border-top: 0;
  }
  .scheduled-card .scheduled-entitlements {
    grid-template-columns: repeat(2, minmax(0, 1fr));
  }
  .license-secondary-grid:not(.has-scheduled) .update-card,
  .license-secondary-grid:not(.has-scheduled) .update-upload {
    grid-template-columns: 1fr;
  }
  .license-secondary-grid .scheduled-entitlements > div:nth-child(2) {
    border-right: 0;
  }
  .license-details-card .license-detail-list > div {
    grid-template-columns: 1fr;
    gap: 5px;
    padding: 11px 0;
  }
}
</style>
