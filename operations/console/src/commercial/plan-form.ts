import type { CommercialPlanDefinition } from '../api/client'
import { CAPABILITIES, CAPABILITY_CATALOG_VERSION, QUOTAS, type FeatureSetId, type CapabilityId, type QuotaId } from '../api/generated/product-capabilities'

export type PlanForm = {
  code: string; name: string; description: string; edition: string
  minimumVersion: string; transferLimit: string; supportTermsVersion: string
  features: CapabilityId[]
  featureSets: FeatureSetId[]
  quotas: { id: QuotaId; mode: '' | 'limited' | 'unlimited'; value: string }[]
  kind: '' | 'annual' | 'free' | 'contact'
  currency: string; annualAmountMinor: string; taxMode: '' | 'inclusive' | 'none'
  timezone: '' | 'UTC' | 'Asia/Shanghai'
  terms: { years: number; enabled: boolean; discountPercent: string }[]
  expiryMode: '' | 'none' | 'fixed'; expiresAt: string
}

export function emptyPlanForm(): PlanForm {
  return {
    code: '', name: '', description: '', edition: '', minimumVersion: '', transferLimit: '', supportTermsVersion: '',
    features: [], featureSets: [], quotas: QUOTAS.map(({ id }) => ({ id, mode: '', value: '' })),
    kind: '', currency: '', annualAmountMinor: '', taxMode: '', timezone: '',
    terms: [1, 2, 3, 4, 5].map(years => ({ years, enabled: false, discountPercent: '' })),
    expiryMode: '', expiresAt: '',
  }
}

export function formFromDefinition(definition: CommercialPlanDefinition): PlanForm {
  const form = emptyPlanForm()
  Object.assign(form, {
    code: definition.code, name: definition.name, description: definition.description, edition: definition.edition,
    minimumVersion: definition.minimum_version, transferLimit: String(definition.transfer_limit), supportTermsVersion: definition.support_terms_version,
    features: [...definition.entitlements.features],
    featureSets: [...(definition.entitlements.feature_sets ?? [])],
    quotas: definition.entitlements.quotas.map(({ id, limit }) => ({ id, mode: limit.mode, value: limit.mode === 'limited' ? String(limit.value) : '' })),
    kind: definition.offer.kind,
  })
  const offer = definition.offer
  if (offer.kind === 'annual') {
    Object.assign(form, { currency: offer.currency, annualAmountMinor: String(offer.annual_amount_minor), taxMode: offer.tax_mode, timezone: offer.term_timezone })
    for (const term of form.terms) {
      const value = offer.terms.find(item => item.years === term.years)
      if (value) { term.enabled = true; term.discountPercent = (value.discount_basis_points / 100).toFixed(2) }
    }
  } else if (offer.kind === 'free') {
    form.expiryMode = offer.expiry.mode
    if (offer.expiry.mode === 'fixed') form.expiresAt = offer.expiry.expires_at
  }
  return form
}

export function unsignedInteger(text: string, label: string, maximum: number, minimum = 0): number {
  if (!/^(0|[1-9][0-9]*)$/.test(text)) throw new Error(`${label}必须是整数`)
  const value = Number(text)
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) throw new Error(`${label}须在 ${minimum} 至 ${maximum} 之间`)
  return value
}

export function discountBasisPoints(text: string): number {
  if (!/^(?:0|[1-9][0-9]{0,2})(?:\.[0-9]{1,2})?$/.test(text)) throw new Error('折扣比例最多保留两位小数')
  const [whole, fraction = ''] = text.split('.')
  const value = Number(whole) * 100 + Number(fraction.padEnd(2, '0'))
  if (value < 1 || value > 10000) throw new Error('折扣比例须在 0.01% 至 100% 之间')
  return value
}

// Presentation only; the server derives and fixes the actual order amount.
export function termAmount(annualMinor: number, years: number, basisPoints: number): number {
  const value = (BigInt(annualMinor) * BigInt(years) * BigInt(basisPoints) + 5000n) / 10000n
  if (value < 1n || value > 9_000_000_000_000n) throw new Error('总金额超出支持范围')
  return Number(value)
}

export function formatAmount(minor: number, currency: string): string {
  // Prices use explicit minor units. Do not assume every ISO currency has cents.
  try {
    const format = new Intl.NumberFormat('zh-CN', { style: 'currency', currency })
    const digits = format.resolvedOptions().maximumFractionDigits ?? 2
    return format.format(minor / 10 ** digits)
  } catch { return `${minor.toLocaleString('zh-CN')} ${currency} 最小单位` }
}

export function toggleCapability(selected: CapabilityId[], id: CapabilityId, enabled: boolean): CapabilityId[] {
  const features = CAPABILITIES as readonly { id: CapabilityId; requires: readonly CapabilityId[] }[]
  const result = new Set(selected)
  if (enabled) {
    const include = (key: CapabilityId) => {
      if (result.has(key)) return
      result.add(key)
      for (const dependency of features.find(item => item.id === key)!.requires) include(dependency)
    }
    include(id)
  } else {
    result.delete(id)
    // Remove dependents as well, including transitive ones.
    for (let pass = 0; pass < features.length; pass++) {
      for (const feature of features) if (feature.requires.some(key => !result.has(key))) result.delete(feature.id)
    }
  }
  return features.filter(feature => result.has(feature.id)).map(feature => feature.id)
}

export function definitionFromForm(form: PlanForm): CommercialPlanDefinition {
  const identity = (text: string, label: string) => {
    if (!/^[A-Za-z0-9._:@+/-]{1,128}$/.test(text)) throw new Error(`${label}须为 1 至 128 位英文字符或标识符`)
    return text
  }
  if (!form.name.trim()) throw new Error('请填写套餐名称')
  if (!form.minimumVersion.trim()) throw new Error('请填写最低客户版本')
  const quotas = QUOTAS.map(({ id, label }) => {
    const quota = form.quotas.find(item => item.id === id)
    if (quota?.mode === 'unlimited') return { id, limit: { mode: 'unlimited' as const } }
    if (quota?.mode === 'limited') return { id, limit: { mode: 'limited' as const, value: unsignedInteger(quota.value, label, 4294967295) } }
    throw new Error(`请选择${label}的限制方式`)
  })
  let offer: CommercialPlanDefinition['offer']
  if (form.kind === 'annual') {
    if (!/^[A-Z]{3}$/.test(form.currency)) throw new Error('请填写三位大写货币代码')
    if (!form.taxMode || !form.timezone) throw new Error('请选择税费方式和合同日历时区')
    const annualAmountMinor = unsignedInteger(form.annualAmountMinor, '年度金额', 9_000_000_000_000, 1)
    const terms = form.terms.filter(term => term.enabled).map(term => ({ years: term.years, discount_basis_points: discountBasisPoints(term.discountPercent) }))
    if (!terms.length) throw new Error('至少选择一个可售期限')
    for (const term of terms) termAmount(annualAmountMinor, term.years, term.discount_basis_points)
    offer = { kind: 'annual', currency: form.currency, annual_amount_minor: annualAmountMinor, tax_mode: form.taxMode, terms, term_rule: 'calendar_years_clamp_day', term_timezone: form.timezone }
  } else if (form.kind === 'free') {
    if (form.featureSets.length) throw new Error('免费套餐只能逐项授予功能 请取消功能集合')
    if (!form.expiryMode) throw new Error('请选择免费授权的到期方式')
    if (form.expiryMode === 'fixed') {
      const canonical = form.expiresAt.includes('.') ? form.expiresAt : form.expiresAt.replace('Z', '.000Z')
      if (!/^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.[0-9]{3})?Z$/.test(form.expiresAt) || !Number.isFinite(Date.parse(form.expiresAt)) || new Date(form.expiresAt).toISOString() !== canonical) throw new Error('免费到期时间须为有效的 UTC 时间 如 2027-01-01T00:00:00Z')
      offer = { kind: 'free', expiry: { mode: 'fixed', expires_at: canonical } }
    } else offer = { kind: 'free', expiry: { mode: 'none' } }
  } else if (form.kind === 'contact') offer = { kind: 'contact' }
  else throw new Error('请选择报价方式')
  return {
    product: 'aster-team', code: identity(form.code, '套餐代码'), name: form.name.trim(), description: form.description.trim(), edition: identity(form.edition, '授权版本'),
    minimum_version: form.minimumVersion.trim(), transfer_limit: unsignedInteger(form.transferLimit, '换机次数', 10000), support_terms_version: identity(form.supportTermsVersion, '支持条款版本'),
    entitlements: { catalog_version: CAPABILITY_CATALOG_VERSION, features: [...form.features], ...(form.featureSets.length ? { feature_sets: [...form.featureSets] } : {}), quotas }, quota_policy_version: 1, offer,
  }
}
