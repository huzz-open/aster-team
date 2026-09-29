import { defineConfig } from '@playwright/test'
import { fileURLToPath } from 'node:url'

export default defineConfig({
  testDir: '.', testMatch: 'member-license.spec.mjs', fullyParallel: false, workers: 1, retries: 0, forbidOnly: true,
  reporter: 'line', outputDir: '../../dist/license-currentness/browser-results',
  use: {
    baseURL: 'http://127.0.0.1:26481', browserName: 'chromium', headless: true,
    viewport: { width: 1366, height: 648 }, trace: 'retain-on-failure', screenshot: 'only-on-failure',
  },
  webServer: {
    command: 'npm run build --workspace @aster/member && npm exec --workspace @aster/member -- vite preview --host 127.0.0.1 --port 26481 --strictPort',
    cwd: fileURLToPath(new URL('../../', import.meta.url)),
    url: 'http://127.0.0.1:26481/login', reuseExistingServer: false, timeout: 120_000,
  },
})
