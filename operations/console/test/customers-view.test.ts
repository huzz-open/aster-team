import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import CustomersView from '../src/views/CustomersView.vue'

const navigation = vi.hoisted(() => ({ route: { path: '/base/customers', query: {} as Record<string, string> }, push: vi.fn(), replace: vi.fn() }))
vi.mock('vue-router', async original => ({ ...await original<typeof import('vue-router')>(), useRoute: () => navigation.route, useRouter: () => navigation }))
beforeEach(() => { navigation.route.query = {}; navigation.push.mockReset(); navigation.replace.mockReset() })

const updateCustomer = vi.fn(async (_id, input) => ({ ...input, id: 'cust_1', created_at: new Date().toISOString(), updated_at: new Date().toISOString() }))
vi.mock('../src/api/client', () => ({
  listCustomers: vi.fn(async () => ({ items: [{ id: 'cust_1', name: 'Aster Customer', legal_name: '', status: 'lead', contact_name: '', contact_email: '', contact_phone: '', contact_wechat: '', notes: '', created_at: new Date().toISOString(), updated_at: new Date().toISOString() }], next: '' })),
  getCustomerProfile: vi.fn(async () => ({ customer: { id: 'cust_1', name: 'Aster Customer', legal_name: '', status: 'lead', contact_name: '', contact_email: '', contact_phone: '', contact_wechat: '', notes: '', created_at: new Date().toISOString(), updated_at: new Date().toISOString() }, contacts: [{ id: 'contact_1', customer_id: 'cust_1', name: 'Finance', email: 'finance@example.com', phone: '', wechat: '', role_title: 'Finance', is_primary: true, created_at: new Date().toISOString(), updated_at: new Date().toISOString() }], billing_profile: { id: 'billing_1', customer_id: 'cust_1', invoice_title: 'Aster Customer Ltd.', tax_identifier: 'TAX-1', billing_email: 'billing@example.com', address: 'Shanghai', created_at: new Date().toISOString(), updated_at: new Date().toISOString() } })),
  updateCustomer: (...args: unknown[]) => updateCustomer(...args),
  createCustomer: vi.fn(), createContact: vi.fn(), upsertBillingProfile: vi.fn(),
}))

test('customer profile exposes contacts, billing and audited customer update form', async () => {
  const wrapper = mount(CustomersView, { global: { stubs: { Teleport: true } } })
  await flushPromises()
  await wrapper.get('.customer-items > button').trigger('click')
  await flushPromises()
  expect(wrapper.text()).toContain('开票资料')
  await wrapper.findAll('.detail-tabs button').find(button => button.text().includes('联系人'))!.trigger('click')
  expect(navigation.push).toHaveBeenCalledWith(expect.objectContaining({ query: expect.objectContaining({ customer: 'cust_1', tab: 'contacts' }) }))
  expect(wrapper.text()).toContain('Finance')
  await wrapper.findAll('.detail-tabs button').find(button => button.text() === '企业信息')!.trigger('click')
  const companyForm = wrapper.get('form.compact-form')
  await companyForm.get('input').setValue('Aster Customer Updated')
  await companyForm.trigger('submit')
  await flushPromises()
  expect(updateCustomer).toHaveBeenCalledWith('cust_1', expect.objectContaining({ name: 'Aster Customer Updated' }))
})

test('a customer detail URL restores its selected tab after refresh', async () => {
  navigation.route.query = { customer: 'cust_1', tab: 'billing' }
  const wrapper = mount(CustomersView, { global: { stubs: { Teleport: true } } })
  await flushPromises()
  expect((wrapper.get('form.compact-form input').element as HTMLInputElement).value).toBe('Aster Customer Ltd.')
  expect(wrapper.find('.detail-tabs button.active').text()).toBe('开票资料')
  wrapper.unmount()
})
