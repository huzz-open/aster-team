import { createApp } from 'vue'
import App from './App.vue'
import router from './router'
import '@aster/ui/styles.css'

if (import.meta.env.MODE === 'demo') {
  const { startDemoMock } = await import('@aster/demo/browser')
  await startDemoMock()
}

createApp(App).use(router).mount('#app')
