import { describe, expect, it } from 'vitest'
import {
  adminOverview,
  adminProfile,
  assertSyntheticDemoFixtures,
  memberDocs,
  hourlyUsageSummary,
  memberKeys,
  members,
  previousUsageSummary,
  syntheticDemoProfile,
  usageSummary,
} from '../../demo/src/fixtures'

describe('website media demo fixtures', () => {
  it('uses only explicitly synthetic identities and reserved domains', () => {
    expect(assertSyntheticDemoFixtures).not.toThrow()
    expect(syntheticDemoProfile).toBe('website-media-synthetic-v1')
    expect(adminProfile.email).toBe('demo-admin@example.invalid')
    expect(members.every(member => member.display_name.startsWith('Demo User '))).toBe(true)
    expect(members.every(member => member.email.endsWith('@example.invalid'))).toBe(true)
    expect(adminOverview.top_members.every(member => member.email.endsWith('@example.invalid'))).toBe(true)
    expect(memberKeys.every(key => key.key_prefix.startsWith('aster_demo_'))).toBe(true)
    expect(new URL(memberDocs.public_api_base_url).hostname.endsWith('.invalid')).toBe(true)
    expect(usageSummary.recent.every(item => item.description.startsWith('Demo '))).toBe(true)
  })

  it('keeps website scenes visually populated', () => {
    expect(members).toHaveLength(9)
    expect(members.filter(member => member.status === 'active')).toHaveLength(8)
    expect(memberKeys.filter(key => key.status === 'active')).toHaveLength(7)
    expect(usageSummary.trend).toHaveLength(7)
    expect(previousUsageSummary.trend).toHaveLength(7)
    expect(previousUsageSummary.trend.at(-1)?.date).toBe('2026-08-27')
    expect(new Set(usageSummary.trend.map(item => item.raw_tokens)).size).toBeGreaterThan(3)
    expect(hourlyUsageSummary.trend).toHaveLength(24)
    expect(hourlyUsageSummary.trend.every(item => item.raw_tokens > 0)).toBe(true)
  })
})
