import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import { localDocumentationRoutes } from '../scripts/local-documentation-vite'
import { publicCatalogPlugin } from './build/public-catalog.mjs'
import { productReleasePlugin } from './build/product-release.mjs'

function developmentPort(name: string, fallback: number): number {
  const value = Number(process.env[name] || fallback)
  return Number.isInteger(value) && value > 0 && value <= 65535 ? value : fallback
}

const frontendPort = developmentPort('ASTER_WEBSITE_FRONTEND_PORT', 14080)
const backendPort = developmentPort('ASTER_WEBSITE_BACKEND_PORT', 8788)

export default defineConfig({
  plugins: [localDocumentationRoutes(import.meta.dirname, 'public'), vue(), publicCatalogPlugin(), productReleasePlugin()],
  server: {
    host: process.env.ASTER_LOCAL_LAN_ENABLED === 'true' ? '0.0.0.0' : '127.0.0.1',
    port: frontendPort,
    strictPort: true,
    proxy: {
      '/api': {
        target: `http://127.0.0.1:${backendPort}`,
        changeOrigin: true,
      },
    },
  },
  build: { sourcemap: false },
})
