<script setup lang="ts">
import { memberCanWrite as canWrite } from '../license-status'
import { onMounted, ref } from 'vue'
import { AButton, AConfirmModal, AEmpty, AIconButton, ALoadingState, AModal, useToast } from '@aster/ui'
import { copyText, formatDate, request, type APIKey } from '@aster/sdk'
import { locale, t } from '../i18n'
const items = ref<APIKey[]>([]),
  open = ref(false),
  name = ref(''),
  createdKey = ref('')
const toast = useToast()
const loading = ref(true),
  action = ref<'create' | 'confirm' | ''>('')
const pendingKeyId = ref<string | null>(null)
async function load() {
  loading.value = true
  try {
    const keyResult = await request<{ items: APIKey[] }>('/api/member/keys')
    items.value = keyResult.items.filter((item) => item.status === 'active')
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('keysLoadFailed'))
  } finally {
    loading.value = false
  }
}
async function create() {
  action.value = 'create'
  try {
    const data = await request<{ key: string }>('/api/member/keys', {
      method: 'POST',
      body: JSON.stringify({ name: name.value }),
    })
    createdKey.value = data.key
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('createFailed'))
  } finally {
    action.value = ''
  }
}
async function confirmAction() {
  const keyId = pendingKeyId.value
  if (!keyId) return
  action.value = 'confirm'
  try {
    await request(`/api/member/keys/${keyId}/revoke`, { method: 'POST' })
    toast.success(t('keyDeleted'))
    pendingKeyId.value = null
    await load()
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('remove'))
  } finally {
    action.value = ''
  }
}
async function copy() {
  try {
    await copyText(createdKey.value)
    toast.success(t('copySuccess'))
  } catch (value) {
    toast.error(value instanceof Error ? value.message : t('copyFailed'))
  }
}
function close() {
  open.value = false
  name.value = ''
  createdKey.value = ''
}
onMounted(load)
</script>
<template>
  <div class="content">
    <header class="page-head">
      <div>
        <h1>{{ t('keysTitle') }}</h1>
      </div>
      <AButton icon="plus" @click="open = true" :disabled="!canWrite">{{ t('createKey') }}</AButton>
    </header>
    <div class="table-wrap">
      <ALoadingState v-if="loading && !items.length" :label="t('loadingKeys')" />
      <table v-else-if="items.length">
        <thead>
          <tr>
            <th>{{ t('name') }}</th>
            <th>{{ t('key') }}</th>
            <th>{{ t('status') }}</th>
            <th>{{ t('lastUsed') }}</th>
            <th>{{ t('createdAt') }}</th>
            <th>{{ t('actions') }}</th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="item in items" :key="item.id">
            <td>
              <strong>{{ item.name }}</strong>
            </td>
            <td class="code">{{ item.key_prefix }}…</td>
            <td>
              <span class="status" :class="{ off: item.status !== 'active' }">{{ item.status === 'active' ? t('active') : t('revoked') }}</span>
            </td>
            <td>{{ formatDate(item.last_used_at, locale) }}</td>
            <td>{{ formatDate(item.created_at, locale) }}</td>
            <td>
              <div class="row-actions"><AIconButton icon="trash" size="small" variant="danger" :label="t('remove')" :disabled="action === 'confirm'" @click="pendingKeyId = item.id" /></div>
            </td>
          </tr>
        </tbody>
      </table>
      <AEmpty v-else :title="t('noKeys')" />
    </div>
    <AModal :open="open" :title="t('createKeyTitle')" :description="t('createKeyDesc')" :close-label="t('close')" :close-disabled="Boolean(action)" @close="close"
      ><div v-if="createdKey" class="form">
        <div class="notice">{{ t('keyOnce') }}</div>
        <div class="notice code">{{ createdKey }}</div>
        <AButton icon="copy" @click="copy">{{ t('copyFullKey') }}</AButton>
        <AButton variant="secondary" @click="close">{{ t('savedClose') }}</AButton>
      </div>
      <form v-else class="form" @submit.prevent="create">
        <label class="field"
          ><span>{{ t('purposeName') }}</span
          ><input v-model="name" :placeholder="t('purposePlaceholder')" required
        /></label>
        <div class="form-actions">
          <AButton variant="secondary" :disabled="action === 'create'" @click="close">{{ t('cancel') }}</AButton
          ><AButton type="submit" :loading="action === 'create'" :disabled="!canWrite">{{ t('create') }}</AButton>
        </div>
      </form></AModal
    ><AConfirmModal :open="!!pendingKeyId" :title="t('remove')" :text="t('deleteConfirm')" :confirm-label="t('remove')" :cancel-label="t('cancel')" danger :busy="action === 'confirm'" @close="pendingKeyId = null" @confirm="confirmAction" />
  </div>
</template>
