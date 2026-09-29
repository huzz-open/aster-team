import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { localDocumentationRoutes, localPublicAssets } from '../../scripts/local-documentation-vite'
import { localAdminCredentials } from '../../scripts/local-admin-credentials-vite'

const controlTarget = process.env.ASTER_CONTROL_PORT
  ? `http://127.0.0.1:${process.env.ASTER_CONTROL_PORT}`
  : 'http://127.0.0.1:11080'

export default defineConfig(({ command }) => ({
  plugins: [localPublicAssets(import.meta.dirname), localDocumentationRoutes(import.meta.dirname), vue(), localAdminCredentials('member')],
  publicDir: command === 'serve' ? '.docs-public' : 'public',
  server: {
    host: process.env.ASTER_LOCAL_LAN_ENABLED === 'true' ? '0.0.0.0' : '127.0.0.1',
    proxy: {
      '/api': controlTarget,
      '/v1': controlTarget,
    },
  },
}))
