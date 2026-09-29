import { preview } from 'vite'
import { fileURLToPath } from 'node:url'
const root = fileURLToPath(new URL('../../operations/console/', import.meta.url))
await preview({ root, configFile: false, preview: { host: '127.0.0.1', port: 26380, strictPort: true, proxy: { '/api': 'http://127.0.0.1:26390' } } })
