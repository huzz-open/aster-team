import { describe, expect, test } from 'vitest'
import { isSemanticVersion } from '../src/release-version'

describe('release version validation', () => {
  test.each([
    '0.0.0',
    '2.0.0',
    '1.0.0-alpha',
    '1.0.0-alpha.1',
    '1.0.0+20130313144700',
    '1.0.0-beta.2+exp.sha.5114f85',
  ])('accepts SemVer 2.0.0 version %s', version => {
    expect(isSemanticVersion(version)).toBe(true)
  })

  test.each([
    '',
    'v1.2.3',
    '1.2',
    '01.2.3',
    '1.02.3',
    '1.2.03',
    '1.0.0-',
    '1.0.0-alpha.01',
    '1.0.0+',
    '1.0.0+build..1',
    '1.0.0\n',
    `1.0.0+${'a'.repeat(65)}`,
  ])('rejects non-SemVer version %s', version => {
    expect(isSemanticVersion(version)).toBe(false)
  })
})
