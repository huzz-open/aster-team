import { createHash, randomUUID } from 'node:crypto'
import { readFileSync } from 'node:fs'
import { parse } from 'yaml'
import { isDeepStrictEqual } from 'node:util'
import { localLicensePolicies } from './license-signing-profile.mjs'

const catalog = parse(readFileSync(new URL('../contracts/catalogs/product-capabilities.yaml', import.meta.url), 'utf8'))

// This is issuer selection only. Operations and Customer still independently
// authenticate the complete signed claims and issuer policy.
export function localEntitlementsWithin(entitlements, ceiling) {
  if (!(entitlements.feature_sets ?? []).every(id => (ceiling.feature_sets ?? []).includes(id))) return false
  const allowed = catalog.capabilities.filter(entry => ceiling.features.includes(entry.id)
    || (ceiling.feature_sets ?? []).includes(entry.feature_set)).map(entry => entry.id)
  if (!entitlements.features.every(id => allowed.includes(id))) return false
  return entitlements.quotas.every(quota => {
    const maximum = ceiling.quotas.find(entry => entry.id === quota.id)?.limit
    return maximum?.mode === 'unlimited' || (quota.limit.mode === 'limited' && maximum?.mode === 'limited' && maximum.value >= quota.limit.value)
  })
}

export function newLocalFulfillment(requestJSON, customerID, now = new Date()) {
  return { schema: 'aster.local-fulfillment.v2', id: randomUUID(), customer_id: customerID,
    request_sha256: createHash('sha256').update(requestJSON).digest('hex'), starts_at: now.toISOString() }
}

export async function requireLocalFulfillmentEnvironment(operations) {
  const environment = await operations.request('/commercial/environment')
  if (environment.fulfillment_environment !== 'local') throw new Error('一键授权只允许履约环境为 local 的 Operations')
}

// This is local development orchestration of the same approved commercial APIs.
// Persist the operation identity and original timestamp before the first write;
// lost responses retry identical business inputs instead of creating new orders.
export async function fulfillLocalDemo({ operations, password, requestJSON, minimumVersion, trustedKeys, state, progress = () => {} }) {
  if (state?.schema !== 'aster.local-fulfillment.v2' || !/^[a-f0-9-]{36}$/.test(state.id) ||
      typeof state.customer_id !== 'string' || !state.customer_id ||
      state.request_sha256 !== createHash('sha256').update(requestJSON).digest('hex') ||
      !Number.isFinite(Date.parse(state.starts_at))) throw new Error('本地履约上下文与当前机器申请不一致，请重新初始化本地环境')
  await requireLocalFulfillmentEnvironment(operations)
  const write = (path, body) => operations.request(path, { method: 'POST', csrfCookie: 'aster_operations_csrf', body })
  const profiles = (await operations.request('/commercial/issuers')).items
  const canIssue = (profile, source, binding, expiry, entitlements) => {
    const policy = profile.policy
    if (!policy?.sources.includes(source) || !policy.bindings.includes(binding) || !policy.expiries.includes(expiry) ||
        !isDeepStrictEqual(profile, trustedKeys.find(key => key.key_id === profile.key_id))) return false
    return localEntitlementsWithin(entitlements, policy.entitlement_ceiling)
  }

  const freeEntitlements = structuredClone(localLicensePolicies()[0].policy.entitlement_ceiling)
  const freeDefinition = {
    product: 'aster-team', code: `local-free-${state.id}`, name: '本地免费版',
    description: '仅用于本地开发与统一安装包联调，不是正式公开发行证书', edition: 'free',
    entitlements: freeEntitlements, quota_policy_version: 1, minimum_version: minimumVersion, transfer_limit: 0,
    support_terms_version: 'local-test-only', offer: { kind: 'free', expiry: { mode: 'none' } },
  }
  const freePlan = await write('/commercial/plans/versions', {
    operation_id: `local_free_plan_${state.id}`, plan_id: '', expected_version: 0, definition: freeDefinition,
  })
  const freeDistribution = await write('/commercial/distributions', {
    operation_id: `local_free_distribution_${state.id}`,
    plan_id: freePlan.snapshot.plan_id, plan_version: freePlan.snapshot.version,
    expected_sha256: freePlan.sha256, not_before: state.starts_at,
    reason: '本地快速授权准备统一安装包免费证书', current_password: password,
  })
  const freeIssuer = profiles.find(profile => canIssue(profile, 'free_distribution', 'unbound', 'none', freeEntitlements))
  if (!freeIssuer) throw new Error('没有与 Customer 当前公钥及权限范围一致的可用免费 v2 签发器')
  const freePath = `/commercial/distributions/${encodeURIComponent(freeDistribution.snapshot.id)}`
  const issuedFree = await write(`${freePath}/issue`, { key_id: freeIssuer.key_id, current_password: password })
  const downloadedFree = await operations.request(`${freePath}/download`, { format: 'bytes' })
  const freeSHA256 = createHash('sha256').update(downloadedFree.bytes).digest('hex')
  if (freeSHA256 !== issuedFree.document_sha256 || freeSHA256 !== downloadedFree.sha256) {
    throw new Error('下载免费授权与分发回执摘要不一致')
  }
  progress('policy', 'completed', `本地免费套餐与分发已就绪：${freeDistribution.snapshot.id}`)

  const entitlements = structuredClone(localLicensePolicies()[1].policy.entitlement_ceiling)
  entitlements.quotas.find(quota => quota.id === 'member_seats').limit.value = 20
  const definition = {
    product: 'aster-team', code: `local-demo-${state.id}`, name: '本地联调 20 席位',
    description: '仅用于本地开发验收，不是正式报价、收款或交付记录', edition: 'commercial',
    entitlements, quota_policy_version: 1, minimum_version: minimumVersion, transfer_limit: 2,
    support_terms_version: 'local-test-only', offer: {
      kind: 'annual', currency: 'CNY', annual_amount_minor: 599900, tax_mode: 'none',
      terms: [{ years: 1, discount_basis_points: 10000 }], term_rule: 'calendar_years_clamp_day', term_timezone: 'Asia/Shanghai',
    },
  }
  const plan = await write('/commercial/plans/versions', { operation_id: `local_plan_${state.id}`, plan_id: '', expected_version: 0, definition })
  const order = await write('/commercial/orders', {
    operation_id: `local_order_${state.id}`, customer_id: state.customer_id,
    plan_id: plan.snapshot.plan_id, plan_version: plan.snapshot.version, years: 1, starts_at: state.starts_at,
  })
  const orderPath = `/commercial/orders/${encodeURIComponent(order.snapshot.order_id)}`
  const payment = await write(`${orderPath}/payment`, {
    operation_id: `local_payment_${state.id}`, expected_order_sha256: order.sha256,
    payment_reference: `LOCAL-TEST-${state.id}`, received_at: state.starts_at,
    notes: '仅本地联调模拟全额到账，没有真实收款', current_password: password,
  })
  progress('order', 'completed', `本地测试套餐与订单已就绪：${order.snapshot.order_id}`)
  // Check the server-side environment, not a caller-provided flag, before signing.
  const context = await operations.request(`${orderPath}/fulfillment-context`)
  if (context.environment !== 'local') throw new Error('一键授权只允许履约环境为 local 的 Operations')
  const approved = await write(`${orderPath}/fulfillment`, {
    operation_id: `local_fulfillment_${state.id}`, expected_order_sha256: order.sha256,
    expected_payment_sha256: payment.sha256, license_request_json: requestJSON,
    reason: '本地开发控制台核对当前安装申请', current_password: password,
  })
  progress('policy', 'completed', `v2 履约已批准：${approved.snapshot.id}`)
  const issuer = approved.claims
    ? profiles.find(profile => profile.key_id === approved.claims.key_id && canIssue(profile, 'commercial_order', 'installation', 'fixed', entitlements))
    : profiles.find(profile => canIssue(profile, 'commercial_order', 'installation', 'fixed', entitlements))
  if (!issuer) throw new Error('没有与 Customer 当前公钥及权限范围一致的可用付费 v2 签发器')
  const path = `/commercial/paid-fulfillments/${encodeURIComponent(approved.snapshot.id)}`
  const issued = await write(`${path}/issue`, { key_id: issuer.key_id, current_password: password })
  const downloaded = await operations.request(`${path}/license`, { format: 'bytes' })
  const sha256 = createHash('sha256').update(downloaded.bytes).digest('hex')
  if (sha256 !== issued.document_sha256 || sha256 !== downloaded.sha256) throw new Error('下载授权与履约回执摘要不一致')
  return { free: { issued: issuedFree, bytes: downloadedFree.bytes, sha256: freeSHA256 }, issued, bytes: downloaded.bytes, sha256 }
}
