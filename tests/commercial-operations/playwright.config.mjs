import { defineConfig } from '@playwright/test'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  testDir: '.', testMatch: 'commercial.spec.mjs', fullyParallel: false, workers: 1, retries: 0, forbidOnly: true,
  reporter: 'line', outputDir: '../../dist/commercial-validation/browser-results',
  use: { baseURL: 'http://127.0.0.1:26380', browserName: 'chromium', headless: true, viewport: { width: 1366, height: 768 }, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
  webServer: { command: 'node tests/commercial-operations/preview.mjs', cwd: fileURLToPath(new URL('../../', import.meta.url)), url: 'http://127.0.0.1:26380/login', reuseExistingServer: false, timeout: 30_000 },
})
