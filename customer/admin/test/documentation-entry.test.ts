import { enableAutoUnmount, mount } from '@vue/test-utils'
import { createMemoryHistory, createRouter } from 'vue-router'
import { afterEach, beforeEach, expect, it } from 'vitest'
import { nextTick } from 'vue'
import AdminLayout from '../src/views/AdminLayout.vue'
import { adminProfile, adminProfileLoaded } from '../src/license-status'
import { locale } from '../src/i18n'

enableAutoUnmount(afterEach)
beforeEach(() => {
  locale.value = 'zh-CN'
  adminProfileLoaded.value = true
})
afterEach(() => {
  adminProfile.value = null
  adminProfileLoaded.value = false
  locale.value = 'zh-CN'
})

it.each(['missing', 'expired', 'active'])('keeps localized documentation available with a %s license', async state => {
  adminProfile.value = {
    email: 'admin@example.test', password_change_required: false, license_state: state,
    license: { state, available: state === 'active', features: [] },
  }
  const router = createRouter({
    history: createMemoryHistory(),
    routes: [{ path: '/:pathMatch(.*)*', component: { template: '<div />' }, meta: { adminPage: '/overview' } }],
  })
  await router.push('/overview')
  await router.isReady()
  const wrapper = mount(AdminLayout, { global: { plugins: [router] } })
  const entry = () => wrapper.get('.console-quick-links a[target="_blank"]')
  expect(entry().text()).toBe('文档')
  expect(entry().classes()).toContain('is-mobile-visible')
  expect(entry().attributes('aria-label')).toBe('文档')
  expect(entry().attributes('href')).toBe('/docs/zh-cn/administration/')
  expect(entry().attributes('rel')).toBe('noreferrer')
  expect(wrapper.find('.console-quick-links a[href="/models"]').exists()).toBe(false)
  locale.value = 'en-US'
  await nextTick()
  expect(entry().text()).toBe('Documentation')
  expect(entry().attributes('href')).toBe('/docs/en/administration/')
})
