import type { LenisOptions } from 'lenis'
import { VueLenis } from 'lenis/vue'
import { createApp, h } from 'vue'
import App from './App.vue'
import 'lenis/dist/lenis.css'
import '@aster/ui/transient-scrollbars'
import '@aster/ui/transient-scrollbars.css'
import './style.css'

const lenisOptions: LenisOptions = {
  anchors: true,
  lerp: 0.1,
  smoothWheel: true,
  syncTouch: false,
  wheelMultiplier: 1,
  stopInertiaOnNavigate: true,
  respectReducedMotion: true,
}

createApp({
  render: () => h(VueLenis, {
    root: true,
    autoRaf: true,
    options: lenisOptions,
  }, {
    default: () => h(App),
  }),
}).mount('#app')
