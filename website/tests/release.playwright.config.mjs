import { defineConfig } from '@playwright/test'
import { dirname, join } from 'node:path'

const inputs = process.env.ASTER_WEBSITE_RELEASE_TEST_INPUTS
if (!inputs) throw new Error('Run npm run test:website:release:windows')
export default defineConfig({
  testDir: '.', testMatch: 'release.spec.mjs', workers: 1, fullyParallel: false, retries: 0, forbidOnly: true,
  timeout: 120_000, reporter: 'line', outputDir: join(dirname(inputs), 'browser-results'),
  use: { browserName: 'chromium', headless: true, baseURL: 'http://127.0.0.1:26991',
    viewport: { width: 1366, height: 648 }, trace: 'retain-on-failure', screenshot: 'only-on-failure' },
})
