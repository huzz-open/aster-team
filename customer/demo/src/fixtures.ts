const timestamp = (day: number, hour = 9) => `2026-08-${String(day).padStart(2, '0')}T${String(hour).padStart(2, '0')}:00:00.000Z`

export const syntheticDemoProfile = 'website-media-synthetic-v1'

export const adminProfile = {
  email: 'demo-admin@example.invalid',
  display_name: 'Demo Administrator',
  password_change_required: false,
  license_state: 'active',
  license: { state: 'active', available: true, features: ['gateway', 'member', 'runner'] },
}

export const memberProfile = {
  id: 'demo-user-01',
  email: 'demo-user-01@example.invalid',
  display_name: 'Demo User 01',
  status: 'active',
  password_change_required: false,
  license_state: 'active',
  balance_tokens: 60_000_000,
  granted_tokens: 100_000_000,
  used_tokens: 40_000_000,
  request_count: 1_200,
  raw_tokens: 36_000_000,
  billed_tokens: 40_000_000,
  created_at: timestamp(3, 2),
}

export const adminOverview = {
  period: '14d',
  days: 14,
  current: { request_count: 3_100, raw_tokens: 106_000_000, billed_tokens: 120_000_000, uncovered_tokens: 0, uncached_input_tokens: 38_000_000, cached_input_tokens: 47_000_000, cache_write_tokens: 8_000_000, output_tokens: 13_000_000 },
  previous: { request_count: 2_800, raw_tokens: 94_000_000, billed_tokens: 105_000_000, uncovered_tokens: 0, uncached_input_tokens: 34_000_000, cached_input_tokens: 41_000_000, cache_write_tokens: 7_000_000, output_tokens: 12_000_000 },
  trend: [
    ['2026-08-20',5_000_000,120,'2026-08-06',4_000_000,100],['2026-08-21',6_000_000,150,'2026-08-07',5_000_000,125],
    ['2026-08-22',7_000_000,180,'2026-08-08',6_000_000,150],['2026-08-23',8_000_000,210,'2026-08-09',7_000_000,175],
    ['2026-08-24',9_000_000,240,'2026-08-10',8_000_000,200],['2026-08-25',10_000_000,270,'2026-08-11',9_000_000,225],
    ['2026-08-26',11_000_000,300,'2026-08-12',10_000_000,250],['2026-08-27',12_000_000,330,'2026-08-13',11_000_000,275],
    ['2026-08-28',11_000_000,300,'2026-08-14',10_000_000,250],['2026-08-29',10_000_000,270,'2026-08-15',9_000_000,225],
    ['2026-08-30',9_000_000,240,'2026-08-16',8_000_000,200],['2026-08-31',8_000_000,210,'2026-08-17',7_000_000,175],
    ['2026-09-01',7_000_000,180,'2026-08-18',6_000_000,150],['2026-09-02',6_000_000,150,'2026-08-19',5_000_000,125],
  ].map(([date, raw_tokens, request_count, previous_date, previous_raw_tokens, previous_request_count]) => ({
    date, previous_date,
    current: { request_count, raw_tokens, billed_tokens: raw_tokens, uncovered_tokens: 0, uncached_input_tokens: 0, cached_input_tokens: 0, cache_write_tokens: 0, output_tokens: 0 },
    previous: { request_count: previous_request_count, raw_tokens: previous_raw_tokens, billed_tokens: previous_raw_tokens, uncovered_tokens: 0, uncached_input_tokens: 0, cached_input_tokens: 0, cache_write_tokens: 0, output_tokens: 0 },
  })),
  top_members: [
    { user_id: 'demo-user-01', display_name: 'Demo User 01', email: 'demo-user-01@example.invalid', request_count: 1_200, raw_tokens: 36_000_000, billed_tokens: 40_000_000, uncovered_tokens: 0, uncached_input_tokens: 13_000_000, cached_input_tokens: 16_000_000, cache_write_tokens: 3_000_000, output_tokens: 4_000_000 },
    { user_id: 'demo-user-02', display_name: 'Demo User 02', email: 'demo-user-02@example.invalid', request_count: 900, raw_tokens: 28_000_000, billed_tokens: 32_000_000, uncovered_tokens: 0, uncached_input_tokens: 10_000_000, cached_input_tokens: 12_000_000, cache_write_tokens: 2_000_000, output_tokens: 4_000_000 },
    { user_id: 'demo-user-03', display_name: 'Demo User 03', email: 'demo-user-03@example.invalid', request_count: 600, raw_tokens: 21_000_000, billed_tokens: 24_000_000, uncovered_tokens: 0, uncached_input_tokens: 7_000_000, cached_input_tokens: 9_000_000, cache_write_tokens: 2_000_000, output_tokens: 3_000_000 },
    { user_id: 'demo-user-04', display_name: 'Demo User 04', email: 'demo-user-04@example.invalid', request_count: 300, raw_tokens: 14_000_000, billed_tokens: 16_000_000, uncovered_tokens: 0, uncached_input_tokens: 5_000_000, cached_input_tokens: 6_000_000, cache_write_tokens: 1_000_000, output_tokens: 2_000_000 },
    { user_id: 'demo-user-05', display_name: 'Demo User 05', email: 'demo-user-05@example.invalid', request_count: 100, raw_tokens: 7_000_000, billed_tokens: 8_000_000, uncovered_tokens: 0, uncached_input_tokens: 3_000_000, cached_input_tokens: 3_000_000, cache_write_tokens: 0, output_tokens: 1_000_000 },
  ],
  model_usage: [
    { model: 'gpt-5.6-luna', request_count: 1_400, raw_tokens: 50_000_000, billed_tokens: 56_000_000, uncovered_tokens: 0, uncached_input_tokens: 17_000_000, cached_input_tokens: 23_000_000, cache_write_tokens: 4_000_000, output_tokens: 6_000_000 },
    { model: 'gpt-5.4', request_count: 900, raw_tokens: 28_000_000, billed_tokens: 32_000_000, uncovered_tokens: 0, uncached_input_tokens: 10_000_000, cached_input_tokens: 12_000_000, cache_write_tokens: 2_000_000, output_tokens: 4_000_000 },
    { model: 'gpt-5.4-mini', request_count: 500, raw_tokens: 18_000_000, billed_tokens: 20_000_000, uncovered_tokens: 0, uncached_input_tokens: 7_000_000, cached_input_tokens: 7_000_000, cache_write_tokens: 1_000_000, output_tokens: 3_000_000 },
    { model: 'o4-mini', request_count: 300, raw_tokens: 10_000_000, billed_tokens: 12_000_000, uncovered_tokens: 0, uncached_input_tokens: 4_000_000, cached_input_tokens: 5_000_000, cache_write_tokens: 1_000_000, output_tokens: 0 },
  ],
  granted_tokens: 540_000_000,
  used_tokens: 216_000_000,
  balance_tokens: 324_000_000,
  licensed_seats: 12,
  period_active_users: 7,
  inactive_period_users: 2,
  request_count: 12_000,
  raw_tokens: 432_000_000,
  billed_tokens: 480_000_000,
  active_keys: 7,
  users: 9,
  active_users: 8,
  enabled_runners: 2,
  online_runners: 2,
  upstream_accounts: 3,
  active_upstream_accounts: 2,
  models: 4,
  enabled_models: 4,
}

export const members = [
  memberProfile,
  { id: 'demo-user-02', email: 'demo-user-02@example.invalid', display_name: 'Demo User 02', status: 'active', password_change_required: false, balance_tokens: 48_000_000, granted_tokens: 80_000_000, used_tokens: 32_000_000, request_count: 900, raw_tokens: 28_000_000, billed_tokens: 32_000_000, created_at: timestamp(5, 3) },
  { id: 'demo-user-03', email: 'demo-user-03@example.invalid', display_name: 'Demo User 03', status: 'active', password_change_required: false, balance_tokens: 36_000_000, granted_tokens: 60_000_000, used_tokens: 24_000_000, request_count: 600, raw_tokens: 21_000_000, billed_tokens: 24_000_000, created_at: timestamp(7, 4) },
  { id: 'demo-user-04', email: 'demo-user-04@example.invalid', display_name: 'Demo User 04', status: 'active', password_change_required: false, balance_tokens: 24_000_000, granted_tokens: 40_000_000, used_tokens: 16_000_000, request_count: 300, raw_tokens: 14_000_000, billed_tokens: 16_000_000, created_at: timestamp(9, 5) },
  { id: 'demo-user-05', email: 'demo-user-05@example.invalid', display_name: 'Demo User 05', status: 'active', password_change_required: false, balance_tokens: 12_000_000, granted_tokens: 20_000_000, used_tokens: 8_000_000, request_count: 100, raw_tokens: 7_000_000, billed_tokens: 8_000_000, created_at: timestamp(11, 6) },
  { id: 'demo-user-06', email: 'demo-user-06@example.invalid', display_name: 'Demo User 06', status: 'active', password_change_required: false, balance_tokens: 54_000_000, granted_tokens: 90_000_000, used_tokens: 36_000_000, request_count: 1_050, raw_tokens: 32_000_000, billed_tokens: 36_000_000, created_at: timestamp(13, 7) },
  { id: 'demo-user-07', email: 'demo-user-07@example.invalid', display_name: 'Demo User 07', status: 'active', password_change_required: false, balance_tokens: 42_000_000, granted_tokens: 70_000_000, used_tokens: 28_000_000, request_count: 780, raw_tokens: 25_000_000, billed_tokens: 28_000_000, created_at: timestamp(15, 8) },
  { id: 'demo-user-08', email: 'demo-user-08@example.invalid', display_name: 'Demo User 08', status: 'active', password_change_required: false, balance_tokens: 30_000_000, granted_tokens: 50_000_000, used_tokens: 20_000_000, request_count: 520, raw_tokens: 18_000_000, billed_tokens: 20_000_000, created_at: timestamp(17, 9) },
  { id: 'demo-user-09', email: 'demo-user-09@example.invalid', display_name: 'Demo User 09', status: 'disabled', password_change_required: false, balance_tokens: 18_000_000, granted_tokens: 30_000_000, used_tokens: 12_000_000, request_count: 240, raw_tokens: 11_000_000, billed_tokens: 12_000_000, created_at: timestamp(19, 10) },
]

export const memberKeys = [
  { id: 'demo-key-01', name: 'Demo key 01', key_prefix: 'aster_demo_0001', status: 'active', last_used_at: timestamp(29, 8), created_at: timestamp(6, 3) },
  { id: 'demo-key-02', name: 'Demo key 02', key_prefix: 'aster_demo_0002', status: 'active', last_used_at: timestamp(29, 6), created_at: timestamp(10, 4) },
  { id: 'demo-key-03', name: 'Demo key 03', key_prefix: 'aster_demo_0003', status: 'active', last_used_at: timestamp(28, 11), created_at: timestamp(14, 7) },
  { id: 'demo-key-04', name: 'Demo key 04', key_prefix: 'aster_demo_0004', status: 'active', last_used_at: timestamp(28, 9), created_at: timestamp(16, 8) },
  { id: 'demo-key-05', name: 'Demo key 05', key_prefix: 'aster_demo_0005', status: 'active', last_used_at: timestamp(27, 10), created_at: timestamp(18, 9) },
  { id: 'demo-key-06', name: 'Demo key 06', key_prefix: 'aster_demo_0006', status: 'active', last_used_at: timestamp(26, 7), created_at: timestamp(20, 10) },
  { id: 'demo-key-07', name: 'Demo key 07', key_prefix: 'aster_demo_0007', status: 'active', last_used_at: timestamp(25, 4), created_at: timestamp(22, 11) },
  { id: 'demo-key-08', name: 'Demo key 08', key_prefix: 'aster_demo_0008', status: 'revoked', last_used_at: timestamp(21, 5), created_at: timestamp(24, 8) },
]

export const memberModels = [
  { id: 'model-gpt-54', public_name: 'gpt-5.4', upstream_name: 'gpt-5.4', display_name: 'GPT-5.4', enabled: true, available: true, provider: 'OpenAI', discovered_at: timestamp(18, 2) },
  { id: 'model-gpt-54-mini', public_name: 'gpt-5.4-mini', upstream_name: 'gpt-5.4-mini', display_name: 'GPT-5.4 mini', enabled: true, available: true, provider: 'OpenAI', discovered_at: timestamp(18, 2) },
  { id: 'model-o4-mini', public_name: 'o4-mini', upstream_name: 'o4-mini', display_name: 'o4-mini', enabled: true, available: true, provider: 'OpenAI', discovered_at: timestamp(19, 3) },
  { id: 'model-codex', public_name: 'gpt-5.2-codex', upstream_name: 'gpt-5.2-codex', display_name: 'GPT-5.2 Codex', enabled: true, available: true, provider: 'OpenAI', discovered_at: timestamp(20, 4) },
]

export const memberDocs = {
  public_api_base_url: 'https://ai.example.invalid',
  base_urls: {
    openai: 'https://ai.example.invalid/v1',
    anthropic: 'https://ai.example.invalid/anthropic',
  },
  codex_config: `model_provider = "aster"

[model_providers]

[model_providers.aster]
base_url = "https://ai.example.invalid/v1"
env_key = "ASTER_API_KEY"
name = "Aster Team"
wire_api = "responses"

[model_providers.aster.http_headers]
x-openai-actor-authorization = "aster-proxy"
`,
  usage_multiplier: 1,
}

type UsagePoint = {
  date: string
  request_count: number
  raw_tokens: number
  billed_tokens: number
  uncached_input_tokens: number
  cached_input_tokens: number
  cache_write_tokens: number
  output_tokens: number
}

const dailyUsage: UsagePoint[] = [
  ['2026-08-28', 180, 4_500_000, 5_000_000, 1_700_000, 1_900_000, 400_000, 500_000],
  ['2026-08-29', 220, 6_300_000, 7_000_000, 2_300_000, 2_700_000, 500_000, 800_000],
  ['2026-08-30', 200, 5_400_000, 6_000_000, 2_000_000, 2_300_000, 400_000, 700_000],
  ['2026-08-31', 260, 7_200_000, 8_000_000, 2_600_000, 3_100_000, 600_000, 900_000],
  ['2026-09-01', 250, 6_300_000, 7_000_000, 2_300_000, 2_700_000, 500_000, 800_000],
  ['2026-09-02', 300, 8_100_000, 9_000_000, 2_900_000, 3_500_000, 700_000, 1_000_000],
  ['2026-09-03', 340, 6_300_000, 7_000_000, 2_300_000, 2_700_000, 500_000, 800_000],
].map(([date, request_count, raw_tokens, billed_tokens, uncached_input_tokens, cached_input_tokens, cache_write_tokens, output_tokens]) => ({
  date: String(date), request_count: Number(request_count), raw_tokens: Number(raw_tokens), billed_tokens: Number(billed_tokens), uncached_input_tokens: Number(uncached_input_tokens), cached_input_tokens: Number(cached_input_tokens), cache_write_tokens: Number(cache_write_tokens), output_tokens: Number(output_tokens),
}))

const fiveMinuteDistribution = [0, .04, .08, .12, .17, .19, .15, .11, .07, .04, .02, .01]
const fiveMinuteUsage: UsagePoint[] = [58_000, 72_000, 66_000, 94_000, 82_000, 118_000, 104_000, 136_000, 122_000, 154_000, 142_000, 176_000, 168_000, 194_000, 182_000, 216_000, 205_000, 238_000, 224_000, 252_000, 236_000, 268_000, 248_000, 286_000].flatMap((hourTotal, hourIndex) => {
  let allocated = 0
  return fiveMinuteDistribution.map((share, bucketIndex) => {
    const billed_tokens = bucketIndex === fiveMinuteDistribution.length - 1
      ? hourTotal - allocated
      : Math.round(hourTotal * share)
    allocated += billed_tokens
    const instant = new Date(Date.UTC(2026, 8, 2, 3 + hourIndex, bucketIndex * 5))
    const raw_tokens = Math.round(billed_tokens * .94)
    return {
      date: instant.toISOString().replace('.000Z', 'Z'),
      request_count: billed_tokens ? Math.max(1, Math.round(billed_tokens / 13_500)) : 0,
      raw_tokens,
      billed_tokens,
      uncached_input_tokens: Math.round(raw_tokens * .42),
      cached_input_tokens: Math.round(raw_tokens * .24),
      cache_write_tokens: Math.round(raw_tokens * .08),
      output_tokens: Math.round(raw_tokens * .26),
    }
  })
})

const sumUsage = (items: UsagePoint[]): UsagePoint => items.reduce<UsagePoint>((total, item) => ({
  date: '2026-08-28/2026-09-03',
  request_count: total.request_count + item.request_count,
  raw_tokens: total.raw_tokens + item.raw_tokens,
  billed_tokens: total.billed_tokens + item.billed_tokens,
  uncached_input_tokens: total.uncached_input_tokens + item.uncached_input_tokens,
  cached_input_tokens: total.cached_input_tokens + item.cached_input_tokens,
  cache_write_tokens: total.cache_write_tokens + item.cache_write_tokens,
  output_tokens: total.output_tokens + item.output_tokens,
}), { date: '', request_count: 0, raw_tokens: 0, billed_tokens: 0, uncached_input_tokens: 0, cached_input_tokens: 0, cache_write_tokens: 0, output_tokens: 0 })

const hourlyUsage = Array.from({ length: 24 }, (_, hourIndex) => {
  const points = fiveMinuteUsage.slice(hourIndex * 12, hourIndex * 12 + 12)
  return { ...sumUsage(points), date: points[0]?.date ?? '' }
})
const hourlyTotal = sumUsage(hourlyUsage)

const modelUsage = [
  { model: 'gpt-5.4', request_count: 700, raw_tokens: 18_000_000, billed_tokens: 20_000_000 },
  { model: 'gpt-5.4-mini', request_count: 500, raw_tokens: 12_600_000, billed_tokens: 14_000_000 },
  { model: 'o4-mini', request_count: 350, raw_tokens: 8_100_000, billed_tokens: 9_000_000 },
  { model: 'gpt-5.2-codex', request_count: 200, raw_tokens: 5_400_000, billed_tokens: 6_000_000 },
]

const modelTrend = modelUsage.flatMap((model, modelIndex) => dailyUsage.map((day, dayIndex) => {
  const share = [0.38, 0.27, 0.2, 0.15][modelIndex] ?? 0.1
  const variation = 0.94 + dayIndex * 0.02
  return {
    ...day,
    model: model.model,
    request_count: Math.round(day.request_count * share),
    raw_tokens: Math.round(day.raw_tokens * share * variation),
    billed_tokens: Math.round(day.billed_tokens * share * variation),
    uncached_input_tokens: Math.round(day.uncached_input_tokens * share * variation),
    cached_input_tokens: Math.round(day.cached_input_tokens * share * variation),
    cache_write_tokens: Math.round(day.cache_write_tokens * share * variation),
    output_tokens: Math.round(day.output_tokens * share * variation),
  }
}))

export const usageSummary = {
  period: '7d',
  granularity: 'day',
  bucket_minutes: null,
  range_start: '2026-08-28T00:00:00+08:00',
  range_end: '2026-09-04T00:00:00+08:00',
  observed_until: '2026-09-04T00:00:00+08:00',
  utc_offset_minutes: 480,
  summary: { ...sumUsage(dailyUsage), uncovered_tokens: 0 },
  trend: dailyUsage,
  models: modelUsage.map((item, index) => ({
    ...sumUsage(dailyUsage.map(day => ({
      ...day,
      request_count: Math.round(day.request_count * ([0.38, 0.27, 0.2, 0.15][index] ?? 0.1)),
      raw_tokens: Math.round(day.raw_tokens * ([0.38, 0.27, 0.2, 0.15][index] ?? 0.1)),
      billed_tokens: Math.round(day.billed_tokens * ([0.38, 0.27, 0.2, 0.15][index] ?? 0.1)),
    }))),
    ...item,
  })),
  model_trend: modelTrend,
  recent: [
    { id: 'demo-usage-001', kind: 'usage', amount_tokens: -60_000, uncached_input_tokens: 24_000, cached_input_tokens: 18_000, cache_write_tokens: 6_000, output_tokens: 12_000, uncovered_tokens: 0, raw_tokens: 60_000, billed_tokens: 60_000, multiplier: 1, protocol: 'openai-responses', model: 'gpt-5.4', api_key_id: 'demo-key-01', runner_id: 'demo-runner-01', description: 'Demo workload 01', created_at: timestamp(29, 8) },
    { id: 'demo-usage-002', kind: 'usage', amount_tokens: -40_000, uncached_input_tokens: 16_000, cached_input_tokens: 12_000, cache_write_tokens: 4_000, output_tokens: 8_000, uncovered_tokens: 0, raw_tokens: 40_000, billed_tokens: 40_000, multiplier: 1, protocol: 'openai-responses', model: 'gpt-5.4-mini', api_key_id: 'demo-key-02', runner_id: 'demo-runner-02', description: 'Demo workload 02', created_at: timestamp(29, 7) },
    { id: 'demo-usage-003', kind: 'usage', amount_tokens: -20_000, uncached_input_tokens: 8_000, cached_input_tokens: 6_000, cache_write_tokens: 2_000, output_tokens: 4_000, uncovered_tokens: 0, raw_tokens: 20_000, billed_tokens: 20_000, multiplier: 1, protocol: 'anthropic-messages', model: 'o4-mini', api_key_id: 'demo-key-03', runner_id: 'demo-runner-01', description: 'Demo workload 03', created_at: timestamp(29, 6) },
  ],
}

const previousDailyUsage = dailyUsage.map((item, index) => {
  const instant = new Date(`${item.date}T00:00:00.000Z`)
  instant.setUTCDate(instant.getUTCDate() - 7)
  const scale = [0.82, 0.88, 0.84, 0.91, 0.86, 0.93, 0.89][index] ?? 0.86
  return {
    ...item,
    date: instant.toISOString().slice(0, 10),
    request_count: Math.round(item.request_count * scale),
    raw_tokens: Math.round(item.raw_tokens * scale),
    billed_tokens: Math.round(item.billed_tokens * scale),
    uncached_input_tokens: Math.round(item.uncached_input_tokens * scale),
    cached_input_tokens: Math.round(item.cached_input_tokens * scale),
    cache_write_tokens: Math.round(item.cache_write_tokens * scale),
    output_tokens: Math.round(item.output_tokens * scale),
  }
})

export const previousUsageSummary = {
  ...usageSummary,
  range_start: '2026-08-21T00:00:00+08:00',
  range_end: '2026-08-28T00:00:00+08:00',
  observed_until: '2026-08-28T00:00:00+08:00',
  summary: { ...sumUsage(previousDailyUsage), uncovered_tokens: 0 },
  trend: previousDailyUsage,
  model_trend: modelTrend.map(item => {
    const instant = new Date(`${item.date}T00:00:00.000Z`)
    instant.setUTCDate(instant.getUTCDate() - 7)
    return { ...item, date: instant.toISOString().slice(0, 10) }
  }),
}

export const hourlyUsageSummary = {
  ...usageSummary,
  period: '1d',
  granularity: 'hour',
  bucket_minutes: 60,
  range_start: '2026-09-02T11:00:00+08:00',
  range_end: '2026-09-03T11:00:00+08:00',
  observed_until: '2026-09-03T11:00:00+08:00',
  summary: { ...hourlyTotal, uncovered_tokens: 0 },
  trend: hourlyUsage,
  models: modelUsage.map((item, index) => ({
    ...item,
    request_count: Math.round(hourlyTotal.request_count * ([.46, .29, .16, .09][index] ?? 0)),
    raw_tokens: Math.round(hourlyTotal.raw_tokens * ([.46, .29, .16, .09][index] ?? 0)),
    billed_tokens: Math.round(hourlyTotal.billed_tokens * ([.46, .29, .16, .09][index] ?? 0)),
  })),
  recent: usageSummary.recent.map((item, index) => ({
    ...item,
    created_at: `2026-09-03T${String(6 - index).padStart(2, '0')}:2${index}:00+08:00`,
  })),
}

function requireSynthetic(condition: boolean, field: string): void {
  if (!condition) throw new Error(`Unsafe demo fixture data: ${field}`)
}

export function assertSyntheticDemoFixtures(): void {
  requireSynthetic(adminProfile.email === 'demo-admin@example.invalid', 'admin email must use the reserved invalid domain')
  requireSynthetic(adminProfile.display_name === 'Demo Administrator', 'admin display name must be explicitly synthetic')

  const expectedUsers = members.map((_, index) => {
    const suffix = String(index + 1).padStart(2, '0')
    return {
      id: `demo-user-${suffix}`,
      email: `demo-user-${suffix}@example.invalid`,
      displayName: `Demo User ${suffix}`,
    }
  })
  members.forEach((member, index) => {
    const expected = expectedUsers[index]!
    requireSynthetic(member.id === expected.id, `member ${index + 1} id must be explicitly synthetic`)
    requireSynthetic(member.email === expected.email, `member ${index + 1} email must use the reserved invalid domain`)
    requireSynthetic(member.display_name === expected.displayName, `member ${index + 1} display name must be explicitly synthetic`)
  })
  adminOverview.top_members.forEach((member, index) => {
    const expected = expectedUsers[index]!
    requireSynthetic(member.user_id === expected.id, `ranking member ${index + 1} id must be explicitly synthetic`)
    requireSynthetic(member.email === expected.email, `ranking member ${index + 1} email must use the reserved invalid domain`)
    requireSynthetic(member.display_name === expected.displayName, `ranking member ${index + 1} display name must be explicitly synthetic`)
  })

  memberKeys.forEach((key, index) => {
    const suffix = String(index + 1).padStart(2, '0')
    requireSynthetic(key.id === `demo-key-${suffix}`, `key ${index + 1} id must be explicitly synthetic`)
    requireSynthetic(key.name === `Demo key ${suffix}`, `key ${index + 1} name must be explicitly synthetic`)
    requireSynthetic(key.key_prefix === `aster_demo_${String(index + 1).padStart(4, '0')}`, `key ${index + 1} prefix must be explicitly synthetic`)
  })

  requireSynthetic(new URL(memberDocs.public_api_base_url).hostname.endsWith('.invalid'), 'public API URL must use the reserved invalid domain')
  Object.entries(memberDocs.base_urls).forEach(([name, url]) => {
    requireSynthetic(new URL(url).hostname.endsWith('.invalid'), `${name} API URL must use the reserved invalid domain`)
  })
  requireSynthetic(memberDocs.codex_config.includes('https://ai.example.invalid/v1'), 'Codex example must use the reserved invalid domain')
  requireSynthetic(usageSummary.recent.every(item => item.id.startsWith('demo-') && item.api_key_id.startsWith('demo-') && item.runner_id.startsWith('demo-') && item.description.startsWith('Demo ')), 'recent usage identities must be explicitly synthetic')
}

assertSyntheticDemoFixtures()
