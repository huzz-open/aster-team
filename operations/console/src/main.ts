import { createApp } from 'vue'
import App from './App.vue'
import router from './router'
import '@aster/ui/styles.css'
import './operations.css'

createApp(App).use(router).mount('#app')
