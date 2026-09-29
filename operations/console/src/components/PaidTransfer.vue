<script setup lang="ts">
import { computed, onUnmounted, ref } from "vue";
import {
  AButton,
  ACopyCode,
  AFilePicker,
  ALoadingState,
  AModal,
  ASelect,
  useToast,
} from "@aster/ui";
import {
  approvePaidTransfer,
  downloadPaidTransfer,
  getCurrentOperationsOperatorID,
  getLatestPaidTransfer,
  getPaidTransfer,
  issuePaidTransfer,
  listV2IssuerProfiles,
  OperationsAPIError,
  type ApprovePaidTransferInput,
  type PaidFulfillmentRecord,
  type PaidTransferRecord,
  type V2IssuerProfile,
} from "../api/client";
import { readPaidLicenseRequest } from "../commercial/paid-fulfillment";
import { submissionJournal } from "../commercial/submission-journal";
import ReauthActionModal from "./ReauthActionModal.vue";

type TransferSubmission = ApprovePaidTransferInput & { fulfillment_id: string };
type IssueSubmission = {
  operation_id: string;
  transfer_id: string;
  key_id: string;
};

const props = defineProps<{ fulfillment: PaidFulfillmentRecord }>();
const emit = defineEmits<{ completed: [] }>();
const toast = useToast();
const owner = getCurrentOperationsOperatorID();
const transferJournal = submissionJournal<TransferSubmission>(
  "paid-transfer",
  owner,
);
const issueJournal = submissionJournal<IssueSubmission>(
  "paid-transfer-issue",
  owner,
);
const pendingTransfer = transferJournal.pending;
const pendingIssue = issueJournal.pending;

const open = ref(false);
const loading = ref(false);
const approving = ref(false);
const issuing = ref(false);
const downloading = ref(false);
const error = ref("");
const latest = ref<PaidTransferRecord | null>(null);
const record = ref<PaidTransferRecord | null>(null);
const requestFile = ref<File | null>(null);
const requestText = ref("");
const requestReading = ref(false);
const reason = ref("");
const checked = ref(false);
const approvalPassword = ref("");
const issuePassword = ref("");
const confirmationAction = ref<"approve" | "issue" | null>(null);
const profiles = ref<V2IssuerProfile[]>([]);
const selectedKey = ref("");
const confirmationBusy = computed(() => approving.value || issuing.value);
let readGeneration = 0;
let activeFile: File | null = null;

const initialClaims = computed(() => props.fulfillment.claims);
const initialExpiry = computed(() =>
  initialClaims.value?.validity?.expiry?.mode === "fixed"
    ? initialClaims.value.validity.expiry.expires_at
    : "",
);
const currentHash = computed(
  () =>
    latest.value?.document_sha256 ?? props.fulfillment.document_sha256 ?? "",
);
const shownCurrentHash = computed(
  () =>
    pendingTransfer.value?.expected_current_document_sha256 ??
    currentHash.value,
);
const currentSequence = computed(
  () => latest.value?.snapshot.transfer_sequence ?? 0,
);
const transferLimit = computed(
  () =>
    props.fulfillment.snapshot.payment.snapshot.order.plan.definition
      .transfer_limit,
);
const exhausted = computed(() => currentSequence.value >= transferLimit.value);
const approvalConflict = computed(() => {
  const pending = pendingTransfer.value;
  const current = latest.value;
  return pending &&
    current &&
    current.snapshot.request.operation_id !== pending.operation_id
    ? current
    : null;
});
const recoveredApproval = computed(() => {
  const pending = pendingTransfer.value;
  const current = latest.value;
  return pending &&
    current &&
    current.snapshot.request.operation_id === pending.operation_id
    ? current
    : null;
});
const validStoredTransfer = computed(() => {
  const value = pendingTransfer.value;
  return (
    !value ||
    (value.fulfillment_id === props.fulfillment.snapshot.id &&
      /^[a-f0-9]{64}$/.test(value.expected_current_document_sha256) &&
      !!value.operation_id &&
      !!value.license_request_json &&
      !!value.reason)
  );
});
const validStoredIssue = computed(() => {
  const value = pendingIssue.value;
  return (
    !value || (!!value.operation_id && !!value.transfer_id && !!value.key_id)
  );
});
const journalError = computed(
  () =>
    transferJournal.error.value ||
    issueJournal.error.value ||
    (!validStoredTransfer.value || !validStoredIssue.value
      ? "待确认换机记录损坏，请保留浏览器记录并人工核对"
      : ""),
);
const issueConflict = computed(() => {
  const pending = pendingIssue.value;
  const current = record.value;
  return pending &&
    current?.snapshot.id === pending.transfer_id &&
    current.claims?.key_id &&
    current.claims.key_id !== pending.key_id
    ? { local: pending.key_id, server: current.claims.key_id }
    : null;
});
const keyOptions = computed(() =>
  profiles.value
    .filter(
      (value) =>
        value.policy.sources.includes("commercial_order") &&
        value.policy.bindings.includes("installation") &&
        value.policy.expiries.includes("fixed"),
    )
    .map((value) => ({ value: value.key_id, label: value.key_id })),
);
const message = (value: unknown, fallback: string) =>
  value instanceof Error ? value.message : fallback;
const statusName = (value: string) =>
  ({ approved: "已批准", prepared: "待完成签发", issued: "已签发" })[value] ??
  value;

async function start() {
  open.value = true;
  confirmationAction.value = null;
  error.value = "";
  record.value = null;
  latest.value = null;
  if (pendingTransfer.value && validStoredTransfer.value) {
    reason.value = pendingTransfer.value.reason;
    requestText.value = pendingTransfer.value.license_request_json;
  }
  if (pendingIssue.value && validStoredIssue.value) {
    selectedKey.value = pendingIssue.value.key_id;
    await recoverIssue();
    return;
  }
  await loadCurrent(!!pendingTransfer.value);
}
function close() {
  if (loading.value || approving.value || issuing.value || downloading.value)
    return;
  open.value = false;
  confirmationAction.value = null;
  approvalPassword.value = "";
  issuePassword.value = "";
}
async function loadCurrent(suppressRecord = false) {
  loading.value = true;
  try {
    latest.value = await getLatestPaidTransfer(props.fulfillment.snapshot.id);
    if (latest.value.status !== "issued" && !suppressRecord)
      record.value = latest.value;
  } catch (value) {
    if (!(value instanceof OperationsAPIError) || value.status !== 404)
      error.value = message(value, "读取当前授权失败");
  } finally {
    loading.value = false;
  }
}
async function recoverIssue() {
  if (!pendingIssue.value || !validStoredIssue.value) return;
  loading.value = true;
  try {
    record.value = await getPaidTransfer(pendingIssue.value.transfer_id);
    if (
      record.value.status === "issued" &&
      record.value.claims?.key_id === pendingIssue.value.key_id
    )
      issueJournal.clear();
  } catch (value) {
    error.value = message(value, "读取待恢复换机签发失败");
  } finally {
    loading.value = false;
  }
}
async function selectRequest(file: File) {
  const generation = ++readGeneration;
  activeFile = file;
  requestReading.value = true;
  error.value = "";
  try {
    const text = await readPaidLicenseRequest(file);
    if (generation === readGeneration && activeFile === file && open.value)
      requestText.value = text;
  } catch (value) {
    if (generation === readGeneration) {
      requestFile.value = null;
      error.value = message(value, "读取新安装请求失败");
    }
  } finally {
    if (generation === readGeneration) requestReading.value = false;
  }
}
function clearRequest() {
  readGeneration++;
  activeFile = null;
  requestFile.value = null;
  requestText.value = "";
  requestReading.value = false;
}
function promptApproval() {
  if (approving.value || loading.value || journalError.value) return;
  if (!pendingTransfer.value && (!currentHash.value || !requestText.value || !reason.value.trim() || !checked.value)) {
    error.value = "请完整核对当前授权、新安装请求和换机原因";
    return;
  }
  error.value = "";
  confirmationAction.value = "approve";
}
function promptIssue() {
  if (issuing.value || loading.value || journalError.value || issueConflict.value) return;
  if (!pendingIssue.value && !selectedKey.value && !record.value?.claims?.key_id) { error.value = "请选择签发密钥"; return; }
  error.value = "";
  confirmationAction.value = "issue";
}
async function confirmAction(password: string) {
  if (confirmationAction.value === "approve") { approvalPassword.value = password; await approve(); }
  else if (confirmationAction.value === "issue") { issuePassword.value = password; await issue(); }
  if (!error.value) confirmationAction.value = null;
}
async function approve() {
  if (
    approving.value ||
    loading.value ||
    journalError.value ||
    (exhausted.value && !pendingTransfer.value) ||
    !approvalPassword.value
  )
    return;
  error.value = "";
  if (!pendingTransfer.value) {
    if (
      !currentHash.value ||
      !requestText.value ||
      !reason.value.trim() ||
      !checked.value
    ) {
      error.value = "请完整核对当前授权、新安装请求和换机原因";
      return;
    }
    try {
      transferJournal.prepare({
        operation_id: `transfer_${crypto.randomUUID()}`,
        fulfillment_id: props.fulfillment.snapshot.id,
        expected_current_document_sha256: currentHash.value,
        license_request_json: requestText.value,
        reason: reason.value.trim(),
      });
    } catch (value) {
      error.value = message(value, "保存待确认换机失败");
      return;
    }
  }
  approving.value = true;
  let attempt: ReturnType<typeof transferJournal.begin> | undefined;
  try {
    attempt = transferJournal.begin();
    const saved = pendingTransfer.value!;
    const input: ApprovePaidTransferInput = {
      operation_id: saved.operation_id,
      expected_current_document_sha256: saved.expected_current_document_sha256,
      license_request_json: saved.license_request_json,
      reason: saved.reason,
    };
    record.value = await approvePaidTransfer(
      saved.fulfillment_id,
      input,
      approvalPassword.value,
    );
    transferJournal.clear();
    checked.value = false;
    emit("completed");
    toast.success("换机授权已批准");
  } catch (value) {
    const rejected =
      !!attempt &&
      value instanceof OperationsAPIError &&
      [400, 401, 403, 404, 409, 422].includes(value.status) &&
      transferJournal.reject(attempt);
    error.value = `${message(value, "批准换机失败")}。${rejected ? "本次请求已明确拒绝，请重新核对当前授权" : "结果尚未确认，请保留原请求并重试"}`;
  } finally {
    approving.value = false;
    approvalPassword.value = "";
  }
}
async function loadProfiles() {
  if (loading.value) return;
  loading.value = true;
  try {
    profiles.value = await listV2IssuerProfiles();
    if (keyOptions.value.length === 1)
      selectedKey.value = keyOptions.value[0]!.value;
    if (!keyOptions.value.length)
      error.value = "没有允许商业订单、安装绑定和固定期限的签发配置";
  } catch (value) {
    error.value = message(value, "读取签发配置失败");
  } finally {
    loading.value = false;
  }
}
async function issue() {
  const current = record.value;
  if (
    !current ||
    current.status === "issued" ||
    issueConflict.value ||
    issuing.value ||
    journalError.value ||
    !issuePassword.value
  )
    return;
  if (!pendingIssue.value) {
    const key = current.claims?.key_id ?? selectedKey.value;
    if (!key) {
      error.value = "请选择受限付费签发密钥";
      return;
    }
    try {
      issueJournal.prepare({
        operation_id: `issue_${current.snapshot.id}`,
        transfer_id: current.snapshot.id,
        key_id: key,
      });
    } catch (value) {
      error.value = message(value, "保存待确认换机签发失败");
      return;
    }
  }
  issuing.value = true;
  let attempt: ReturnType<typeof issueJournal.begin> | undefined;
  try {
    attempt = issueJournal.begin();
    const saved = pendingIssue.value!;
    record.value = await issuePaidTransfer(
      saved.transfer_id,
      saved.key_id,
      issuePassword.value,
    );
    issueJournal.clear();
    latest.value = record.value;
    emit("completed");
    toast.success("换机授权已签发");
  } catch (value) {
    const rejected =
      !!attempt &&
      value instanceof OperationsAPIError &&
      [400, 401, 403, 404, 409, 422].includes(value.status) &&
      issueJournal.reject(attempt);
    error.value = `${message(value, "签发换机授权失败")}。${rejected ? "本次请求已明确拒绝，请重新读取记录" : "结果尚未确认，请保留原签发身份并重试"}`;
  } finally {
    issuing.value = false;
    issuePassword.value = "";
  }
}
async function download() {
  if (
    !record.value ||
    record.value.status !== "issued" ||
    issueConflict.value ||
    downloading.value
  )
    return;
  await downloadRecord(record.value);
}
async function downloadLatest() {
  if (!latest.value || latest.value.status !== "issued" || downloading.value)
    return;
  await downloadRecord(latest.value);
}
async function downloadRecord(value: PaidTransferRecord) {
  downloading.value = true;
  try {
    await downloadPaidTransfer(value);
    toast.success("换机授权文件摘要已核对");
  } catch (value) {
    error.value = message(value, "下载换机授权失败");
  } finally {
    downloading.value = false;
  }
}
function acceptServerIssueKey() {
  const conflict = issueConflict.value;
  if (!conflict || !record.value?.claims?.key_id) return;
  issueJournal.clear();
  selectedKey.value = record.value.claims.key_id;
  error.value = "";
  toast.info("已结束本标签页的旧密钥请求，并采用服务器冻结记录");
}
function abandonPendingApproval() {
  const current = approvalConflict.value;
  if (!current) return;
  transferJournal.clear();
  record.value = current.status === "issued" ? null : current;
  error.value = "";
  toast.info("已结束本标签页的旧换机请求，并采用服务器最新记录");
}
function abandonUnresolvedApproval() {
  if (!pendingTransfer.value || latest.value) return;
  transferJournal.clear();
  error.value = "";
  toast.info("已结束本标签页的旧换机请求，请重新核对后提交");
}
function adoptRecoveredApproval() {
  const current = recoveredApproval.value;
  if (!current) return;
  transferJournal.clear();
  record.value = current;
  error.value = "";
  toast.info("已采用服务器确认的原换机记录");
}
function abandonUnfrozenIssue() {
  if (
    !pendingIssue.value ||
    record.value?.status !== "approved" ||
    record.value.claims
  )
    return;
  issueJournal.clear();
  selectedKey.value = "";
  profiles.value = [];
  error.value = "";
  toast.info("已结束旧密钥请求，可重新选择签发配置");
}
onUnmounted(() => {
  readGeneration++;
  activeFile = null;
});
</script>

<template>
  <AButton
    variant="secondary"
    :disabled="fulfillment.status !== 'issued'"
    @click="start"
    >办理换机</AButton
  >
  <AModal
    :open="open"
    title="付费授权换机"
    description="从当前已签发授权迁移到新的安装环境，原套餐权益和到期时间保持不变。"
    :close-disabled="loading || approving || issuing || downloading"
    @close="close"
  >
    <div class="transfer-flow">
      <p v-if="journalError || error" class="transfer-error" role="alert">
        {{ journalError || error }}
      </p>
      <ALoadingState v-if="loading" label="正在核对当前授权链" />
      <dl v-if="initialClaims" class="transfer-info">
        <dt>许可证</dt>
        <dd>{{ initialClaims.license_id }}</dd>
        <dt>当前序号</dt>
        <dd>{{ currentSequence }}</dd>
        <dt>允许换机</dt>
        <dd>{{ transferLimit }} 次</dd>
        <dt>到期时间</dt>
        <dd>{{ initialExpiry }}</dd>
      </dl>
      <p v-if="exhausted && !pendingTransfer" class="transfer-error">
        该订单的换机次数已经用完
      </p>
      <div
        v-if="
          latest?.status === 'issued' &&
          (!record || record.snapshot.id !== latest.snapshot.id)
        "
        class="transfer-current"
      >
        <strong>当前换机授权</strong>
        <dl class="transfer-info">
          <dt>换机序号</dt>
          <dd>{{ latest.snapshot.transfer_sequence }}</dd>
          <dt>文件摘要</dt>
          <dd>{{ latest.document_sha256 }}</dd>
        </dl>
        <AButton
          variant="secondary"
          :loading="downloading"
          @click="downloadLatest"
          >下载当前换机授权</AButton
        >
      </div>
      <div v-if="approvalConflict" class="transfer-conflict" role="alert">
        <strong>服务器已有另一条更新的换机记录</strong>
        <p>
          本标签页保留 {{ pendingTransfer?.operation_id }}，服务器最新记录为
          {{
            approvalConflict.snapshot.request.operation_id
          }}。可先重试原操作；确认不再恢复原操作后，再采用服务器记录。
        </p>
        <AButton variant="secondary" @click="abandonPendingApproval"
          >确认结束旧请求并采用服务器记录</AButton
        >
      </div>
      <form
        v-if="!record && !loading && (!exhausted || !!pendingTransfer)"
        class="transfer-form"
        @submit.prevent="promptApproval"
      >
        <template v-if="pendingTransfer">
          <p class="transfer-note">
            上次批准结果尚未确认。重新输入当前密码后只重试同一换机请求。
          </p>
          <dl class="transfer-info">
            <dt>换机原因</dt>
            <dd>{{ pendingTransfer.reason }}</dd>
            <dt>操作编号</dt>
            <dd>{{ pendingTransfer.operation_id }}</dd>
          </dl>
          <AButton
            v-if="recoveredApproval"
            type="button"
            variant="secondary"
            @click="adoptRecoveredApproval"
            >采用服务器已确认记录</AButton
          >
          <AButton
            v-else-if="!latest"
            type="button"
            variant="secondary"
            @click="abandonUnresolvedApproval"
            >结束旧请求并重新核对</AButton
          >
        </template>
        <template v-else>
          <AFilePicker
            v-model="requestFile"
            label="选择新机器的 v2 安装请求"
            hint="JSON 最大 16 KiB，也可选择 CLI 生成的二维码图片"
            accept="application/json,.json,image/png,image/jpeg,image/webp,.png,.jpg,.jpeg,.webp"
            required
            :loading="requestReading"
            @select="selectRequest"
            @clear="clearRequest"
          />
          <label class="field"
            ><span>换机原因</span
            ><textarea
              v-model="reason"
              aria-label="付费授权换机原因"
              maxlength="2000"
              rows="3"
              required
            />
          </label>
          <label class="transfer-check"
            ><input v-model="checked" type="checkbox" required /><span
              >已核对当前授权、新机器安装请求和剩余换机次数</span
            ></label
          >
        </template>
        <p class="transfer-note">当前授权文件摘要</p>
        <ACopyCode
          :value="shownCurrentHash"
          label="复制"
          copied-label="已复制"
        />
        <AButton
          type="submit"
          :loading="approving"
          :disabled="!!journalError"
          >{{ pendingTransfer ? "重试原批准" : "确认批准换机" }}</AButton
        >
      </form>
      <template v-if="record">
        <p class="transfer-stage">{{ statusName(record.status) }}</p>
        <dl class="transfer-info">
          <dt>换机序号</dt>
          <dd>{{ record.snapshot.transfer_sequence }}</dd>
          <dt>新安装编号</dt>
          <dd>{{ record.snapshot.installation_request.installation_id }}</dd>
          <dt>操作编号</dt>
          <dd>{{ record.snapshot.request.operation_id }}</dd>
          <dt>签发密钥</dt>
          <dd>{{ record.claims?.key_id || "尚未冻结" }}</dd>
        </dl>
        <div v-if="issueConflict" class="transfer-conflict" role="alert">
          <strong>服务器已经固定另一签发密钥</strong>
          <p>
            本标签页保留 {{ issueConflict.local }}，可信换机记录固定为
            {{ issueConflict.server }}。核对后采用服务器记录才能继续。
          </p>
          <AButton variant="secondary" @click="acceptServerIssueKey"
            >确认采用服务器固定密钥</AButton
          >
        </div>
        <form
          v-if="record.status !== 'issued'"
          class="transfer-form"
          @submit.prevent="promptIssue"
        >
          <AButton
            v-if="
              pendingIssue && record.status === 'approved' && !record.claims
            "
            type="button"
            variant="secondary"
            @click="abandonUnfrozenIssue"
            >结束旧密钥请求并重新选择</AButton
          >
          <AButton
            v-if="
              record.status === 'approved' && !profiles.length && !pendingIssue
            "
            type="button"
            variant="secondary"
            @click="loadProfiles"
            >读取签发配置</AButton
          >
          <label
            v-else-if="record.status === 'approved' && !pendingIssue"
            class="field"
            ><span>付费签发密钥</span
            ><ASelect v-model="selectedKey" :options="keyOptions" required
          /></label>
          <AButton
            type="submit"
            :loading="issuing"
            :disabled="
              !!journalError ||
              !!issueConflict ||
              (record.status === 'approved' && !pendingIssue && !selectedKey)
            "
            >{{
              pendingIssue || record.status === "prepared"
                ? "重试原签发"
                : "签发换机授权"
            }}</AButton
          >
        </form>
        <div v-else class="transfer-form">
          <p class="transfer-note">换机授权文件摘要</p>
          <ACopyCode
            :value="record.document_sha256 || ''"
            label="复制"
            copied-label="已复制"
          />
          <AButton
            variant="secondary"
            :loading="downloading"
            :disabled="!!issueConflict"
            @click="download"
            >下载换机授权文件</AButton
          >
        </div>
      </template>
      <AButton
        variant="secondary"
        :disabled="loading || approving || issuing || downloading"
        @click="close"
        >关闭</AButton
      >
    </div>
  </AModal>
  <ReauthActionModal :open="!!confirmationAction" :title="confirmationAction === 'issue' ? '确认签发换机授权' : '确认批准换机'" :busy="confirmationBusy" :error="error" @close="confirmationAction = null" @submit="confirmAction" />
</template>

<style scoped>
.transfer-flow,
.transfer-form {
  display: grid;
  gap: 14px;
  min-width: 0;
}
.transfer-info {
  display: grid;
  grid-template-columns: 92px minmax(0, 1fr);
  gap: 9px 14px;
  margin: 0;
  font-size:var(--font-size-body);
}
.transfer-info dt,
.transfer-note {
  color: var(--muted);
}
.transfer-info dd {
  min-width: 0;
  margin: 0;
  overflow-wrap: anywhere;
}
.transfer-note,
.transfer-check {
  margin: 0;
  font-size:var(--font-size-body);
  line-height: 1.7;
}
.transfer-check {
  display: flex;
  align-items: flex-start;
  gap: 10px;
}
.transfer-check input {
  width: 16px;
  height: 16px;
  margin-top: 3px;
}
.transfer-error {
  margin: 0;
  border: 1px solid var(--line);
  border-radius: 10px;
  padding: 12px;
  font-size:var(--font-size-body);
  line-height: 1.6;
}
.transfer-stage {
  width: max-content;
  margin: 0;
  border-radius: 999px;
  padding: 5px 10px;
  background: var(--accent-soft);
  color: var(--accent);
  font-size:var(--font-size-body);
  font-weight: 650;
}
.transfer-conflict {
  display: grid;
  gap: 8px;
  border: 1px solid color-mix(in srgb, var(--warning) 42%, var(--line));
  border-radius: 12px;
  padding: 13px;
  background: var(--warning-soft);
  font-size:var(--font-size-body);
  line-height: 1.6;
}
.transfer-conflict p {
  margin: 0;
  overflow-wrap: anywhere;
}
.transfer-current {
  display: grid;
  gap: 10px;
  border: 1px solid var(--line);
  border-radius: 12px;
  padding: 13px;
}
</style>
