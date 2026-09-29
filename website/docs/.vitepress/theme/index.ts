import DefaultTheme from 'vitepress/theme'
import ModelFieldMatrix from './ModelFieldMatrix.vue'
import ModelCallGuide from './ModelCallGuide.vue'
import ImageFieldMatrix from './ImageFieldMatrix.vue'
import InstanceModelList from './InstanceModelList.vue'
import InstanceApiBase from './InstanceApiBase.vue'
import InstanceCurlExample from './InstanceCurlExample.vue'
import Layout from './Layout.vue'
import type { Component } from 'vue'
import './style.css'

export default {
  extends: DefaultTheme,
  Layout,
  enhanceApp({ app }: { app: { component: (name: string, component: Component) => void } }) {
    app.component('ModelFieldMatrix', ModelFieldMatrix)
    app.component('ModelCallGuide', ModelCallGuide)
    app.component('ImageFieldMatrix', ImageFieldMatrix)
    app.component('InstanceModelList', InstanceModelList)
    app.component('InstanceApiBase', InstanceApiBase)
    app.component('InstanceCurlExample', InstanceCurlExample)
  },
}
