import { defineConfig } from '@playwright/test'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  testDir: '.',
  testMatch: 'layout.spec.mjs',
  workers: 1,
  reporter: 'line',
  outputDir: '../../dist/operations-ui-validation',
  use: { browserName: 'chromium', headless: true },
  webServer: {
    command: 'npm run dev --workspace operations/console -- --host 127.0.0.1 --port 26388 --strictPort',
    cwd: fileURLToPath(new URL('../../', import.meta.url)),
    url: 'http://127.0.0.1:26388/login',
    reuseExistingServer: false,
    timeout: 30_000,
  },
})
