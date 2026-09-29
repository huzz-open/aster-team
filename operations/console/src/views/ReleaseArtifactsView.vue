<script setup lang="ts">
import { computed, onMounted, reactive, ref } from 'vue'
import { AButton, AEmpty, AIconButton, ALoadingState, AModal, useToast } from '@aster/ui'
import { downloadReleaseArtifact, importReleaseArtifact, listReleaseArtifacts, type ReleaseArtifact } from '../api/client'
import { isSemanticVersion, MAXIMUM_SEMANTIC_VERSION_LENGTH, SEMANTIC_VERSION_HELP, SEMANTIC_VERSION_INPUT_PATTERN } from '../release-version'
const releases = ref<ReleaseArtifact[]>([])
const releaseOpen = ref(false); const saving = ref(false); const loading = ref(true)
const toast = useToast()
const releaseForm = reactive({ inbox_filename: '', version: '', platform: 'linux' as const, architecture: 'amd64' as const, expected_sha256: '', release_manifest_sha256: '', signature_ref: '' })
const releaseVersionInvalid = computed(() => releaseForm.version.length > 0 && !isSemanticVersion(releaseForm.version))
onMounted(async () => { try { releases.value = await listReleaseArtifacts() } catch (error) { toast.error(error instanceof Error ? error.message : '读取安装包失败') } finally { loading.value = false } })
async function importRelease() {
  saving.value = true
  try { releases.value = [await importReleaseArtifact(releaseForm), ...releases.value]; releaseOpen.value = false; toast.success('安装包已导入') }
  catch (error) { toast.error(error instanceof Error ? error.message : '导入失败') }
  finally { saving.value = false }
}
async function download(id: string) {
  try { await downloadReleaseArtifact(id); toast.success('已开始保存安装包') }
  catch (error) { toast.error(error instanceof Error ? error.message : '下载安装包失败') }
}
</script>

<template>
<section class="content">
  <div class="page-head"><h1>安装包</h1><AButton icon="upload" variant="secondary" @click="releaseOpen = true">导入安装包</AButton></div>
  <div class="table-wrap"><ALoadingState v-if="loading" label="正在读取安装包…" /><table v-else-if="releases.length" class="flat-data-table"><thead><tr><th>版本</th><th>发布物 ID</th><th>平台</th><th>大小</th><th>压缩包 SHA-256</th><th>清单 SHA-256</th><th>操作</th></tr></thead><tbody><tr v-for="item in releases" :key="item.id"><td><strong>{{ item.version }}</strong></td><td class="code">{{ item.id }}</td><td>{{ item.platform }} / {{ item.architecture }}</td><td>{{ (item.size_bytes / 1024 / 1024).toFixed(2) }} MiB</td><td><code>{{ item.sha256 }}</code></td><td><code>{{ item.release_manifest_sha256 }}</code></td><td><AIconButton icon="download" label="下载安装包" size="small" @click="download(item.id)" /></td></tr></tbody></table><AEmpty v-else title="暂无安装包" /></div>
<AModal :open="releaseOpen" title="从受控 inbox 导入 Release" @close="releaseOpen = false"><form class="form" @submit.prevent="importRelease"><div class="notice">手工导入当前接受 <code>aster-team-&lt;version&gt;-linux-amd64.tar.gz</code>。压缩包 SHA-256 和包内 RELEASE.json SHA-256 都必须来自发布工作流的独立留档。</div><label class="field"><span>inbox 文件名</span><input v-model="releaseForm.inbox_filename" required></label><div class="two-columns"><label class="field"><span>版本号</span><input v-model.trim="releaseForm.version" required :pattern="SEMANTIC_VERSION_INPUT_PATTERN" :maxlength="MAXIMUM_SEMANTIC_VERSION_LENGTH" :title="SEMANTIC_VERSION_HELP" aria-describedby="release-artifact-version-help"><small id="release-artifact-version-help">{{ SEMANTIC_VERSION_HELP }}</small><small v-if="releaseVersionInvalid" class="danger-text">请输入有效的 SemVer 2.0.0 版本号。</small></label><label class="field"><span>目标</span><input value="Linux / amd64 / SQLCipher" disabled></label></div><label class="field"><span>压缩包 SHA-256</span><input v-model="releaseForm.expected_sha256" required pattern="[0-9a-f]{64}"></label><label class="field"><span>RELEASE.json SHA-256</span><input v-model="releaseForm.release_manifest_sha256" required pattern="[0-9a-f]{64}"></label><label class="field"><span>发布签名引用</span><input v-model="releaseForm.signature_ref" placeholder="可选签名对象或 CI 证明引用"></label><div class="form-actions"><AButton variant="secondary" type="button" :disabled="saving" @click="releaseOpen = false">取消</AButton><AButton type="submit" :loading="saving" :disabled="releaseVersionInvalid">导入并验哈希</AButton></div></form></AModal>
</section>
</template>
