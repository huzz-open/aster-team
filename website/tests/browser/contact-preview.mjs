import { expect } from '@playwright/test'

export async function checkContactLinks(browser, origin) {
  const context = await browser.newContext({ reducedMotion: 'reduce' })
  await context.route('https://api.github.com/**', route => route.fulfill({ status: 404, json: {} }))
  const page = await context.newPage()
  try {
    await page.goto(origin)
    for (const width of [1920, 768, 390, 320]) {
      await page.setViewportSize({ width, height: 945 })
      for (const locale of ['en', 'zh']) {
        await page.getByRole('button', { name: locale === 'en' ? 'Switch to English' : '切换为简体中文', exact: true }).click()
        const contact = page.locator('.contact-links')
        await contact.scrollIntoViewIfNeeded()
        await expect(contact.getByRole('heading')).toHaveText(locale === 'en' ? 'Get in touch' : '联系我们')
        expect(await contact.locator('a').evaluateAll(links => links.map(link => link.getAttribute('href')))).toEqual([
          'https://github.com/huzz-open/aster-team/issues', await contact.locator('a[href^="mailto:"]').getAttribute('href'),
        ])
        const issues = contact.getByRole('link', { name: 'GitHub Issues' })
        await expect(issues).toHaveAttribute('target', '_blank')
        await expect(issues).toHaveAttribute('rel', 'noopener noreferrer')
        expect(await contact.evaluate(element => element.scrollWidth <= element.clientWidth)).toBe(true)
        await contact.screenshot({ path: `target/website-review/contact-${locale}-${width}.png` })
      }
    }
  } finally { await context.close() }
}
