import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '.', testMatch: ['real-entitlements.spec.mjs', 'real-support.spec.mjs', 'real-retention.spec.mjs', 'real-free-switch.spec.mjs'], workers: 1, fullyParallel: false,
  retries: 0, forbidOnly: true, timeout: 90_000, reporter: 'line',
  outputDir: '../../dist/license-real/browser-results',
  use: { browserName: 'chromium', headless: true, locale: 'zh-CN', viewport: { width: 1366, height: 648 }, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
})
