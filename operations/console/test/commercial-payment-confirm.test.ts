import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, expect, test, vi } from 'vitest'
import CommercialPaymentConfirm from '../src/components/CommercialPaymentConfirm.vue'
import ReauthActionModal from '../src/components/ReauthActionModal.vue'

const api = vi.hoisted(() => ({ context: vi.fn() }))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  getCommercialPaymentContext: api.context,
}))

beforeEach(() => {
  sessionStorage.clear()
  api.context.mockReset().mockResolvedValue({
    order_id: 'order_1', customer_id: 'customer_1', order_sha256: 'a'.repeat(64),
    amount_minor: 23998, currency: 'CNY', starts_at: '2026-09-16T00:00:00Z',
    ends_at: '2027-09-16T00:00:00Z', source: 'manual', status: 'pending_payment',
  })
})

test('embedded payment loads the active order and keeps confirmation fields in one workspace', async () => {
  const wrapper = mount(CommercialPaymentConfirm, { props: { embedded: true, launcher: false } })
  await (wrapper.vm as unknown as { startForOrder: (id: string) => Promise<void> }).startForOrder('order_1')
  await flushPromises()
  expect(api.context).toHaveBeenCalledWith('order_1')
  expect(wrapper.find('input[aria-label="到账订单编号"]').exists()).toBe(false)
  expect(wrapper.text()).not.toContain('读取到账订单')
  expect(wrapper.get('input[aria-label="到账凭据"]')).toBeTruthy()
  expect(wrapper.get('input[aria-label="实际到账时间（UTC）"]')).toBeTruthy()
  expect(wrapper.find('input[aria-label="到账确认当前密码"]').exists()).toBe(false)
  await wrapper.get('input[aria-label="到账凭据"]').setValue('bank_1')
  await wrapper.get('input[type="checkbox"]').setValue(true)
  await wrapper.get('form').trigger('submit')
  expect(wrapper.getComponent(ReauthActionModal).props('open')).toBe(true)
  expect(wrapper.get('details.payment-order-details').element.hasAttribute('open')).toBe(false)
  wrapper.unmount()
})
