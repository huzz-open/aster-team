/** Formats a quota amount using units people naturally read in the active language. */
export function formatNaturalTokenAmount(value: number, locale = 'zh-CN'): string {
  const chinese = locale.toLowerCase().startsWith('zh')
  if (!Number.isSafeInteger(value) || value < 1 || value > 10_000_000_000) {
    return chinese ? '1～100 亿 Token' : '1–10 billion tokens'
  }
  const units = chinese
    ? [{ size: 100_000_000, label: '亿' }, { size: 10_000, label: '万' }]
    : [{ size: 1_000_000_000, label: 'billion' }, { size: 1_000_000, label: 'million' }, { size: 1_000, label: 'thousand' }]
  const unit = units.find(candidate => value >= candidate.size)
  if (!unit) return `${new Intl.NumberFormat(locale).format(value)} ${chinese ? 'Token' : value === 1 ? 'token' : 'tokens'}`
  const amount = new Intl.NumberFormat(locale, { maximumFractionDigits: 2 }).format(value / unit.size)
  return chinese ? `${amount} ${unit.label} Token` : `${amount} ${unit.label} tokens`
}
