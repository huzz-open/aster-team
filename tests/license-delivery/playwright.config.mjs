import { defineConfig } from '@playwright/test'

const operationsPort = process.env.ASTER_LICENSE_OPERATIONS_PORT ?? '12080'

export default defineConfig({
  testDir: '.',
  testMatch: 'delivery.spec.mjs',
  fullyParallel: false,
  forbidOnly: true,
  retries: process.env.CI ? 1 : 0,
  workers: 1,
  reporter: process.env.CI ? [['line'], ['html', { open: 'never', outputFolder: '../../target/license-delivery/report' }]] : 'line',
  outputDir: '../../target/license-delivery/results',
  use: {
    browserName: 'chromium',
    headless: true,
    trace: 'retain-on-failure',
    screenshot: 'only-on-failure',
  },
  webServer: [
    {
      command: `npm run dev --workspace @aster/operations-console -- --host 127.0.0.1 --port ${operationsPort} --strictPort`,
      url: `http://127.0.0.1:${operationsPort}/login`,
      reuseExistingServer: false,
      timeout: 120_000,
    },
  ],
})
