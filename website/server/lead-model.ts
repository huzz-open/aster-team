export type UsageEvidence = 'tokens' | 'time'
export type DailyAiTime = 'under1h' | '1h-4h' | '4h-8h' | 'over8h'
export type LeadLocale = 'zh' | 'en'

export type ParsedLead = {
  contact: string
  company: string
  teamSize: number
  activeUsers: number
  evidence: UsageEvidence
  weeklyTokens: number | null
  dailyTime: DailyAiTime | null
  usageLevel: 0 | 1 | 2
  recommendedProAccounts: number
  locale: LeadLocale
  turnstileToken: string
  honeypotTriggered: boolean
}

export class InvalidLeadError extends Error {
  constructor(message: string) {
    super(message)
    this.name = 'InvalidLeadError'
  }
}

const dailyTimes = new Set<DailyAiTime>(['under1h', '1h-4h', '4h-8h', 'over8h'])
const proWeeklyTokenCapacity = 32
const normalUsersPerPro = 3

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function readString(value: unknown, field: string, min: number, max: number): string {
  if (typeof value !== 'string') throw new InvalidLeadError(`${field} must be a string`)
  const normalized = value.trim()
  if (normalized.length < min || normalized.length > max) throw new InvalidLeadError(`${field} has an invalid length`)
  return normalized
}

function readInteger(value: unknown, field: string, min: number, max: number): number {
  const parsed = Number(value)
  if (!Number.isInteger(parsed) || parsed < min || parsed > max) throw new InvalidLeadError(`${field} is out of range`)
  return parsed
}

function readFiniteNumber(value: unknown, field: string, min: number, max: number): number {
  const parsed = Number(value)
  if (!Number.isFinite(parsed) || parsed < min || parsed > max) throw new InvalidLeadError(`${field} is out of range`)
  return parsed
}

export function assessUsage(
  activeUsers: number,
  evidence: UsageEvidence,
  weeklyTokens: number | null,
  dailyTime: DailyAiTime | null,
): { level: 0 | 1 | 2; recommended: number } {
  const baselineAccounts = Math.max(1, Math.ceil(activeUsers / normalUsersPerPro))
  if (evidence === 'tokens') {
    if (weeklyTokens === null) throw new InvalidLeadError('weeklyTokens is required for token evidence')
    const tokenAccounts = Math.max(1, Math.ceil(weeklyTokens / proWeeklyTokenCapacity))
    const loadRatio = weeklyTokens / (baselineAccounts * proWeeklyTokenCapacity)
    const level: 0 | 1 | 2 = loadRatio < 0.35 ? 0 : loadRatio <= 1 ? 1 : 2
    return { level, recommended: Math.max(baselineAccounts, tokenAccounts) }
  }

  if (dailyTime === null) throw new InvalidLeadError('dailyTime is required for time evidence')
  const level: 0 | 1 | 2 = dailyTime === 'under1h' ? 0 : dailyTime === 'over8h' ? 2 : 1
  const usersPerAccount = level === 2 ? 2 : normalUsersPerPro
  return { level, recommended: Math.max(1, Math.ceil(activeUsers / usersPerAccount)) }
}

export function parseLeadPayload(value: unknown): ParsedLead {
  if (!isRecord(value)) throw new InvalidLeadError('request body must be an object')

  const contact = readString(value.contact, 'contact', 2, 200)
  const company = value.company === undefined || value.company === null || value.company === ''
    ? ''
    : readString(value.company, 'company', 1, 120)
  const teamSize = readInteger(value.teamSize, 'teamSize', 1, 500)
  const activeUsers = readInteger(value.activeUsers, 'activeUsers', 1, teamSize)
  if (value.evidence !== 'tokens' && value.evidence !== 'time') throw new InvalidLeadError('evidence is invalid')
  const evidence = value.evidence
  const weeklyTokens = evidence === 'tokens'
    ? readFiniteNumber(value.weeklyTokens, 'weeklyTokens', 0.1, 100000)
    : null
  const dailyTimeValue = value.dailyTime
  const dailyTime = evidence === 'time' && typeof dailyTimeValue === 'string' && dailyTimes.has(dailyTimeValue as DailyAiTime)
    ? dailyTimeValue as DailyAiTime
    : null
  if (evidence === 'time' && dailyTime === null) throw new InvalidLeadError('dailyTime is invalid')
  const locale: LeadLocale = value.locale === 'en' ? 'en' : 'zh'
  const turnstileToken = readString(value.turnstileToken, 'turnstileToken', 1, 2048)
  const honeypotTriggered = typeof value.website === 'string' && value.website.trim().length > 0
  const usage = assessUsage(activeUsers, evidence, weeklyTokens, dailyTime)

  return {
    contact,
    company,
    teamSize,
    activeUsers,
    evidence,
    weeklyTokens,
    dailyTime,
    usageLevel: usage.level,
    recommendedProAccounts: usage.recommended,
    locale,
    turnstileToken,
    honeypotTriggered,
  }
}
