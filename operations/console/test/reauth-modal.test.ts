import { mount } from '@vue/test-utils'
import { expect, test } from 'vitest'
import ReauthActionModal from '../src/components/ReauthActionModal.vue'

test('reauthentication modal masks and clears the password', async () => {
  const wrapper = mount(ReauthActionModal, { props: { open: true, title: 'Confirm', description: 'Sensitive action' }, global: { stubs: { Teleport: true } } })
  const input = wrapper.get('input')
  expect(input.attributes('type')).toBe('password')
  await input.setValue('current-password')
  await wrapper.get('form').trigger('submit')
  expect(wrapper.emitted('submit')?.[0]).toEqual(['current-password'])
  await wrapper.setProps({ open: false })
  await wrapper.setProps({ open: true })
  expect((wrapper.get('input').element as HTMLInputElement).value).toBe('')
})
