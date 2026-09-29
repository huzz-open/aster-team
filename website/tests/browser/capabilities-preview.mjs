import { expect } from '@playwright/test'

export async function checkCapabilities(browser, origin) {
  const page = await browser.newPage({ reducedMotion: 'reduce' })
  const media = []
  page.on('request', request => { if (/\/product-media\//.test(request.url())) media.push(request.url()) })
  await page.route('https://api.github.com/**', route => route.fulfill({ status: 404, json: {} }))
  try {
    await page.goto(origin)
    const section = page.locator('.capability-section')
    for (const width of [1440, 768, 390]) {
      await page.setViewportSize({ width, height: 1000 })
      for (const locale of ['en', 'zh']) {
        await page.getByRole('button', { name: locale === 'en' ? 'Switch to English' : '切换为简体中文', exact: true }).click()
        await section.scrollIntoViewIfNeeded()
        await expect(section.locator('.capability-list article')).toHaveCount(6)
        await expect(section.locator('.capability-roles article')).toHaveCount(2)
        await expect(section.locator('img,video,.product-walkthrough,.product-gallery')).toHaveCount(0)
        await expect(section).toContainText(locale === 'zh' ? '成员独立使用' : 'Independent access for members')
        await expect(section).toContainText(locale === 'zh' ? '管理员统一管理' : 'Central control for administrators')
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
        expect(await section.locator('.capability-list p').first().evaluate(element => getComputedStyle(element).fontSize)).toBe('16px')
        if (width === 1440) {
          const tops = await section.locator('.capability-list h3').evaluateAll(elements => elements.map(element => element.getBoundingClientRect().top))
          expect(Math.max(...tops.slice(0, 3)) - Math.min(...tops.slice(0, 3))).toBeLessThan(1)
          expect(Math.max(...tops.slice(3)) - Math.min(...tops.slice(3))).toBeLessThan(1)
        }
        await section.screenshot({ path: `target/website-review/capabilities-${locale}-${width}.png` })
      }
    }
    expect(media).toEqual([])
  } finally { await page.close() }
}
