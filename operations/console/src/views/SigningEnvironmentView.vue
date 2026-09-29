<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { AButton, AEmpty, ALoadingState, useToast } from '@aster/ui'
import { listV2IssuerProfiles, type V2IssuerProfile } from '../api/client'

const toast = useToast()
const route = useRoute()
const router = useRouter()
const items = ref<V2IssuerProfile[]>([])
const selected = ref<V2IssuerProfile | null>(null)
const loading = ref(false)
const filteredPolicies = computed(() => selected.value ? [
  ['授权来源', selected.value.policy.sources.join('、')],
  ['安装绑定', selected.value.policy.bindings.join('、')],
  ['到期方式', selected.value.policy.expiries.join('、')],
] : [])
async function load() { loading.value = true; try { items.value = await listV2IssuerProfiles(); const key = typeof route.query.key === 'string' ? route.query.key : selected.value?.key_id; selected.value = key ? items.value.find(item => item.key_id === key) ?? null : items.value[0] ?? null } catch (value) { toast.error(value instanceof Error ? value.message : '读取签名配置失败') } finally { loading.value = false } }
function selectProfile(item: V2IssuerProfile) {
  selected.value = item
  if (route.query.key !== item.key_id) void router.push({ path: route.path, query: { ...route.query, key: item.key_id } })
}
onMounted(load)
watch(() => route.query.key, key => { selected.value = typeof key === 'string' ? items.value.find(item => item.key_id === key) ?? null : items.value[0] ?? null })
</script>

<template>
  <section class="content master-page">
    <div class="page-head"><div><h1>签名与环境</h1></div><AButton variant="secondary" :disabled="loading" @click="load">刷新</AButton></div>
    <div class="master-detail">
      <section class="master-list"><ALoadingState v-if="loading" label="正在读取签名配置" /><button v-for="item in items" v-else :key="item.key_id" :class="{ active: selected?.key_id === item.key_id }" @click="selectProfile(item)"><strong>{{ item.key_id }}</strong><span>{{ item.policy.sources.join(' · ') }}</span></button><AEmpty v-if="!loading && !items.length" title="没有可用签名配置" /></section>
      <section v-if="selected" class="detail-panel"><header><div><h2>{{ selected.key_id }}</h2><span class="status">可用</span></div></header><dl><template v-for="row in filteredPolicies" :key="row[0]"><dt>{{ row[0] }}</dt><dd>{{ row[1] }}</dd></template></dl><div class="security-note"><strong>密钥材料不可见</strong></div></section>
      <AEmpty v-else class="detail-panel" title="选择签名配置" />
    </div>
  </section>
</template>

<style scoped>
.master-page{display:flex;flex-direction:column;min-height:0}.master-detail{display:grid;grid-template-columns:360px minmax(0,1fr);gap:12px;min-height:0;flex:1}.master-list,.detail-panel{min-height:0;border:1px solid var(--line);border-radius:14px;background:var(--surface);padding:14px}.master-list{display:grid;align-content:start;gap:8px}.master-list button{display:grid;gap:6px;width:100%;border:1px solid transparent;border-radius:11px;padding:14px;background:transparent;text-align:left;color:inherit}.master-list button span{color:var(--muted);font-size:var(--font-size-body)}.master-list button.active{border-color:var(--accent);background:var(--accent-soft)}.detail-panel{padding:24px}.detail-panel header>div{display:flex;align-items:center;gap:12px}.detail-panel h2{margin:0}.detail-panel dl{display:grid;grid-template-columns:110px minmax(0,1fr);gap:18px;margin:30px 0}.detail-panel dt{color:var(--muted)}.detail-panel dd{margin:0}.security-note{display:grid;gap:6px;border:1px solid var(--line);border-radius:12px;padding:16px}.security-note span{color:var(--muted);font-size:var(--font-size-body)}@media(max-width:800px){.master-detail{grid-template-columns:1fr}.master-list{max-height:260px}}
</style>
