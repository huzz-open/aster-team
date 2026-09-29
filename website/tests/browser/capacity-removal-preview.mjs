import { expect } from '@playwright/test'

export async function checkCapacityRemoval(browser, origin) {
  const page = await browser.newPage({ reducedMotion: 'reduce', viewport: { width: 1920, height: 1080 } })
  const rateRequests = []
  page.on('request', request => {
    if (/apizero|frankfurter|currencyexchangetool|exchangerate\.fun/.test(request.url())) rateRequests.push(request.url())
  })
  await page.route('https://api.github.com/**', route => route.fulfill({ status: 404, json: {} }))
  await page.addInitScript(() => localStorage.setItem('aster-team:trial-lead-draft:v1', JSON.stringify({
    contact: 'acceptance@example.invalid', company: 'Northstar', teamSize: 12, activeUsers: 5,
    evidence: 'tokens', weeklyTokens: 32, dailyTime: '4h-8h',
  })))
  try {
    await page.goto(origin)
    const form = page.locator('.lead-form')
    for (const locale of ['en', 'zh']) {
      await page.getByRole('button', { name: locale === 'en' ? 'Switch to English' : '切换为简体中文', exact: true }).click()
      await expect(page.locator('.calculator-section,.usage-assessment,#section-5')).toHaveCount(0)
      await expect(page.locator('.site-header nav')).not.toContainText(/Capacity|容量测算/)
      await expect(form.locator('input[autocomplete="email tel"]')).toHaveValue('acceptance@example.invalid')
      await form.locator('label:has(input[value="tokens"])').click()
      await expect(form.locator('input[value="tokens"]')).toBeChecked()
      await expect(form.locator('.lead-token-field input')).toHaveValue('32')
      await expect(form).not.toContainText(/Pro 20x|3\.2B|32 亿|每 3 名|capacity estimate/)
      await form.locator('label:has(input[value="time"])').click()
      await expect(form.locator('input[value="time"]')).toBeChecked()
      await expect(form.getByRole('combobox')).toHaveText(locale === 'en' ? '4–8 hours' : '4–8 小时')
      expect(await page.evaluate(() => JSON.parse(localStorage.getItem('aster-team:trial-lead-draft:v1')).activeUsers)).toBe(5)
      await page.locator('.trial-layout').screenshot({ path: `target/website-review/team-form-${locale}.png` })
    }
    expect(rateRequests).toEqual([])
  } finally { await page.close() }
}
