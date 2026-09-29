import { defineConfig } from '@playwright/test'
import real from './real.playwright.config.mjs'

export default defineConfig({
  ...real, testMatch: 'real-downloads.spec.mjs',
  outputDir: '../../dist/license-downloads/browser-results',
})
