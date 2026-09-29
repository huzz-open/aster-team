// Focused, frontend-only acceptance checks against an already running preview.
// Does not start services, submit forms or invoke the repository verification suite.
import { chromium } from 'playwright'
import { expect } from '@playwright/test'
import { mkdirSync } from 'node:fs'
import { checkDemo } from '../demo-scenarios.mjs'
import { checkDownloads } from './download-preview.mjs'
import { checkCapabilities } from './capabilities-preview.mjs'
import { checkContactLinks } from './contact-preview.mjs'
import { checkCapacityRemoval } from './capacity-removal-preview.mjs'

const origin = process.argv[2]
if (!origin || !['127.0.0.1', 'localhost'].includes(new URL(origin).hostname)) throw new Error('Pass an existing local preview URL')
const browser = await chromium.launch()
mkdirSync('target/website-review', { recursive: true })
try {
  await checkCapabilities(browser, origin)
  await checkContactLinks(browser, origin)
  await checkCapacityRemoval(browser, origin)
  const page = await browser.newPage({ viewport: { width: 1536, height: 960 }, reducedMotion: 'reduce' })
  const errors = []
  page.on('pageerror', error => errors.push(error.message))
  await page.route('https://api.github.com/repos/huzz-open/aster-team/releases/latest', route => route.fulfill({ status: 404, json: { message: 'Not Found' } }))
  await page.goto(origin)
  await expect(page.locator('.calculator-section,.usage-assessment')).toHaveCount(0)
  await expect(page.locator('.site-header nav button')).toHaveCount(4)
  for (const [width,height] of [[1920,1080],[1920,945],[1920,1000],[1536,864],[1440,900]]) {
    await page.setViewportSize({width,height})
    for (const language of ['Switch to English','切换为简体中文']) {
      await page.getByRole('button',{name:language,exact:true}).click()
      await page.evaluate(()=>scrollTo({top:0,behavior:'instant'}))
      await expect(page.locator('.gateway-endpoints h2')).toHaveCount(0)
      await expect(page.locator('.hero-download-note,.hero-system>h2')).toHaveCount(0)
      expect(await page.locator('.hero-actions').evaluate(element=>element.getBoundingClientRect().top-document.querySelector('.hero-copy>p').getBoundingClientRect().bottom)).toBeGreaterThanOrEqual(28)
      expect(await page.locator('.hero-system').evaluate(element=>element.getBoundingClientRect().top-document.querySelector('.hero-actions').getBoundingClientRect().bottom)).toBeGreaterThanOrEqual(48)
      await expect(page.locator('.gateway-endpoints li')).toHaveCount(6)
      expect(await page.locator('.hero-free-start').evaluate(element=>element.getBoundingClientRect().bottom)).toBeLessThanOrEqual(height)
      expect(await page.locator('.hero-copy h1').evaluate(element=>getComputedStyle(element).fontSize)).toBe('64px')
      if (height === 945) {
        expect(await page.locator('.gateway-endpoints ul').first().evaluate(element=>parseFloat(getComputedStyle(element).rowGap))).toBeGreaterThan(20)
        expect(await page.locator('.hero-free-start').evaluate(element=>innerHeight-element.getBoundingClientRect().bottom)).toBeLessThan(40)
      }
      await page.screenshot({path:`target/website-review/hero-fit-${width}-${height}-${language==='Switch to English'?'en':'zh'}.png`})
    }
  }
  for (const reducedMotion of ['reduce', 'no-preference']) {
    await page.emulateMedia({ reducedMotion })
    for (const width of [1920,390]) {
      await page.setViewportSize({ width, height: 945 })
      for (const language of ['Switch to English','切换为简体中文']) {
        await page.getByRole('button',{name:language,exact:true}).click()
        for (const [index,selector] of [[0,'.demo-heading'],[1,'.capability-section .section-heading'],[2,'.deployment-copy'],[3,'.catalog-heading']]) {
          if (width === 390) await page.locator('.menu-toggle').click()
          await page.locator('.site-header nav button').nth(index).click()
          await expect.poll(async()=>page.locator(selector).evaluate(element=>Math.abs(element.getBoundingClientRect().top-document.querySelector('.site-header').getBoundingClientRect().bottom-24))).toBeLessThan(2)
          await expect(page.locator('.site-header nav button').nth(index)).toHaveClass(/active/)
        }
        await expect(page.locator('.catalog-note')).toHaveCount(0)
        await expect(page.locator('.catalog-section')).not.toContainText('本页面暂未提供已验证的发行包')
        if (width === 1920) await page.screenshot({path:`target/website-review/download-${reducedMotion}-${language==='Switch to English'?'en':'zh'}.png`})
      }
    }
  }
  await page.emulateMedia({ reducedMotion: 'reduce' })
  for (const width of [1536, 390]) {
    await page.setViewportSize({ width, height: 960 })
    await checkDemo(page)
    const demo = page.locator('[data-product-demo]')
    await expect(demo.locator('.demo-workflow,.demo-scope,.demo-footnote')).toHaveCount(0)
    const before = await demo.evaluate(element => element.getBoundingClientRect().height)
    await demo.getByRole('tab', { name: 'Member', exact: true }).click()
    await demo.locator('nav').getByRole('button', { name: 'Usage Analytics', exact: true }).click()
    expect(await demo.evaluate(element => element.getBoundingClientRect().height)).toBe(before)
    const content = demo.locator('.demo-content')
    await content.scrollIntoViewIfNeeded()
    expect(await content.evaluate(element => element.scrollHeight > element.clientHeight)).toBe(true)
    expect(await content.evaluate(element => getComputedStyle(element).scrollbarWidth)).toBe('none')
    const windowBefore = await page.evaluate(() => scrollY)
    await content.hover()
    await page.mouse.wheel(0, 220)
    await expect.poll(() => content.evaluate(element => element.scrollTop)).toBeGreaterThan(0)
    await expect(page.locator('.a-transient-scrollbar.is-vertical')).toHaveClass(/is-visible/)
    await expect(page.locator('.a-transient-scrollbar.is-vertical')).not.toHaveClass(/is-visible/, { timeout: 2500 })
    expect(await page.evaluate(() => scrollY)).toBe(windowBefore)
    await demo.screenshot({ path: `target/website-review/demo-${width}.png` })
    await demo.locator('nav').getByRole('button', { name: 'Dashboard', exact: true }).click()
    expect(await content.evaluate(element => element.scrollTop)).toBe(0)
  }
  for (const width of [1920, 1366, 768, 390]) {
    await page.setViewportSize({ width, height: width === 1920 ? 1080 : 960 })
    for (const locale of ['en', 'zh']) {
      await page.getByRole('button', { name: locale === 'en' ? 'Switch to English' : '切换为简体中文', exact: true }).click()
      await expect(page.locator('main.site-scroll')).not.toContainText(/Pro 20x|Estimate from actual usage first|先按实际使用估算容量|每 3 名/)
      const unsupported = await page.evaluate(() => [...document.querySelectorAll('body *')].filter(element => {
        if (element.closest('[data-product-demo],svg') || !element.getClientRects().length || ![...element.childNodes].some(node => node.nodeType === Node.TEXT_NODE && node.textContent.trim())) return false
        return ![14, 16, 24, 32, 48, 64].includes(parseFloat(getComputedStyle(element).fontSize))
      }).map(element => ({ tag: element.tagName, class: element.className, size: getComputedStyle(element).fontSize })))
      expect(unsupported).toEqual([])
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(true)
      for (const input of await page.locator('.product-inquiry input[name="contact"],.lead-field input').all()) {
        expect(await input.evaluate(element => parseFloat(getComputedStyle(element).fontSize))).toBe(16)
      }
      if (locale === 'en') expect(await page.locator('.product-inquiry input[name="contact"]').getAttribute('placeholder')).not.toMatch(/wechat|微信/i)
      await expect(page.locator('.site-footer')).toContainText(locale === 'zh' ? 'HUZZ · 弧之舟（HuZhiZhou）' : 'A product by HUZZ (HuZhiZhou)')
      await expect(page.locator('.site-footer a[href^="mailto:"]')).toHaveCount(0)
      await expect(page.locator('.inquiry-community a')).toHaveAttribute('href','https://github.com/huzz-open/aster-team/issues/new/choose')
      await expect(page.locator('.other-releases')).toHaveAttribute('href','https://github.com/huzz-open/aster-team/releases')
      await expect(page.locator('.latest-release-version')).not.toBeEmpty()
      const connections = page.locator('.client-connections')
      for (const name of ['Codex', 'Claude Code', 'Cursor']) {
        await connections.getByRole('tab', { name, exact: true }).click()
        await expect(connections.getByRole('tabpanel')).toContainText(name)
        const guide = name === 'Codex' ? 'codex.md' : name === 'Claude Code' ? 'claude-code.md' : 'member-guide.md'
        await expect(connections.locator('.aster-client-guide')).toHaveAttribute('href', `https://github.com/huzz-open/aster-team/blob/main/docs/${locale === 'zh' ? 'zh-CN/' : ''}${guide}`)
        await expect(connections.locator('.official-client-guide')).toHaveAttribute('href', /^https:\/\//)
        await expect(connections.locator('.codex-workflow')).toHaveCount(name === 'Codex' ? 1 : 0)
        if (name === 'Codex') {
          await expect(connections.locator('.codex-workflow')).toContainText(locale === 'zh' ? '无需反复输入 Key' : 'No repeated key entry')
          await expect(connections.locator('.client-caveat')).toContainText(locale === 'zh' ? '本地工作流' : 'local Codex workflows')
          await connections.screenshot({ path: `target/website-review/codex-guide-${width}-${locale}.png` })
        } else await expect(connections.getByRole('tabpanel')).not.toContainText('Codex')
      }
      await connections.getByRole('tab', { name: 'Cursor', exact: true }).press('Home')
      await expect(connections.getByRole('tab', { name: 'Codex', exact: true })).toBeFocused()
      await page.evaluate(() => scrollTo({ top: 0, behavior: 'instant' }))
      await page.screenshot({ path: `target/website-review/hero-${width}-${locale}.png` })
      await connections.screenshot({ path: `target/website-review/clients-${width}-${locale}.png` })
      await page.locator('.catalog-heading').scrollIntoViewIfNeeded()
      await page.screenshot({ path: `target/website-review/plans-${width}-${locale}.png` })
      if (width === 1920 || width === 390) {
        await page.locator('.private-architecture-scroll').screenshot({ path: `target/website-review/architecture-${width}-${locale}.png` })
      }
    }
  }
  // An unconfigured preview must explain availability without inventing a package or price.
  if (await page.locator('.release-pending').count()) {
    await checkDownloads(browser, origin)
    await expect(page.locator('a.latest-download-button')).toHaveAttribute('href', 'https://github.com/huzz-open/aster-team/releases/download/v2.1.1/aster-team-2.1.1-linux-amd64.tar.gz')
    await expect(page.locator('.release-pending a').nth(1)).toHaveAttribute('href', 'https://github.com/huzz-open/aster-team/releases')
  }
  if (await page.locator('.plan-empty').count()) {
    await page.locator('.plan-empty button').click()
    await expect(page.locator('.product-inquiry input[name="contact"]')).toBeFocused()
  }
  expect(errors).toEqual([])
  console.log('PASS: bilingual demo, bounded scrolling and transient indicators, typography, responsive layout, client guides and availability fallback')
} finally { await browser.close() }
