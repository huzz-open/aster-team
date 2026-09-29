<script setup lang="ts">
import { useLicensedFeature } from '../license-status'
const canWrite = useLicensedFeature('member')
import { computed, onBeforeUnmount, onMounted, reactive, ref, useId, watch } from 'vue'
import { AButton, ACheckbox, AConfirmModal, AEmpty, AIconButton, AInfoTip, ALoadingState, AModal, APagination, ASelect, useToast } from '@aster/ui'
import { copyText, formatDate, formatNaturalTokenAmount, formatTokens, request, type AdminMemberOption, type Voucher } from '@aster/sdk'
import { locale } from '../i18n'

type GeneratedVoucher = Voucher & {
  code: string
  recipient?: AdminMemberOption | null
}

const items = ref<Voucher[]>([])
const page = ref(1)
const pageSize = ref(50)
const total = ref(0)
const toast = useToast()
const quotaInputId = useId()
const users = ref<AdminMemberOption[]>([])
const open = ref(false)
const generated = ref<GeneratedVoucher[]>([])
const deletingID = ref('')
const loading = ref(false)
const createSaving = ref(false)
const pendingRemoval = ref<Voucher | null>(null)
const keyword = ref('')
const status = ref('')
const delivery = ref('')
const validity = ref('')
let filterTimer: number | undefined
const form = reactive<{
  name: string
  quota_tokens: number
  max_redemptions: number
  valid_days: number | ''
  recipient_user_ids: string[]
}>({
  name: '',
  quota_tokens: 1_000_000,
  max_redemptions: 1,
  valid_days: '',
  recipient_user_ids: [],
})
const tx = (zh: string, en: string) => locale.value === 'en-US' ? en : zh
const number = (value: number) => formatTokens(value, locale.value)
const date = (value?: string) => formatDate(value, locale.value)
const statusChoices = computed(() => [
  { value: '', label: tx('全部状态', 'All statuses') },
  { value: 'active', label: tx('可兑换', 'Available') },
  { value: 'redeemed', label: tx('已领完', 'Fully claimed') },
  { value: 'expired', label: tx('已过期', 'Expired') },
])
const deliveryChoices = computed(() => [
  { value: '', label: tx('全部投放方式', 'All delivery types') },
  { value: 'public', label: tx('公开兑换券', 'Public vouchers') },
  { value: 'assigned', label: tx('指定成员', 'Assigned members') },
])
const validityChoices = computed(() => [
  { value: '', label: tx('全部有效期', 'All validity periods') },
  { value: 'permanent', label: tx('永久有效', 'Never expires') },
  { value: 'scheduled', label: tx('设置了到期时间', 'Has expiration') },
])

function query() {
  const params = new URLSearchParams({ limit: String(pageSize.value), offset: String((page.value - 1) * pageSize.value) })
  if (keyword.value.trim()) params.set('keyword', keyword.value.trim())
  if (status.value) params.set('status', status.value)
  if (delivery.value) params.set('delivery', delivery.value)
  if (validity.value) params.set('validity', validity.value)
  return params
}

async function load() {
  loading.value = true
  try {
    const [vouchers, members] = await Promise.all([
      request<{ items: Voucher[]; total: number }>(`/api/admin/vouchers?${query()}`),
      users.value.length ? Promise.resolve(null) : request<{ items: AdminMemberOption[] }>('/api/admin/vouchers/recipients'),
    ])
    items.value = vouchers.items; total.value = vouchers.total
    const maximumPage = Math.max(1, Math.ceil(total.value / pageSize.value))
    if (page.value > maximumPage) page.value = maximumPage
    if (members) users.value = members.items
  } catch (value) { toast.error(value instanceof Error ? value.message : tx('兑换券加载失败', 'Could not load vouchers')) }
  finally { loading.value = false }
}

async function create() {
  createSaving.value = true
  try {
    const result = await request<{ vouchers: GeneratedVoucher[] }>('/api/admin/vouchers', {
      method: 'POST',
      body: JSON.stringify({ ...form, valid_days: form.valid_days === '' ? null : form.valid_days }),
    })
    generated.value = result.vouchers
    page.value = 1
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('创建失败', 'Could not create vouchers'))
  } finally {
    createSaving.value = false
  }
}

async function remove(item: Voucher) {
  if (item.redeemed_count > 0) return
  deletingID.value = item.id
  try {
    await request(`/api/admin/vouchers/${encodeURIComponent(item.id)}`, { method: 'DELETE' })
    pendingRemoval.value = null
    toast.success(tx('兑换券已删除。', 'Voucher deleted.'))
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : tx('删除失败', 'Delete failed'))
  } finally {
    deletingID.value = ''
  }
}

function close() {
  open.value = false
  generated.value = []
  Object.assign(form, {
    name: '',
    quota_tokens: 1_000_000,
    max_redemptions: 1,
    valid_days: '',
    recipient_user_ids: [],
  })
}

async function copyAll() {
  const text = generated.value.map(item => `${item.recipient?.email ?? item.name}: ${item.code}`).join('\n')
  try { await copyText(text); toast.success(tx('兑换券已复制到剪贴板。', 'Vouchers copied to the clipboard.')) }
  catch (value) { toast.error(value instanceof Error ? value.message : tx('复制失败，请手动选择兑换券。', 'Copy failed. Select and copy the vouchers manually.')) }
}

function toggleAllRecipients() {
  form.recipient_user_ids = form.recipient_user_ids.length === users.value.length ? [] : users.value.map(user => user.id)
}

function statusText(item: Voucher) {
  if (item.status === 'active') return tx('可兑换', 'Available')
  if (item.status === 'expired') return tx('已过期', 'Expired')
  if (item.status === 'redeemed') return tx('已领完', 'Fully claimed')
  return item.status
}

function resetFilters() {
  const unchanged = !keyword.value && !status.value && !delivery.value && !validity.value
  keyword.value = ''; status.value = ''; delivery.value = ''; validity.value = ''
  if (unchanged) void load()
}

onMounted(load)
watch([keyword, status, delivery, validity], () => {
  window.clearTimeout(filterTimer)
  filterTimer = window.setTimeout(() => {
    if (page.value === 1) void load()
    else page.value = 1
  }, 200)
})
watch([page, pageSize], () => void load())
onBeforeUnmount(() => window.clearTimeout(filterTimer))
</script>

<template>
  <div class="content paginated-page">
    <header class="page-head">
      <div>
        <h1>{{ tx('兑换券', 'Vouchers') }}</h1>
      </div>
      <AButton icon="plus" @click="open = true" :disabled="!canWrite">{{ tx('创建 / 批量投放', 'Create / distribute') }}</AButton>
    </header>

    <div class="filter-bar voucher-filter">
      <label class="field search-field"><span>{{ tx('搜索', 'Search') }}</span><input v-model="keyword" :placeholder="tx('名称、券码前缀或成员邮箱', 'Name, code prefix, or member email')"></label>
      <label class="field"><span>{{ tx('状态', 'Status') }}</span><ASelect v-model="status" :options="statusChoices" :aria-label="tx('状态', 'Status')" /></label>
      <label class="field"><span>{{ tx('投放方式', 'Delivery type') }}</span><ASelect v-model="delivery" :options="deliveryChoices" :aria-label="tx('投放方式', 'Delivery type')" /></label>
      <label class="field"><span>{{ tx('有效期', 'Validity') }}</span><ASelect v-model="validity" :options="validityChoices" :aria-label="tx('有效期', 'Validity')" /></label>
      <AButton class="list-filter-action--reset" variant="secondary" @click="resetFilters">{{ tx('重置', 'Reset') }}</AButton>
    </div>

    <div class="table-wrap paginated-scroll">
      <ALoadingState v-if="loading && !items.length" :label="tx('正在读取兑换券…', 'Loading vouchers…')" />
      <table v-else-if="items.length" class="voucher-table flat-data-table">
        <thead><tr><th>{{ tx('名称', 'Name') }}</th><th>{{ tx('前缀', 'Prefix') }}</th><th>{{ tx('额度', 'Quota') }}</th><th>{{ tx('投放对象', 'Recipients') }}</th><th>{{ tx('投放人数', 'Recipients count') }}</th><th>{{ tx('已兑换', 'Redeemed') }}</th><th>{{ tx('兑换上限', 'Limit') }}</th><th>{{ tx('兑换成员', 'Redeemed by') }}</th><th>{{ tx('最近兑换', 'Last redeemed') }}</th><th>{{ tx('到期时间', 'Expires') }}</th><th>{{ tx('创建时间', 'Created') }}</th><th>{{ tx('状态', 'Status') }}</th><th>{{ tx('操作', 'Actions') }}</th></tr></thead>
        <tbody>
          <tr v-for="item in items" :key="item.id">
            <td><strong>{{ item.name }}</strong></td>
            <td class="code">{{ item.code_prefix }}…</td>
            <td>{{ number(item.quota_tokens) }}</td>
            <td>{{ item.delivery_count ? item.delivered_to_emails : tx('公开兑换券', 'Public voucher') }}</td>
            <td>{{ item.delivery_count || tx('不限成员', 'Any member') }}</td>
            <td><strong>{{ item.redeemed_count }}</strong></td>
            <td>{{ item.max_redemptions }}</td>
            <td>{{ item.redeemed_by_emails || '—' }}</td>
            <td>{{ date(item.last_redeemed_at) }}</td>
            <td>{{ item.expires_at ? date(item.expires_at) : tx('永久有效', 'Never expires') }}</td>
            <td>{{ date(item.created_at) }}</td>
            <td><span class="status" :class="{ off: item.status !== 'active' }">{{ statusText(item) }}</span></td>
            <td>
              <AIconButton v-if="item.redeemed_count === 0" icon="trash" size="small" variant="danger" :label="tx('删除兑换券', 'Delete voucher')" :disabled="!canWrite || deletingID === item.id" @click="pendingRemoval = item" />
              <span v-else :title="tx('已有兑换记录，需要保留额度账本审计链路', 'Redemption records must be retained for quota-ledger auditing')">{{ tx('需保留记录', 'Record retained') }}</span>
            </td>
          </tr>
        </tbody>
      </table>
      <AEmpty v-else :title="tx('尚未创建兑换券', 'No vouchers yet')" />
    </div>
    <APagination v-if="total > 0" v-model:page="page" v-model:page-size="pageSize" :total="total" :loading="loading" :locale="locale" />

    <AModal :open="open" :title="tx('创建兑换券', 'Create vouchers')" :description="tx('未选择成员时生成一张公开券；选择多个成员时，每人生成一张独立券。', 'With no recipients, one public voucher is generated. Selecting members creates a separate voucher for each.')" :close-label="tx('关闭', 'Close')" :close-disabled="createSaving" @close="close">
      <div v-if="generated.length" class="form">
        <div class="notice">{{ tx(`已生成 ${generated.length} 张兑换券。完整券码只在这里显示一次；已投放成员可直接在用户端点击领取。`, `${generated.length} vouchers generated. Full codes are shown only once; assigned members can claim them in the member console.`) }}</div>
        <div class="generated-vouchers">
          <div v-for="item in generated" :key="item.id" class="notice">
            <strong>{{ item.recipient ? `${item.recipient.display_name} · ${item.recipient.email}` : tx('公开兑换券', 'Public voucher') }}</strong>
            <div class="code">{{ item.code }}</div>
          </div>
        </div>
        <AButton icon="copy" @click="copyAll">{{ tx('复制全部兑换券', 'Copy all vouchers') }}</AButton>
        <AButton variant="secondary" @click="close">{{ tx('我已保存，关闭', 'Saved, close') }}</AButton>
      </div>
      <form v-else class="form" @submit.prevent="create">
        <label class="field"><span>{{ tx('用途名称', 'Purpose') }}</span><input v-model="form.name" :placeholder="tx('例如：研发组体验额度', 'Example: Engineering trial quota')" required></label>
        <label class="field" :for="quotaInputId"><span class="field-label field-label--split"><span>{{ tx('结算 Token 数量', 'Billed token amount') }}</span><output class="token-amount-hint">{{ formatNaturalTokenAmount(form.quota_tokens, locale) }}</output></span><input :id="quotaInputId" v-model.number="form.quota_tokens" type="number" min="1" max="10000000000" step="1" required></label>
        <label class="field"><span>{{ tx('兑换券有效期（天，可选）', 'Voucher validity (days, optional)') }}</span><input v-model.number="form.valid_days" type="number" min="1" max="3650" step="1" :placeholder="tx('留空表示永久有效', 'Leave blank to never expire')"><small>{{ tx('仅限制兑换券在创建后多少天内可以领取；领取后的 Token 不会因此过期。留空时兑换券永久有效。', 'This only limits how long the voucher can be claimed after creation. Claimed tokens do not expire. Leave blank for a voucher that never expires.') }}</small></label>
        <div class="field">
          <div class="recipient-head"><span class="field-label">{{ tx('投放给成员（可多选）', 'Recipients (multiple allowed)') }}<AInfoTip :text="tx('不选择时生成公开券；选择成员只控制可见性，不会自动增加余额。', 'Leave empty for a public voucher. Selecting members controls visibility and does not add quota automatically.')" /></span><button v-if="users.length" type="button" class="link-button" @click="toggleAllRecipients">{{ form.recipient_user_ids.length === users.length ? tx('清空', 'Clear') : tx('全选', 'Select all') }}</button></div>
          <div class="recipient-picker">
            <ACheckbox v-for="user in users" :key="user.id" v-model="form.recipient_user_ids" :value="user.id" :label="user.display_name" :description="user.email" />
            <p v-if="!users.length">{{ tx('还没有可投放的成员。', 'There are no eligible members.') }}</p>
          </div>
        </div>
        <label v-if="!form.recipient_user_ids.length" class="field"><span class="field-label">{{ tx('最多兑换人数', 'Maximum redemptions') }}<AInfoTip :text="tx('同一成员对同一张券只能兑换一次。', 'Each member can redeem a voucher only once.')" /></span><input v-model.number="form.max_redemptions" type="number" min="1" required></label>
        <div v-else class="notice">{{ tx(`将为 ${form.recipient_user_ids.length} 个成员分别生成一次性兑换券，互不抢占。`, `A separate one-time voucher will be generated for each of ${form.recipient_user_ids.length} members.`) }}</div>
        <div class="form-actions"><AButton variant="secondary" :disabled="createSaving" @click="close">{{ tx('取消', 'Cancel') }}</AButton><AButton type="submit" :loading="createSaving" :disabled="!canWrite">{{ tx('生成兑换券', 'Generate vouchers') }}</AButton></div>
      </form>
    </AModal>
    <AConfirmModal :open="!!pendingRemoval" :title="tx('删除兑换券', 'Delete voucher')" :text="tx(`删除“${pendingRemoval?.name || ''}”后，尚未领取的成员将无法再使用它。`, `Members who have not claimed “${pendingRemoval?.name || ''}” will no longer be able to use it.`)" :confirm-label="tx('确认删除', 'Delete voucher')" confirm-icon="trash" :cancel-label="tx('取消', 'Cancel')" danger :busy="!!deletingID" @close="pendingRemoval = null" @confirm="pendingRemoval && remove(pendingRemoval)" />
  </div>
</template>

<style scoped>
.voucher-filter { align-items: center; }
.voucher-filter .field { min-width: 130px; flex: 1 1 150px; }
.voucher-filter .search-field { min-width: 240px; flex: 1.5 1 320px; }
.voucher-filter > .a-button { margin-left: auto; }
.voucher-table { min-width: 1200px; }
.link-button:disabled { opacity: .55; cursor: wait; }
.generated-vouchers { display: grid; gap: 8px; max-height: 280px; overflow: auto; }
.generated-vouchers strong { display: block; margin-bottom: 6px; }
.recipient-head { display: flex; align-items: center; justify-content: space-between; }
.recipient-picker { display: grid; gap: 6px; max-height: 190px; padding: 8px; overflow: auto; border: 1px solid var(--line); border-radius: 10px; background: var(--surface-2); }
@media(max-width: 1040px) { .voucher-filter { flex-wrap: wrap; } }
@media(max-width: 760px) { .voucher-filter .field, .voucher-filter .search-field { min-width: 100%; flex-basis: 100%; } }
</style>
