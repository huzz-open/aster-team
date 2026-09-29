import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { localAdminCredentials } from '../../scripts/local-admin-credentials-vite'

const operationsTarget = process.env.ASTER_OPERATIONS_ADDR
  ? `http://${process.env.ASTER_OPERATIONS_ADDR}`
  : 'http://127.0.0.1:12090'

export default defineConfig({
  plugins: [vue(), localAdminCredentials('operations')],
  server: {
    host: process.env.ASTER_LOCAL_LAN_ENABLED === 'true' ? '0.0.0.0' : '127.0.0.1',
    proxy: {
      '/api': operationsTarget,
      '/health': operationsTarget,
    },
  },
})
