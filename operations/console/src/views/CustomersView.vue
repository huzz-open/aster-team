<script setup lang="ts">
import { computed, onMounted, reactive, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, ACheckbox, AEmpty, ALoadingState, APagination, ASelect, useToast } from '@aster/ui'
import {
  createContact,
  createCustomer,
  getCustomerProfile,
  listCustomers,
  updateCustomer,
  upsertBillingProfile,
  type BillingProfileInput,
  type ContactInput,
  type Customer,
  type CustomerInput,
  type CustomerProfile,
} from '../api/client'
import { presentationLabel } from '../presentation'

type DetailTab = 'enterprise' | 'contacts' | 'billing'

const toast = useToast()
const route = useRoute()
const router = useRouter()
const customers = ref<Customer[]>([])
const profile = ref<CustomerProfile | null>(null)
const selectedId = ref('')
const query = ref('')
const page = ref(1)
const hasNext = ref(false)
const cursors = ref<Record<number, string>>({ 1: '' })
const loading = ref(false)
const saving = ref<'create' | 'customer' | 'contact' | 'billing' | ''>('')
const creating = ref(false)
const activeTab = ref<DetailTab>('enterprise')
let detailRequest = 0
let profileLoadingID = ''

const blankCustomer = (): CustomerInput => ({
  name: '', legal_name: '', status: 'lead', contact_name: '', contact_email: '',
  contact_phone: '', contact_wechat: '', notes: '',
})
const form = reactive<CustomerInput>(blankCustomer())
const edit = reactive<CustomerInput>(blankCustomer())
const contact = reactive<ContactInput>({ name: '', email: '', phone: '', wechat: '', role_title: '', is_primary: false })
const billing = reactive<BillingProfileInput>({ invoice_title: '', tax_identifier: '', billing_email: '', address: '' })

const customerStatusOptions = [
  { value: 'lead', label: '线索' },
  { value: 'active', label: '正式客户' },
  { value: 'inactive', label: '停用' },
]

const filteredCustomers = computed(() => {
  const keyword = query.value.trim().toLowerCase()
  if (!keyword) return customers.value
  return customers.value.filter((customer) =>
    [customer.name, customer.legal_name, customer.contact_name, customer.contact_email]
      .some((value) => value?.toLowerCase().includes(keyword)),
  )
})

async function load() {
  loading.value = true
  try {
    const result = await listCustomers(cursors.value[page.value] || '')
    customers.value = result.items
    hasNext.value = Boolean(result.next)
    if (result.next) cursors.value[page.value + 1] = result.next
    else delete cursors.value[page.value + 1]
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '读取客户失败')
  } finally {
    loading.value = false
  }
}

function startCreate() {
  detailRequest++
  profileLoadingID = ''
  creating.value = true
  selectedId.value = ''
  profile.value = null
  activeTab.value = 'enterprise'
  Object.assign(form, blankCustomer())
  writeLocation({ mode: 'new' })
}

function writeLocation(next: { customer?: string; tab?: DetailTab; mode?: 'new' }, method: 'push' | 'replace' = 'push') {
  const query = { ...route.query }
  delete query.customer; delete query.tab; delete query.mode
  if (next.customer) query.customer = next.customer
  if (next.tab && next.tab !== 'enterprise') query.tab = next.tab
  if (next.mode) query.mode = next.mode
  if (route.query.customer === query.customer && route.query.tab === query.tab && route.query.mode === query.mode) return
  void router[method]({ path: route.path, query })
}

function cancelCreate() {
  creating.value = false
  writeLocation({})
}

function selectTab(tab: DetailTab) {
  activeTab.value = tab
  if (selectedId.value) writeLocation({ customer: selectedId.value, tab })
}

async function submit() {
  saving.value = 'create'
  try {
    const created = await createCustomer(form)
    Object.assign(form, blankCustomer())
    creating.value = false
    if (page.value !== 1) page.value = 1
    else await load()
    await manage(created)
    toast.success('客户创建成功')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '创建客户失败')
  } finally {
    saving.value = ''
  }
}

async function manageID(id: string, updateUrl = true) {
  const request = ++detailRequest
  creating.value = false
  selectedId.value = id
  activeTab.value = 'enterprise'
  profile.value = null
  profileLoadingID = id
  if (updateUrl) writeLocation({ customer: id })
  try {
    const result = await getCustomerProfile(id)
    if (request !== detailRequest) return
    profile.value = result
    Object.assign(edit, profile.value.customer)
    Object.assign(billing, profile.value.billing_profile || { invoice_title: '', tax_identifier: '', billing_email: '', address: '' })
    if (!updateUrl) {
      const tab = route.query.tab
      activeTab.value = tab === 'contacts' || tab === 'billing' ? tab : 'enterprise'
    }
  } catch (value) {
    if (request === detailRequest) toast.error(value instanceof Error ? value.message : '读取客户档案失败')
  } finally {
    if (request === detailRequest) profileLoadingID = ''
  }
}

function manage(customer: Customer) { return manageID(customer.id) }

async function restoreLocation() {
  if (route.query.mode === 'new') {
    if (!creating.value) startCreate()
    return
  }
  const id = typeof route.query.customer === 'string' ? route.query.customer : ''
  if (!id) { detailRequest++; profileLoadingID = ''; creating.value = false; selectedId.value = ''; profile.value = null; return }
  if (profileLoadingID === id) return
  if (selectedId.value !== id || !profile.value) await manageID(id, false)
  const tab = route.query.tab
  activeTab.value = tab === 'contacts' || tab === 'billing' ? tab : 'enterprise'
}

async function saveCustomer() {
  if (!profile.value) return
  saving.value = 'customer'
  try {
    const updated = await updateCustomer(profile.value.customer.id, edit)
    profile.value.customer = updated
    customers.value = customers.value.map((item) => item.id === updated.id ? updated : item)
    toast.success('企业档案已更新')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '更新客户失败')
  } finally {
    saving.value = ''
  }
}

async function addContact() {
  if (!profile.value) return
  saving.value = 'contact'
  try {
    const created = await createContact(profile.value.customer.id, contact)
    if (created.is_primary) profile.value.contacts = profile.value.contacts.map((item) => ({ ...item, is_primary: false }))
    profile.value.contacts.push(created)
    Object.assign(contact, { name: '', email: '', phone: '', wechat: '', role_title: '', is_primary: false })
    toast.success('联系人已新增')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '新增联系人失败')
  } finally {
    saving.value = ''
  }
}

async function saveBilling() {
  if (!profile.value) return
  saving.value = 'billing'
  try {
    profile.value.billing_profile = await upsertBillingProfile(profile.value.customer.id, billing)
    toast.success('开票资料已保存')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : '保存开票资料失败')
  } finally {
    saving.value = ''
  }
}

onMounted(() => { void load(); void restoreLocation() })
watch(page, () => void load())
watch(() => [route.query.customer, route.query.tab, route.query.mode], () => { void restoreLocation() })
</script>

<template>
  <section class="content master-page">
    <div class="page-head">
      <h1>客户资料</h1>
      <AButton icon="plus" @click="startCreate">新增客户</AButton>
    </div>

    <div class="master-detail">
      <aside class="customer-list">
        <label class="search-field">
          <span class="sr-only">搜索客户</span>
          <input v-model="query" type="search" placeholder="搜索客户、联系人或邮箱">
        </label>

        <div class="customer-items">
          <ALoadingState v-if="loading && !customers.length" label="正在读取客户…" />
          <button
            v-for="customer in filteredCustomers"
            v-else
            :key="customer.id"
            type="button"
            :class="{ active: selectedId === customer.id }"
            @click="manage(customer)"
          >
            <span class="customer-row-head">
              <strong>{{ customer.name }}</strong>
              <span class="status" :class="{ off: customer.status === 'inactive', warning: customer.status === 'lead' }">
                {{ presentationLabel('customerStatus', customer.status) }}
              </span>
            </span>
            <span>{{ customer.legal_name || '未填写企业全称' }}</span>
            <span>{{ customer.contact_name || '—' }} · {{ customer.contact_email || '—' }}</span>
          </button>
          <AEmpty v-if="!loading && !filteredCustomers.length" title="暂无客户" />
        </div>

        <APagination v-if="customers.length || page > 1" v-model:page="page" :has-next="hasNext" :loading="loading" />
      </aside>

      <main class="detail-panel">
        <form v-if="creating" class="compact-form" @submit.prevent="submit">
          <div class="detail-title"><h2>新增客户</h2></div>
          <div class="form-grid">
            <label class="field"><span>客户简称</span><input v-model="form.name" required minlength="2" maxlength="160"></label>
            <label class="field"><span>企业全称</span><input v-model="form.legal_name" maxlength="200"></label>
            <label class="field"><span>状态</span><ASelect v-model="form.status" :options="customerStatusOptions" aria-label="客户状态" /></label>
            <label class="field"><span>主联系人</span><input v-model="form.contact_name" maxlength="120"></label>
            <label class="field"><span>联系邮箱</span><input v-model="form.contact_email" type="email" maxlength="320"></label>
            <label class="field"><span>电话</span><input v-model="form.contact_phone" maxlength="64"></label>
            <label class="field"><span>微信</span><input v-model="form.contact_wechat" maxlength="120"></label>
            <label class="field span-2"><span>内部备注</span><textarea v-model="form.notes" maxlength="4000"></textarea></label>
          </div>
          <div class="detail-actions">
            <AButton variant="secondary" type="button" @click="cancelCreate">取消</AButton>
            <AButton type="submit" :loading="saving === 'create'">保存客户</AButton>
          </div>
        </form>

        <template v-else-if="profile">
          <div class="detail-title">
            <div><h2>{{ profile.customer.name }}</h2><span>{{ profile.customer.legal_name || '—' }}</span></div>
            <span class="status" :class="{ off: profile.customer.status === 'inactive', warning: profile.customer.status === 'lead' }">
              {{ presentationLabel('customerStatus', profile.customer.status) }}
            </span>
          </div>
          <nav class="detail-tabs" aria-label="客户资料分类">
            <button type="button" :class="{ active: activeTab === 'enterprise' }" @click="selectTab('enterprise')">企业信息</button>
            <button type="button" :class="{ active: activeTab === 'contacts' }" @click="selectTab('contacts')">联系人 {{ profile.contacts.length }}</button>
            <button type="button" :class="{ active: activeTab === 'billing' }" @click="selectTab('billing')">开票资料</button>
          </nav>

          <form v-if="activeTab === 'enterprise'" class="compact-form" @submit.prevent="saveCustomer">
            <div class="form-grid">
              <label class="field"><span>客户简称</span><input v-model="edit.name" required minlength="2"></label>
              <label class="field"><span>企业全称</span><input v-model="edit.legal_name"></label>
              <label class="field"><span>状态</span><ASelect v-model="edit.status" :options="customerStatusOptions" aria-label="客户状态" /></label>
              <label class="field"><span>主联系人</span><input v-model="edit.contact_name"></label>
              <label class="field"><span>邮箱</span><input v-model="edit.contact_email" type="email"></label>
              <label class="field"><span>电话</span><input v-model="edit.contact_phone"></label>
              <label class="field"><span>微信</span><input v-model="edit.contact_wechat"></label>
              <label class="field span-2"><span>合同 / 支持备注</span><textarea v-model="edit.notes"></textarea></label>
            </div>
            <div class="detail-actions"><AButton type="submit" :loading="saving === 'customer'">保存变更</AButton></div>
          </form>

          <div v-else-if="activeTab === 'contacts'" class="contacts-layout">
            <div class="contact-cards">
              <article v-for="item in profile.contacts" :key="item.id">
                <div><strong>{{ item.name }}</strong><span v-if="item.is_primary" class="status">主联系人</span></div>
                <span>{{ item.role_title || '—' }}</span>
                <span>{{ item.email || '—' }} · {{ item.phone || '—' }} · {{ item.wechat || '—' }}</span>
              </article>
              <AEmpty v-if="!profile.contacts.length" title="暂无联系人" />
            </div>
            <form class="contact-form" @submit.prevent="addContact">
              <h3>新增联系人</h3>
              <div class="form-grid">
                <label class="field"><span>姓名</span><input v-model="contact.name" required></label>
                <label class="field"><span>职务</span><input v-model="contact.role_title"></label>
                <label class="field"><span>邮箱</span><input v-model="contact.email" type="email"></label>
                <label class="field"><span>电话</span><input v-model="contact.phone"></label>
                <label class="field"><span>微信</span><input v-model="contact.wechat"></label>
                <ACheckbox v-model="contact.is_primary" label="设为主联系人" />
              </div>
              <div class="detail-actions"><AButton type="submit" :loading="saving === 'contact'">添加联系人</AButton></div>
            </form>
          </div>

          <form v-else class="compact-form" @submit.prevent="saveBilling">
            <div class="form-grid">
              <label class="field"><span>发票抬头</span><input v-model="billing.invoice_title" required></label>
              <label class="field"><span>税号</span><input v-model="billing.tax_identifier"></label>
              <label class="field"><span>收票邮箱</span><input v-model="billing.billing_email" type="email"></label>
              <label class="field span-2"><span>注册地址 / 邮寄地址</span><textarea v-model="billing.address"></textarea></label>
            </div>
            <div class="detail-actions"><AButton type="submit" :loading="saving === 'billing'">保存开票资料</AButton></div>
          </form>
        </template>

        <AEmpty v-else title="请选择客户" />
      </main>
    </div>
  </section>
</template>

<style scoped>
.master-page{display:flex;min-height:0;flex-direction:column}.page-head{display:flex;align-items:center;justify-content:space-between;margin-bottom:16px}.page-head h1{margin:0}.master-detail{display:grid;min-height:0;flex:1;grid-template-columns:360px minmax(0,1fr);gap:14px}.customer-list,.detail-panel{min-height:0;border:1px solid var(--line);border-radius:14px;background:var(--surface)}.customer-list{display:grid;grid-template-rows:auto minmax(0,1fr) auto;padding:12px}.search-field input{width:100%}.customer-items{min-height:0;overflow:auto;padding:10px 2px}.customer-items>button{display:grid;width:100%;gap:7px;border:1px solid transparent;border-radius:11px;padding:13px;background:transparent;color:inherit;text-align:left}.customer-items>button:hover{background:var(--surface-soft)}.customer-items>button.active{border-color:var(--accent);background:var(--accent-soft)}.customer-items>button>span:not(.customer-row-head){overflow:hidden;color:var(--muted);font-size:var(--font-size-body);text-overflow:ellipsis;white-space:nowrap}.customer-row-head{display:flex;align-items:center;justify-content:space-between;gap:10px}.detail-panel{display:flex;flex-direction:column;padding:22px 24px}.detail-title{display:flex;min-height:48px;align-items:flex-start;justify-content:space-between;gap:16px}.detail-title h2{margin:0}.detail-title>div{display:grid;gap:4px}.detail-title>div>span{color:var(--muted);font-size:var(--font-size-body)}.detail-tabs{display:flex;gap:6px;margin:10px 0 22px;border-bottom:1px solid var(--line)}.detail-tabs button{border:0;border-bottom:2px solid transparent;padding:10px 16px;background:transparent;color:var(--muted)}.detail-tabs button.active{border-color:var(--accent);color:var(--text);font-weight:700}.compact-form{display:flex;min-height:0;flex:1;flex-direction:column}.form-grid{display:grid;grid-template-columns:repeat(2,minmax(0,1fr));gap:16px}.span-2{grid-column:span 2}.field textarea{min-height:74px;resize:none}.detail-actions{display:flex;justify-content:flex-end;gap:10px;margin-top:auto;padding-top:22px}.contacts-layout{display:grid;min-height:0;grid-template-columns:minmax(0,1fr) minmax(320px,.8fr);gap:20px}.contact-cards{display:grid;align-content:start;gap:9px}.contact-cards article{display:grid;gap:6px;border:1px solid var(--line);border-radius:11px;padding:13px}.contact-cards article>div{display:flex;align-items:center;justify-content:space-between}.contact-cards article>span{color:var(--muted);font-size:var(--font-size-body)}.contact-form{border-left:1px solid var(--line);padding-left:20px}.contact-form h3{margin:0 0 16px}.contact-form .form-grid{gap:12px}.status{display:inline-flex;width:max-content;align-items:center;border-radius:999px;padding:3px 9px;background:var(--success-soft);color:var(--success);font-size:var(--font-size-body)}.status.warning{background:var(--warning-soft);color:var(--warning)}.status.off{background:var(--surface-soft);color:var(--muted)}@media(max-width:980px){.master-detail{grid-template-columns:300px minmax(0,1fr)}.contacts-layout{grid-template-columns:1fr}.contact-form{border-left:0;border-top:1px solid var(--line);padding:18px 0 0}}@media(max-width:760px){.master-detail{grid-template-columns:1fr}.customer-list{max-height:300px}.form-grid{grid-template-columns:1fr}.span-2{grid-column:auto}}
</style>
