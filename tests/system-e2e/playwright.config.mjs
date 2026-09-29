import { defineConfig } from '@playwright/test'
import { join, resolve } from 'node:path'

const runRoot = resolve(process.env.ASTER_SYSTEM_E2E_RUN_ROOT || '../../target/system-e2e/unscoped')

export default defineConfig({
  testDir: '.',
  testMatch: 'system.spec.mjs',
  fullyParallel: false,
  forbidOnly: true,
  timeout: 15 * 60_000,
  globalTimeout: 30 * 60_000,
  expect: { timeout: 20_000 },
  retries: 0,
  workers: 1,
  reporter: [['line'], ['html', { open: 'never', outputFolder: join(runRoot, 'browser-report') }]],
  outputDir: join(runRoot, 'browser-results'),
  use: {
    browserName: 'chromium',
    headless: process.env.ASTER_SYSTEM_E2E_HEADED !== 'true',
    ignoreHTTPSErrors: true,
    actionTimeout: 30_000,
    navigationTimeout: 30_000,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
    video: 'retain-on-failure',
  },
})
