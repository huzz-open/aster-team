import { defineConfig } from '@playwright/test'

export default defineConfig({
  testDir: '.', testMatch: 'website.spec.mjs', fullyParallel: false, workers: 1, retries: 0, forbidOnly: true,
  timeout: 45_000, reporter: 'line', outputDir: '../../dist/website-validation/browser-results',
  use: { baseURL: 'http://127.0.0.1:26990', browserName: 'chromium', headless: true, viewport: { width: 1366, height: 648 },
    trace: 'retain-on-failure', screenshot: 'only-on-failure' },
})
