import createClient from 'openapi-fetch'
import type { components, paths } from './generated/schema'
import { isSemanticVersion, SEMANTIC_VERSION_HELP } from '../release-version'
import { notifyOperationsSessionExpired } from '../session-expiry'

export type Customer = components['schemas']['Customer']
export type CustomerInput = components['schemas']['CustomerInput']
export type CustomerProfile = components['schemas']['CustomerProfile']
export type Contact = components['schemas']['Contact']
export type ContactInput = components['schemas']['ContactInput']
export type BillingProfileInput = components['schemas']['BillingProfileInput']
export type Overview = components['schemas']['Overview']
export type Operator = components['schemas']['Operator']
export type BackupRecord = components['schemas']['BackupRecord']
export type AuditEvent = components['schemas']['AuditEvent']
export type ReleaseArtifact = components['schemas']['ReleaseArtifact']
export type UpgradeEnvironment = components['schemas']['UpgradeEnvironment']
export type UpgradeEnvironmentInput = components['schemas']['UpgradeEnvironmentInput']
export type EnvironmentUpgrade = components['schemas']['EnvironmentUpgrade']
export type EnvironmentUpgradeDetail = components['schemas']['EnvironmentUpgradeDetail']
export type TargetMaintenance = components['schemas']['TargetMaintenance']
export type UpgradeProbeSample = components['schemas']['UpgradeProbeSample']

export async function listUpgradeEnvironments() {
  return unwrap(await client.GET('/upgrade-environments')).items
}
export async function createUpgradeEnvironment(body: UpgradeEnvironmentInput) {
  return unwrap(await client.POST('/upgrade-environments', { body, params: { header: { 'X-CSRF-Token': csrfToken() } } }))
}
export async function inspectUpgradeEnvironment(environmentID: string) {
  return unwrap(await client.POST('/upgrade-environments/{environmentID}/inspect', { params: { path: { environmentID }, header: { 'X-CSRF-Token': csrfToken() } } }))
}
export async function listEnvironmentUpgrades() {
  return unwrap(await client.GET('/environment-upgrades')).items
}
export async function createEnvironmentUpgrade(environment_id: string, artifact_id: string) {
  return unwrap(await client.POST('/environment-upgrades', { body: { environment_id, artifact_id }, params: { header: { 'X-CSRF-Token': csrfToken() } } }))
}
export async function getEnvironmentUpgrade(upgradeID: string, after = 0) {
  return unwrap(await client.GET('/environment-upgrades/{upgradeID}', { params: { path: { upgradeID }, query: { after } } }))
}
export type ReleaseArtifactInput = components['schemas']['ReleaseArtifactInput']
export type ReleaseTarget = components['schemas']['ReleaseTarget']
export type ReleaseTaskArtifact = components['schemas']['ReleaseTaskArtifact']
export type ReleaseTask = components['schemas']['ReleaseTask']
export type ReleaseTaskDetail = components['schemas']['ReleaseTaskDetail']
export type ReleaseCapabilities = components['schemas']['ReleaseCapabilities']
export type ReleasePublishRequest = components['schemas']['ReleasePublishRequest']

const client = createClient<paths>({ baseUrl: '/api/operations/v1', credentials: 'include' })
let sessionEstablished = false
let currentOperatorID = ''
export function getCurrentOperationsOperatorID(): string { return currentOperatorID }

export class OperationsAPIError extends Error {
  constructor(public code: string, message: string, public status: number, public number?: number) {
    super(number ? `${message}（错误码：${number}）` : message)
    this.name = 'OperationsAPIError'
  }
}

export class OperationsSessionExpiredError extends OperationsAPIError {
  constructor(code: string, status: number, number?: number) {
    super(code, '', status, number)
    this.name = 'OperationsSessionExpiredError'
    this.message = ''
  }
}

function csrfToken(): string {
  const prefix = 'aster_operations_csrf='
  const entry = document.cookie.split(';').map(value => value.trim()).find(value => value.startsWith(prefix))
  return entry ? decodeURIComponent(entry.slice(prefix.length)) : ''
}

type ErrorDetail = { code?: string; message?: string; number?: number } | undefined

function apiError(status: number, detail: ErrorDetail, fallbackMessage = '请求失败', notifyOnUnauthorized = true): OperationsAPIError {
  const code = detail?.code ?? 'REQUEST_FAILED'
  const number = detail?.number
  // A failed current-password check is also HTTP 401, but the session remains
  // valid. Only a session error (or an unclassified HTTP 401) expires the UI.
  const expired = code === 'UNAUTHORIZED' || number === 61_001 || (status === 401 && !detail?.code && number === undefined)
  if (notifyOnUnauthorized && expired) {
    sessionEstablished = false
    currentOperatorID = ''
    notifyOperationsSessionExpired()
    return new OperationsSessionExpiredError(code, status, number)
  }
  return new OperationsAPIError(code, detail?.message ?? fallbackMessage, status, number)
}

function unwrap<T>(result: { data?: T; error?: components['schemas']['ErrorResponse']; response: Response }, notifyOnUnauthorized = true): T {
  if (result.data !== undefined) return result.data
  const error = result.error?.error
  throw apiError(result.response.status, error, '请求失败', notifyOnUnauthorized)
}

export async function login(email: string, password: string) {
  const session = unwrap(await client.POST('/session', { body: { email, password } }), false)
  sessionEstablished = true
  currentOperatorID = session.operator.id
  return session
}

export async function getSession() {
  const session = unwrap(await client.GET('/session'), sessionEstablished)
  sessionEstablished = true
  currentOperatorID = session.operator.id
  return session
}

export async function changePassword(currentPassword: string, newPassword: string): Promise<void> {
  const result = await client.PUT('/session/password', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: { current_password: currentPassword, new_password: newPassword } })
  if (result.response.status !== 204) unwrap(result)
}

export async function logout(): Promise<void> {
  const result = await client.DELETE('/session', { params: { header: { 'X-CSRF-Token': csrfToken() } } })
  if (result.response.status !== 204) unwrap(result)
  sessionEstablished = false
  currentOperatorID = ''
}

export async function getOverview(): Promise<Overview> {
  return unwrap(await client.GET('/overview'))
}

export async function listCustomers(after = '', limit = 50) {
  const query = after ? { after, limit } : { limit }
  return unwrap(await client.GET('/customers', { params: { query } }))
}

export async function createCustomer(input: CustomerInput): Promise<Customer> {
  return unwrap(await client.POST('/customers', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function getCustomerProfile(customerID: string): Promise<CustomerProfile> {
  return unwrap(await client.GET('/customers/{customerID}', { params: { path: { customerID } } }))
}

export async function updateCustomer(customerID: string, input: CustomerInput): Promise<Customer> {
  return unwrap(await client.PATCH('/customers/{customerID}', { params: { path: { customerID }, header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function createContact(customerID: string, input: ContactInput): Promise<Contact> {
  return unwrap(await client.POST('/customers/{customerID}/contacts', { params: { path: { customerID }, header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function upsertBillingProfile(customerID: string, input: BillingProfileInput) {
  return unwrap(await client.PUT('/customers/{customerID}/billing-profile', { params: { path: { customerID }, header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function listBackupHistory(limit = 50): Promise<BackupRecord[]> {
  return unwrap(await client.GET('/backup-history', { params: { query: { limit } } })).items
}

export async function downloadOperationsExport(currentPassword: string): Promise<void> {
  const response = await fetch('/api/operations/v1/exports', { method: 'POST', credentials: 'include', headers: { 'Content-Type': 'application/json', 'X-CSRF-Token': csrfToken() }, body: JSON.stringify({ current_password: currentPassword }) })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '导出失败')
  }
  const url = URL.createObjectURL(await response.blob())
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = `aster-operations-export-${new Date().toISOString().slice(0, 10)}.json`
  anchor.click()
  URL.revokeObjectURL(url)
}

export async function listAuditEvents(limit = 100): Promise<AuditEvent[]> {
  return unwrap(await client.GET('/audit-events', { params: { query: { limit } } })).items
}

export async function listReleaseArtifacts(limit = 50): Promise<ReleaseArtifact[]> {
  return unwrap(await client.GET('/release-artifacts', { params: { query: { limit } } })).items
}

export async function importReleaseArtifact(input: ReleaseArtifactInput): Promise<ReleaseArtifact> {
  if (!isSemanticVersion(input.version)) throw new Error(SEMANTIC_VERSION_HELP)
  return unwrap(await client.POST('/release-artifacts', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function listReleaseTasks(limit = 50): Promise<ReleaseTask[]> {
  return unwrap(await client.GET('/release-tasks', { params: { query: { limit } } })).items
}

export async function getReleaseTask(taskID: string): Promise<ReleaseTaskDetail> {
  return unwrap(await client.GET('/release-tasks/{taskID}', { params: { path: { taskID } } }))
}

export async function getReleaseCapabilities(): Promise<ReleaseCapabilities> {
  return unwrap(await client.GET('/release-capabilities'))
}

export async function createReleaseTask(version: string, sourceRef: string, freeDistributionID: string): Promise<ReleaseTaskDetail> {
  if (!isSemanticVersion(version)) throw new Error(SEMANTIC_VERSION_HELP)
  return unwrap(await client.POST('/release-tasks', {
    params: { header: { 'X-CSRF-Token': csrfToken() } }, body: { version, source_ref: sourceRef, free_distribution_id: freeDistributionID },
  }))
}

export async function syncReleaseTask(taskID: string): Promise<ReleaseTaskDetail> {
  return unwrap(await client.POST('/release-tasks/{taskID}/sync', {
    params: { path: { taskID }, header: { 'X-CSRF-Token': csrfToken() } },
  }))
}

export async function retryReleaseTask(taskID: string): Promise<ReleaseTaskDetail> {
  return unwrap(await client.POST('/release-tasks/{taskID}/retry', {
    params: { path: { taskID }, header: { 'X-CSRF-Token': csrfToken() } },
  }))
}

export async function reverifyReleaseTask(taskID: string, artifactID: string): Promise<ReleaseTaskDetail> {
  return unwrap(await client.POST('/release-tasks/{taskID}/artifacts/{artifactID}/reverify', {
    params: { path: { taskID, artifactID }, header: { 'X-CSRF-Token': csrfToken() } },
  }))
}

export async function listReleasePublishRequests(limit = 50): Promise<ReleasePublishRequest[]> {
  return unwrap(await client.GET('/release-publish-requests', { params: { query: { limit } } })).items
}

export async function requestReleasePublish(artifactID: string): Promise<ReleasePublishRequest> {
  return unwrap(await client.POST('/release-artifacts/{artifactID}/publish-requests', {
    params: { path: { artifactID }, header: { 'X-CSRF-Token': csrfToken() } }, body: {},
  }))
}

export async function decideReleasePublish(requestID: string, decision: 'approved' | 'rejected', comment: string, currentPassword: string): Promise<ReleasePublishRequest> {
  return unwrap(await client.POST('/release-publish-requests/{requestID}/decision', {
    params: { path: { requestID }, header: { 'X-CSRF-Token': csrfToken() } },
    body: { decision, comment, current_password: currentPassword },
  }))
}

export async function executeReleasePublish(requestID: string, currentPassword: string): Promise<ReleasePublishRequest> {
  return unwrap(await client.POST('/release-publish-requests/{requestID}/execute', {
    params: { path: { requestID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { current_password: currentPassword },
  }))
}

export async function downloadReleaseArtifact(artifactID: string): Promise<void> {
  const response = await fetch(`/api/operations/v1/release-artifacts/${encodeURIComponent(artifactID)}/download`, { credentials: 'include' })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '下载发布物失败')
  }
  const disposition = response.headers.get('Content-Disposition') || ''
  const fileName = /^attachment; filename="(aster-team-[A-Za-z0-9.+-]+-(?:linux|windows)-amd64\.tar\.gz)"$/i.exec(disposition)?.[1]
  if (!fileName) throw new Error('下载响应缺少有效的安装包文件名')
  const url = URL.createObjectURL(await response.blob())
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = fileName
  anchor.click()
  URL.revokeObjectURL(url)
}

export async function rotateUpgradeCredentials(environmentID: string, body: components['schemas']['UpgradeCredentials']) {
  const result = await client.PUT('/upgrade-environments/{environmentID}/credentials', { body, params: { path: { environmentID }, header: { 'X-CSRF-Token': csrfToken() } } })
  if (result.response.status !== 204) unwrap(result)
}

export type CommercialPlanDefinition = components['schemas']['CommercialPlanDefinition']
export type PublicCatalog = components['schemas']['PublicCatalog']
export type PublicCatalogRequest = components['schemas']['PublicCatalogRequest']
export type PublicCatalogPreview = components['schemas']['PublicCatalogPreview']
export type CatalogApprovalRecord = components['schemas']['CatalogApprovalRecord']
export type ApprovePublicCatalogInput = Omit<components['schemas']['ApprovePublicCatalogInput'], 'current_password'>
export async function previewPublicCatalog(input: PublicCatalogRequest): Promise<PublicCatalogPreview> {
  return unwrap(await client.POST('/commercial/catalogs/preview', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}
export async function approvePublicCatalog(input: ApprovePublicCatalogInput, password: string): Promise<CatalogApprovalRecord> {
  return unwrap(await client.POST('/commercial/catalogs', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export async function listCatalogApprovals(limit = 20): Promise<CatalogApprovalRecord[]> {
  return unwrap(await client.GET('/commercial/catalogs', { params: { query: { limit } } })).items
}
export type PublicationRecord = components['schemas']['PublicationRecord']
export type PublicationFailure = components['schemas']['PublicationFailure']
export async function listPublicationFailures(publicationID: string): Promise<PublicationFailure[]> {
  return unwrap(await client.GET('/commercial/publications/{publicationID}/failures', { params: { path: { publicationID } } })).items
}
export type PreparePublicationInput = Omit<components['schemas']['PreparePublicationRequest'], 'current_password'>
export async function preparePublication(input: PreparePublicationInput, password: string): Promise<PublicationRecord> {
  return unwrap(await client.POST('/commercial/publications', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export async function listPublications(): Promise<PublicationRecord[]> {
  return unwrap(await client.GET('/commercial/publications', { params: { query: { limit: 100 } } })).items
}
export async function getPublication(publicationID: string): Promise<PublicationRecord> {
  return unwrap(await client.GET('/commercial/publications/{publicationID}', { params: { path: { publicationID } } }))
}
export async function getPublicationHead(environment: 'local' | 'production'): Promise<string> {
  return unwrap(await client.GET('/commercial/publication-heads/{environment}', { params: { path: { environment } } })).active_id
}
export async function acceptPublication(publicationID: string, password: string): Promise<PublicationRecord> {
  return unwrap(await client.POST('/commercial/publications/{publicationID}/accept', { params: { path: { publicationID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { current_password: password } }))
}
export async function getCatalogApproval(catalogID: string): Promise<CatalogApprovalRecord> {
  return unwrap(await client.GET('/commercial/catalogs/{catalogID}', { params: { path: { catalogID } } }))
}
export async function exportPublicCatalog(catalogID: string, password: string): Promise<CatalogApprovalRecord> {
  return unwrap(await client.POST('/commercial/catalogs/{catalogID}/export', { params: { path: { catalogID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { current_password: password } }))
}
export async function downloadPublicCatalog(record: CatalogApprovalRecord): Promise<void> {
  const response = await fetch(`/api/operations/v1/commercial/catalogs/${encodeURIComponent(record.snapshot.id)}/download`, { credentials: 'include' })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '下载公开目录失败')
  }
  const data = await response.arrayBuffer()
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', data)), byte => byte.toString(16).padStart(2, '0')).join('')
  if (digest !== record.public.sha256 || digest !== response.headers.get('X-Content-SHA256') || response.headers.get('X-Catalog-Revision') !== record.snapshot.id) throw new Error('公开目录版本或摘要不一致，请重新核对')
  const url = URL.createObjectURL(new Blob([data], { type: 'application/json' }))
  const anchor = document.createElement('a')
  anchor.href = url; anchor.download = 'plans.json'; anchor.click(); URL.revokeObjectURL(url)
}
export type CommercialPlanRecord = components['schemas']['CommercialPlanRecord']
export type CommercialOrderRecord = components['schemas']['CommercialOrderRecord']
export type FreezeCommercialPlanInput = components['schemas']['FreezeCommercialPlanInput']
export type PlanDraftRecord = components['schemas']['PlanDraftRecord']
export type SavePlanDraftInput = components['schemas']['SavePlanDraftInput']
export type FreezePlanDraftInput = components['schemas']['FreezePlanDraftInput']
export async function listPlanDrafts(): Promise<PlanDraftRecord[]> {
  return unwrap(await client.GET('/commercial/plan-drafts', { params: { query: { limit: 100 } } })).items
}
export async function getPlanDraft(draftID: string, revision?: number): Promise<PlanDraftRecord> {
  return unwrap(await client.GET('/commercial/plan-drafts/{draftID}', { params: { path: { draftID }, query: revision === undefined ? {} : { revision } } }))
}
export async function savePlanDraft(input: SavePlanDraftInput): Promise<PlanDraftRecord> {
  return unwrap(await client.POST('/commercial/plan-drafts', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}
export async function freezePlanDraft(input: FreezePlanDraftInput): Promise<CommercialPlanRecord> {
  return unwrap(await client.POST('/commercial/plan-drafts/freeze', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}
export type CreateCommercialOrderInput = components['schemas']['CreateCommercialOrderInput']
export type CreateQuotationOrderInput = components['schemas']['CreateQuotationOrderInput']
export type QuotationSource = components['schemas']['QuotationSource']
export async function getQuotationSource(reference: string): Promise<QuotationSource> {
  return unwrap(await client.GET('/commercial/quotation-sources/{reference}', { params: { path: { reference } } }))
}
export async function createQuotationOrder(input: CreateQuotationOrderInput): Promise<CommercialOrderRecord> {
  return unwrap(await client.POST('/commercial/quotation-orders', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export async function listCommercialPlans(limit = 50): Promise<CommercialPlanRecord[]> {
  return unwrap(await client.GET('/commercial/plans', { params: { query: { limit } } })).items
}
export async function getCommercialPlan(planID: string, version: number): Promise<CommercialPlanRecord> {
  return unwrap(await client.GET('/commercial/plans/{planID}/versions/{version}', { params: { path: { planID, version } } }))
}
export async function getCurrentCommercialPlan(planID: string): Promise<CommercialPlanRecord> {
  return unwrap(await client.GET('/commercial/plans/{planID}', { params: { path: { planID } } }))
}
export async function freezeCommercialPlan(input: FreezeCommercialPlanInput): Promise<CommercialPlanRecord> {
  return unwrap(await client.POST('/commercial/plans/versions', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}
export type CommercialOrderListOptions = {
  limit?: number
  offset?: number
  keyword?: string
  status?: CommercialOrderRecord['status'] | ''
  stage?: 'all' | 'pending_payment' | 'pending_approval' | 'pending_issue' | 'issued' | ''
}
export async function listCommercialOrders(options: CommercialOrderListOptions = {}) {
  const query: {
    limit?: number
    offset?: number
    keyword?: string
    status?: Exclude<CommercialOrderListOptions['status'], '' | undefined>
    stage?: Exclude<CommercialOrderListOptions['stage'], '' | undefined>
  } = {}
  if (options.limit !== undefined) query.limit = options.limit
  if (options.offset !== undefined) query.offset = options.offset
  if (options.keyword) query.keyword = options.keyword
  if (options.status) query.status = options.status
  if (options.stage) query.stage = options.stage
  return unwrap(await client.GET('/commercial/orders', { params: { query } }))
}
export async function getCommercialOrder(orderID: string): Promise<CommercialOrderRecord> {
  return unwrap(await client.GET('/commercial/orders/{orderID}', { params: { path: { orderID } } }))
}
export async function createCommercialOrder(input: CreateCommercialOrderInput): Promise<CommercialOrderRecord> {
  return unwrap(await client.POST('/commercial/orders', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: input }))
}

export type FreeDistributionRecord = components['schemas']['FreeDistributionRecord']
// Persist only the business request; reauthentication is supplied per attempt.
export type ApproveFreeDistributionInput = Omit<components['schemas']['ApproveFreeDistributionInput'], 'current_password'>
export type V2IssuerProfile = components['schemas']['V2IssuerProfile']
export async function listFreeDistributions(): Promise<FreeDistributionRecord[]> {
  return unwrap(await client.GET('/commercial/distributions')).items
}
export async function getFreeDistribution(distributionID: string): Promise<FreeDistributionRecord> {
  return unwrap(await client.GET('/commercial/distributions/{distributionID}', { params: { path: { distributionID } } }))
}
export async function approveFreeDistribution(input: ApproveFreeDistributionInput, currentPassword: string): Promise<FreeDistributionRecord> {
  return unwrap(await client.POST('/commercial/distributions', { params: { header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: currentPassword } }))
}
export async function issueFreeDistribution(distributionID: string, keyID: string, currentPassword: string): Promise<FreeDistributionRecord> {
  return unwrap(await client.POST('/commercial/distributions/{distributionID}/issue', { params: { path: { distributionID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { key_id: keyID, current_password: currentPassword } }))
}
export async function listV2IssuerProfiles(): Promise<V2IssuerProfile[]> {
  return unwrap(await client.GET('/commercial/issuers')).items
}
export async function downloadFreeDistribution(record: FreeDistributionRecord): Promise<void> {
  const response = await fetch(`/api/operations/v1/commercial/distributions/${encodeURIComponent(record.snapshot.id)}/download`, { credentials: 'include' })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '下载免费授权失败')
  }
  const data = await response.arrayBuffer()
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', data)), byte => byte.toString(16).padStart(2, '0')).join('')
  if (digest !== record.document_sha256 || digest !== response.headers.get('X-Content-SHA256')) throw new Error('授权文件摘要与分发记录不一致，请重新核对')
  const url = URL.createObjectURL(new Blob([data], { type: 'application/json' }))
  const anchor = document.createElement('a')
  anchor.href = url; anchor.download = `aster-free-${record.snapshot.id}.license.json`
  anchor.click(); URL.revokeObjectURL(url)
}

export type CommercialPaymentRecord = components['schemas']['CommercialPaymentRecord']
export type CommercialPaymentContext = components['schemas']['CommercialPaymentContext']
export type ConfirmCommercialPaymentInput = Omit<components['schemas']['ConfirmCommercialPaymentInput'], 'current_password'>
export async function getCommercialPaymentContext(orderID: string): Promise<CommercialPaymentContext> {
  return unwrap(await client.GET('/commercial/orders/{orderID}/payment-context', { params: { path: { orderID } } }))
}
export async function getCommercialPayment(orderID: string): Promise<CommercialPaymentRecord> {
  return unwrap(await client.GET('/commercial/orders/{orderID}/payment', { params: { path: { orderID } } }))
}
export async function confirmCommercialPayment(orderID: string, input: ConfirmCommercialPaymentInput, password: string): Promise<CommercialPaymentRecord> {
  return unwrap(await client.POST('/commercial/orders/{orderID}/payment', { params: { path: { orderID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export type PaidLicenseRequest = components['schemas']['license-request.v2.schema']

export type PaidFulfillmentRecord = components['schemas']['PaidFulfillmentRecord']
export type PaidFulfillmentContext = components['schemas']['PaidFulfillmentContext']
export type PaidLifecycleSource = components['schemas']['PaidLifecycleSource']
export type ApprovePaidFulfillmentInput = Omit<components['schemas']['ApprovePaidFulfillmentInput'], 'current_password'>
export type PaidRedeliveryRecord = components['schemas']['PaidRedeliveryRecord']
export type RecordPaidRedeliveryInput = Omit<components['schemas']['RecordPaidRedeliveryInput'], 'current_password'>
export type PaidTransferRecord = components['schemas']['PaidTransferRecord']
export type ApprovePaidTransferInput = Omit<components['schemas']['ApprovePaidTransferInput'], 'current_password'>
export async function getPaidFulfillmentContext(orderID: string): Promise<PaidFulfillmentContext> {
  return unwrap(await client.GET('/commercial/orders/{orderID}/fulfillment-context', { params: { path: { orderID } } }))
}
export async function getPaidLifecycleSource(kind: 'renewal' | 'upgrade', sourceID: string): Promise<PaidLifecycleSource> {
  return unwrap(await client.GET('/commercial/paid-lifecycle-sources/{kind}/{sourceID}', { params: { path: { kind, sourceID } } }))
}
export async function getPaidFulfillmentForOrder(orderID: string): Promise<PaidFulfillmentRecord> {
  return unwrap(await client.GET('/commercial/orders/{orderID}/fulfillment', { params: { path: { orderID } } }))
}
export async function getPaidFulfillment(fulfillmentID: string): Promise<PaidFulfillmentRecord> {
  return unwrap(await client.GET('/commercial/paid-fulfillments/{fulfillmentID}', { params: { path: { fulfillmentID } } }))
}
export async function approvePaidFulfillment(orderID: string, input: ApprovePaidFulfillmentInput, password: string): Promise<PaidFulfillmentRecord> {
  return unwrap(await client.POST('/commercial/orders/{orderID}/fulfillment', { params: { path: { orderID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export async function issuePaidFulfillment(fulfillmentID: string, keyID: string, password: string): Promise<PaidFulfillmentRecord> {
  return unwrap(await client.POST('/commercial/paid-fulfillments/{fulfillmentID}/issue', { params: { path: { fulfillmentID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { key_id: keyID, current_password: password } }))
}
export async function recordPaidRedelivery(fulfillmentID: string, input: RecordPaidRedeliveryInput, password: string): Promise<PaidRedeliveryRecord> {
  return unwrap(await client.POST('/commercial/paid-fulfillments/{fulfillmentID}/redeliveries', { params: { path: { fulfillmentID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export async function getPaidTransfer(transferID: string): Promise<PaidTransferRecord> {
  return unwrap(await client.GET('/commercial/paid-transfers/{transferID}', { params: { path: { transferID } } }))
}
export async function getLatestPaidTransfer(fulfillmentID: string): Promise<PaidTransferRecord> {
  return unwrap(await client.GET('/commercial/paid-fulfillments/{fulfillmentID}/transfers/latest', { params: { path: { fulfillmentID } } }))
}
export async function approvePaidTransfer(fulfillmentID: string, input: ApprovePaidTransferInput, password: string): Promise<PaidTransferRecord> {
  return unwrap(await client.POST('/commercial/paid-fulfillments/{fulfillmentID}/transfers', { params: { path: { fulfillmentID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { ...input, current_password: password } }))
}
export async function issuePaidTransfer(transferID: string, keyID: string, password: string): Promise<PaidTransferRecord> {
  return unwrap(await client.POST('/commercial/paid-transfers/{transferID}/issue', { params: { path: { transferID }, header: { 'X-CSRF-Token': csrfToken() } }, body: { key_id: keyID, current_password: password } }))
}
export async function downloadPaidTransfer(record: PaidTransferRecord): Promise<void> {
  const response = await fetch(`/api/operations/v1/commercial/paid-transfers/${encodeURIComponent(record.snapshot.id)}/license`, { credentials: 'include' })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '下载换机授权失败')
  }
  const data = await response.arrayBuffer()
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', data)), byte => byte.toString(16).padStart(2, '0')).join('')
  if (digest !== record.document_sha256 || digest !== response.headers.get('X-Content-SHA256')) throw new Error('换机授权文件摘要与签发记录不一致，请重新核对')
  const url = URL.createObjectURL(new Blob([data], { type: 'application/json' }))
  const anchor = document.createElement('a')
  anchor.href = url; anchor.download = `aster-paid-transfer-${record.snapshot.id}.license.json`
  anchor.click(); URL.revokeObjectURL(url)
}
export async function downloadPaidFulfillment(record: PaidFulfillmentRecord): Promise<void> {
  const response = await fetch(`/api/operations/v1/commercial/paid-fulfillments/${encodeURIComponent(record.snapshot.id)}/license`, { credentials: 'include' })
  if (!response.ok) {
    const body = await response.json().catch(() => null) as components['schemas']['ErrorResponse'] | null
    throw apiError(response.status, body?.error, '下载付费授权失败')
  }
  const data = await response.arrayBuffer()
  const digest = Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256', data)), byte => byte.toString(16).padStart(2, '0')).join('')
  if (digest !== record.document_sha256 || digest !== response.headers.get('X-Content-SHA256')) throw new Error('授权文件摘要与履约记录不一致，请重新核对')
  const url = URL.createObjectURL(new Blob([data], { type: 'application/json' }))
  const anchor = document.createElement('a')
  anchor.href = url; anchor.download = `aster-paid-${record.snapshot.id}.license.json`
  anchor.click(); URL.revokeObjectURL(url)
}
