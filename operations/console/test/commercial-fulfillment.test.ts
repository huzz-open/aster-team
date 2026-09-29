import fixtureData from '../../../contracts/test-vectors/plan-definition.v1.json'
import { flushPromises, mount } from '@vue/test-utils'
import { ACopyCode, AFilePicker, ASelect } from '@aster/ui'
import { nextTick } from 'vue'
import { beforeEach, expect, test, vi } from 'vitest'
import CommercialFulfillment from '../src/components/CommercialFulfillment.vue'
import ReauthActionModal from '../src/components/ReauthActionModal.vue'
import {
  OperationsAPIError,
  type CommercialPlanDefinition,
  type PaidFulfillmentContext,
  type PaidFulfillmentRecord,
  type PaidLifecycleSource,
  type PaidTransferRecord,
} from '../src/api/client'

const api = vi.hoisted(() => ({
  context: vi.fn(), lifecycle: vi.fn(), forOrder: vi.fn(), get: vi.fn(), approve: vi.fn(), issue: vi.fn(), profiles: vi.fn(), download: vi.fn(), redeliver: vi.fn(),
  latestTransfer: vi.fn(), getTransfer: vi.fn(), approveTransfer: vi.fn(), issueTransfer: vi.fn(), downloadTransfer: vi.fn(),
}))
vi.mock('../src/api/client', async importOriginal => ({
  ...await importOriginal<typeof import('../src/api/client')>(),
  getCurrentOperationsOperatorID: () => 'operator_1',
  getPaidFulfillmentContext: api.context,
  getPaidLifecycleSource: api.lifecycle,
  getPaidFulfillmentForOrder: api.forOrder,
  getPaidFulfillment: api.get,
  approvePaidFulfillment: api.approve,
  issuePaidFulfillment: api.issue,
  recordPaidRedelivery: api.redeliver,
  listV2IssuerProfiles: api.profiles,
  downloadPaidFulfillment: api.download,
  getLatestPaidTransfer: api.latestTransfer,
  getPaidTransfer: api.getTransfer,
  approvePaidTransfer: api.approveTransfer,
  issuePaidTransfer: api.issueTransfer,
  downloadPaidTransfer: api.downloadTransfer,
}))

const definition = fixtureData as CommercialPlanDefinition
const plan = { schema: 'aster.plan-snapshot.v1' as const, plan_id: 'paid_20', version: 4, definition }
const context: PaidFulfillmentContext = {
  order_id: 'order_1', customer_id: 'customer_1', order_sha256: 'a'.repeat(64), payment_id: 'payment_1', payment_sha256: 'b'.repeat(64),
  plan, amount_minor: 599900, currency: 'CNY', starts_at: '2026-09-06T00:00:00.000Z', ends_at: '2027-09-06T00:00:00.000Z',
  status: 'fulfillment_pending', source: 'manual', environment: 'local', minimum_version: definition.minimum_version,
  catalog_version: definition.entitlements.catalog_version, quota_policy_version: definition.quota_policy_version,
}
function record(status: 'approved' | 'prepared' | 'issued', keyID = 'paid-v2'): PaidFulfillmentRecord {
  const value = {
    snapshot: {
      schema: 'aster.paid-fulfillment.v1', id: 'fulfillment_1', environment: 'local',
      payment: {
        snapshot: {
          schema: 'aster.payment-snapshot.v1', id: 'payment_1', order: {
            schema: 'aster.order-snapshot.v1', order_id: 'order_1', customer_id: 'customer_1', plan, plan_sha256: 'c'.repeat(64),
            years: 1, discount_basis_points: 10000, amount_minor: 599900, currency: 'CNY', tax_mode: 'inclusive',
            starts_at: context.starts_at, ends_at: context.ends_at,
          }, order_sha256: context.order_sha256, amount_minor: 599900, currency: 'CNY', channel: 'bank_transfer',
          reference: 'reference_1', paid_at: '2026-09-06T00:00:00.000Z', recorded_at: '2026-09-06T00:00:00.000Z', recorded_by: 'operator_1',
        }, sha256: context.payment_sha256, operation_id: 'payment_op', status: 'confirmed',
      },
      request_sha256: 'd'.repeat(64), installation_request: {
        schema: 'aster.license-request.v2', request_id: 'request_1', installation_id: 'installation_1', generated_at: '2026-09-06T00:00:00.000Z',
        product: definition.product, product_version: definition.minimum_version, machine: { fingerprint: 'e'.repeat(64) },
      },
      request: { operation_id: 'fulfillment_op', expected_order_sha256: context.order_sha256, expected_payment_sha256: context.payment_sha256, license_request_json: '{}', reason: '到账与安装均已核对' },
      approved_at: '2026-09-06T01:00:00.000Z', approved_by: 'operator_1', reason: '到账与安装均已核对',
    },
    sha256: 'f'.repeat(64), operation_id: 'fulfillment_op', status,
    ...(status === 'approved' ? {} : { claims: { key_id: keyID, schema: 'aster.license.v2' } }),
    ...(status === 'issued' ? { document_sha256: '1'.repeat(64), issued_at: '2026-09-06T02:00:00.000Z' } : {}),
  }
  return value as unknown as PaidFulfillmentRecord
}
function transferRecord(
  status: "approved" | "prepared" | "issued",
  sequence = 1,
): PaidTransferRecord {
  const fulfillment = record("issued");
  const value = {
    snapshot: {
      schema: "aster.paid-transfer.v1",
      id: `transfer_${sequence}`,
      fulfillment_id: fulfillment.snapshot.id,
      fulfillment_sha256: fulfillment.sha256,
      customer_id: "customer_1",
      original_claims: fulfillment.claims,
      transfer_limit: definition.transfer_limit,
      request: {
        operation_id: `transfer_op_${sequence}`,
        expected_current_document_sha256: "1".repeat(64),
        license_request_json: "{}",
        reason: "换机",
      },
      installation_request: fulfillment.snapshot.installation_request,
      request_sha256: "2".repeat(64),
      previous_document_sha256: "1".repeat(64),
      previous_binding: {
        mode: "installation",
        installation_id: "previous_installation",
        machine_fingerprint_sha256: "3".repeat(43),
        transfer_sequence: sequence - 1,
      },
      previous_issued_at: "2026-09-06T02:00:00.000Z",
      transfer_sequence: sequence,
      environment: "local",
      approved_by: "operator_1",
      approved_at: "2026-09-06T03:00:00.000Z",
    },
    sha256: "4".repeat(64),
    status,
    ...(status === "approved"
      ? {}
      : { claims: { ...fulfillment.claims, key_id: "paid-v2" } }),
    ...(status === "issued"
      ? {
          document: { claims: fulfillment.claims, signature: "signature" },
          document_sha256: "5".repeat(64),
        }
      : {}),
  };
  return value as unknown as PaidTransferRecord;
}
const lifecycleSource = {
  schema: 'aster.paid-lifecycle-source.v1', kind: 'renewal', source_id: 'fulfillment_previous', source_record_kind: 'paid_transfer',
  source_record_id: 'transfer_current', source_record_sha256: '2'.repeat(64), customer_id: 'customer_1', license_id: 'license_previous',
  customer_ref: 'customer_ref_1',
  environment: 'local', document_sha256: '3'.repeat(64), binding: { mode: 'installation', installation_id: 'installation_1', machine_fingerprint_sha256: '4'.repeat(43), transfer_sequence: 1 },
  valid_from: '2025-09-06T00:00:00.000Z', valid_until: context.starts_at,
} as PaidLifecycleSource

const render = () => mount(CommercialFulfillment, { global: { stubs: { Teleport: true } } })
async function confirmPassword(wrapper: ReturnType<typeof render>, password: string) {
  const dialog = wrapper.findAllComponents(ReauthActionModal).find(component => component.props('open'))
  expect(dialog).toBeTruthy()
  dialog!.vm.$emit('submit', password)
  await flushPromises()
}
async function openAndLookup(wrapper: ReturnType<typeof render>) {
  await wrapper.get('button').trigger('click')
  await wrapper.get('input[aria-label="付费交付订单编号"]').setValue('order_1')
  await wrapper.findAll('button').find(button => button.text() === '读取交付状态')!.trigger('click')
  await flushPromises()
}

beforeEach(() => {
  sessionStorage.clear()
  for (const fn of Object.values(api)) fn.mockReset()
  api.forOrder.mockRejectedValue(new OperationsAPIError('PAID_FULFILLMENT_NOT_FOUND', '尚未批准', 404))
  api.context.mockResolvedValue(structuredClone(context))
})

test('approval reads frozen rights without requiring issuer-profile permission', async () => {
  const wrapper = render()
  await openAndLookup(wrapper)
  expect(wrapper.text()).toContain(definition.name)
  expect(wrapper.text()).toContain('5,999.00')
  expect(wrapper.text()).toContain('等待批准')
  expect(api.profiles).not.toHaveBeenCalled()
  wrapper.unmount()
})

test('embedded fulfillment restores the matching workflow node from the saved order', async () => {
  for (const [status, node] of [['approved', 6], ['prepared', 7], ['issued', 8]] as const) {
    api.forOrder.mockResolvedValueOnce(record(status))
    const wrapper = mount(CommercialFulfillment, { props: { embedded: true, launcher: false }, global: { stubs: { Teleport: true } } })
    await (wrapper.vm as unknown as { startForOrder: (orderID: string) => Promise<void> }).startForOrder('order_1')
    await flushPromises()
    expect(wrapper.emitted('stage-change')?.at(-1)).toEqual([node])
    wrapper.unmount()
  }
})

test('embedded approval stops at certificate selection and signing needs a separate node', async () => {
  api.forOrder.mockResolvedValue(record('approved'))
  api.profiles.mockResolvedValue([{ key_id: 'paid-v2', policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'] } }])
  const wrapper = mount(CommercialFulfillment, { props: { embedded: true, launcher: false, viewStep: 6 }, global: { stubs: { Teleport: true } } })
  await (wrapper.vm as unknown as { startForOrder: (orderID: string) => Promise<void> }).startForOrder('order_1')
  await flushPromises()
  expect(wrapper.text()).toContain('下一步：签发授权')
  expect(wrapper.text()).not.toContain('签发付费授权')
  await wrapper.findAll('button').find(button => button.text() === '下一步：签发授权')!.trigger('click')
  expect(wrapper.emitted('ready-to-issue')?.length).toBe(1)
  expect(api.issue).not.toHaveBeenCalled()
  await wrapper.setProps({ viewStep: 7 })
  expect(wrapper.text()).toContain('签发付费授权')
  await wrapper.setProps({ viewStep: 5 })
  expect(wrapper.text()).toContain('到账与安装均已核对')
  expect(wrapper.text()).not.toContain('签发付费授权')
  wrapper.unmount()
})

test('approval advances only to saved approval review, never directly to delivery', async () => {
  api.approve.mockResolvedValue(record('approved'))
  const wrapper = mount(CommercialFulfillment, { props: { embedded: true, launcher: false, viewStep: 4 }, global: { stubs: { Teleport: true } } })
  await (wrapper.vm as unknown as { startForOrder: (orderID: string) => Promise<void> }).startForOrder('order_1')
  const picker = wrapper.getComponent(AFilePicker)
  const file = new File(['{"schema":"aster.license-request.v2"}'], 'request.json')
  picker.vm.$emit('update:modelValue', file); await nextTick(); picker.vm.$emit('select', file)
  await flushPromises()
  expect(wrapper.emitted('stage-change')?.at(-1)).toEqual([5])
  await wrapper.setProps({ viewStep: 5 })
  await wrapper.get('textarea[aria-label="付费交付批准说明"]').setValue('到账与安装均已核对')
  await wrapper.get('input[type="checkbox"]').setValue(true)
  await wrapper.get('form').trigger('submit')
  await confirmPassword(wrapper, 'current-password')
  expect(wrapper.emitted('stage-change')?.at(-1)).toEqual([6])
  expect(wrapper.emitted('completed')).toBeUndefined()
  wrapper.unmount()
})

test('renewal approval freezes the current source preview in the recoverable request', async () => {
  api.lifecycle.mockResolvedValue(structuredClone(lifecycleSource))
  api.approve.mockResolvedValue(record('approved'))
  const wrapper = render()
  await openAndLookup(wrapper)
  const select = wrapper.findAllComponents(ASelect)[0]!
  select.vm.$emit('update:modelValue', 'renewal')
  await nextTick()
  await wrapper.get('input[aria-label="付费履约来源编号"]').setValue('fulfillment_previous')
  await wrapper.findAll('button').find(button => button.text() === '核对当前来源')!.trigger('click')
  await flushPromises()
  expect(wrapper.text()).toContain('license_previous')
  expect(wrapper.text()).toContain(lifecycleSource.document_sha256)
  const raw = '{"schema":"aster.license-request.v2"}'
  const picker = wrapper.getComponent(AFilePicker)
  const file = new File([raw], 'request.json')
  picker.vm.$emit('update:modelValue', file); await nextTick(); picker.vm.$emit('select', file)
  await flushPromises()
  await wrapper.get('textarea[aria-label="付费交付批准说明"]').setValue('续费合同与当前安装已核对')
  await wrapper.get('input[type="checkbox"]').setValue(true)
  await wrapper.get('form').trigger('submit')
  await confirmPassword(wrapper, 'current-password')
  expect(api.approve).toHaveBeenCalledOnce()
  expect(api.approve.mock.calls[0]![1].lifecycle).toEqual({ kind: 'renewal', source_id: 'fulfillment_previous', expected_document_sha256: lifecycleSource.document_sha256 })
  wrapper.unmount()
})

test('a renewal handoff opens the unified fulfillment with the order and source preselected', async () => {
  const renewalSource = { ...structuredClone(lifecycleSource), kind: 'renewal', source_id: 'fulfillment_previous', source_record_kind: 'paid_fulfillment' } as PaidLifecycleSource
  api.lifecycle.mockResolvedValue(renewalSource)
  const wrapper = render()
  await (wrapper.vm as unknown as { startSeeded: (orderID: string, kind: 'renewal', sourceID: string) => Promise<void> }).startSeeded('order_1', 'renewal', 'fulfillment_previous')
  await flushPromises()
  expect(api.forOrder).toHaveBeenCalledWith('order_1')
  expect(api.context).toHaveBeenCalledWith('order_1')
  expect(api.lifecycle).toHaveBeenCalledWith('renewal', 'fulfillment_previous')
  expect(wrapper.text()).toContain('license_previous')
  expect((wrapper.get('input[aria-label="付费履约来源编号"]').element as HTMLInputElement).value).toBe('fulfillment_previous')
  wrapper.unmount()
})

test('a structurally damaged local recovery record is retained and never rendered as trusted context', async () => {
  const damaged = [
    {},
    (() => { const value = structuredClone(context) as any; delete value.plan.definition.entitlements.features; return value })(),
    (() => { const value = structuredClone(context) as any; delete value.plan.definition.offer; return value })(),
    (() => { const value = structuredClone(context) as any; value.plan.definition.entitlements.quotas[0].limit = null; return value })(),
    (() => { const value = structuredClone(context) as any; value.plan.definition.offer.terms[0].years = 1.5; return value })(),
    (() => { const value = structuredClone(context) as any; value.plan.definition.offer.terms[0].years = 0; return value })(),
    (() => { const value = structuredClone(context) as any; value.plan.definition.offer.annual_amount_minor = 9000000000000; value.plan.definition.offer.terms[0] = { years: 5, discount_basis_points: 10000 }; return value })(),
  ]
  for (const [index, brokenContext] of damaged.entries()) {
    sessionStorage.clear()
    sessionStorage.setItem('aster.operations.pending.v1:fulfillment:operator_1', JSON.stringify({
      schema: 'aster.operations.pending.v1', owner: 'operator_1', uncertain: true,
      input: {
        operation_id: `fulfillment_damaged_${index}`, order_id: 'order_1', expected_order_sha256: 'a'.repeat(64),
        expected_payment_sha256: 'b'.repeat(64), license_request_json: '{}', reason: 'damaged', context: brokenContext,
      },
    }))
    const wrapper = render()
    await wrapper.get('button').trigger('click')
    expect(wrapper.get('[role="alert"]').text()).toContain('记录损坏')
    expect(wrapper.text()).not.toContain('等待批准')
    expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment:operator_1')).toBeTruthy()
    expect(api.context).not.toHaveBeenCalled()
    wrapper.unmount()
  }
})

test('only the current file read may populate approval input after replacement, clear or close', async () => {
  const deferred = () => {
    let resolve!: (value: ArrayBuffer) => void
    const promise = new Promise<ArrayBuffer>(done => { resolve = done })
    return { promise, resolve }
  }
  const values = (wrapper: ReturnType<typeof render>) => wrapper.findAllComponents(ACopyCode).map(item => item.props('value'))
  const wrapper = render()
  await openAndLookup(wrapper)
  const picker = wrapper.getComponent(AFilePicker)
  const rawA = '{"request_id":"installation-A"}'
  const rawB = '{"request_id":"installation-B"}'
  const readA = deferred(); const readB = deferred()
  const fileA = new File([rawA], 'a.json'); const fileB = new File([rawB], 'b.json')
  vi.spyOn(fileA, 'arrayBuffer').mockReturnValue(readA.promise)
  vi.spyOn(fileB, 'arrayBuffer').mockReturnValue(readB.promise)
  picker.vm.$emit('update:modelValue', fileA); await nextTick(); picker.vm.$emit('select', fileA)
  picker.vm.$emit('update:modelValue', fileB); await nextTick(); picker.vm.$emit('select', fileB)
  readB.resolve(new TextEncoder().encode(rawB).buffer as ArrayBuffer); await flushPromises()
  readA.resolve(new TextEncoder().encode(rawA).buffer as ArrayBuffer); await flushPromises()
  expect(values(wrapper)).toContain(rawB)
  expect(values(wrapper)).not.toContain(rawA)

  const rawC = '{"request_id":"installation-C"}'
  const readC = deferred(); const fileC = new File([rawC], 'c.json')
  vi.spyOn(fileC, 'arrayBuffer').mockReturnValue(readC.promise)
  picker.vm.$emit('update:modelValue', fileC); await nextTick(); picker.vm.$emit('select', fileC)
  picker.vm.$emit('update:modelValue', null); await nextTick(); picker.vm.$emit('clear')
  readC.resolve(new TextEncoder().encode(rawC).buffer as ArrayBuffer); await flushPromises()
  expect(values(wrapper)).not.toContain(rawC)

  const rawD = '{"request_id":"installation-D"}'
  const readD = deferred(); const fileD = new File([rawD], 'd.json')
  vi.spyOn(fileD, 'arrayBuffer').mockReturnValue(readD.promise)
  picker.vm.$emit('update:modelValue', fileD); await nextTick(); picker.vm.$emit('select', fileD)
  await wrapper.findAll('button').find(button => button.text() === '关闭')!.trigger('click')
  readD.resolve(new TextEncoder().encode(rawD).buffer as ArrayBuffer); await flushPromises()
  await wrapper.get('button').trigger('click')
  expect(values(wrapper)).not.toContain(rawD)
  wrapper.unmount()
})

test('an uncertain approval preserves exact request and operation but never the password', async () => {
  api.approve.mockRejectedValueOnce(new TypeError('响应丢失')).mockResolvedValueOnce(record('approved'))
  let wrapper = render()
  await openAndLookup(wrapper)
  const raw = '{\r\n  "schema": "aster.license-request.v2",\r\n  "request_id": "same"\r\n}\r\n'
  const picker = wrapper.getComponent(AFilePicker)
  const requestFile = new File([raw], 'request.json')
  picker.vm.$emit('update:modelValue', requestFile); await nextTick(); picker.vm.$emit('select', requestFile)
  await flushPromises()
  await wrapper.get('textarea[aria-label="付费交付批准说明"]').setValue('到账与安装均已核对')
  await wrapper.get('input[type="checkbox"]').setValue(true)
  await wrapper.get('form').trigger('submit')
  await confirmPassword(wrapper, 'never-store-me')
  const first = structuredClone(api.approve.mock.calls[0])
  const stored = sessionStorage.getItem('aster.operations.pending.v1:fulfillment:operator_1')!
  expect(JSON.parse(stored).input.license_request_json).toBe(raw)
  expect(stored).not.toContain('never-store-me')
  expect(wrapper.text()).toContain('结果尚未确认')
  wrapper.unmount()

  wrapper = render()
  expect(wrapper.get('button').text()).toBe('继续批准交付')
  await wrapper.get('button').trigger('click')
  expect(wrapper.find('input[aria-label="付费批准当前密码"]').exists()).toBe(false)
  await wrapper.get('form').trigger('submit')
  await confirmPassword(wrapper, 'new-current-password')
  expect(api.approve.mock.calls[1][0]).toBe(first[0])
  expect(api.approve.mock.calls[1][1]).toEqual(first[1])
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment:operator_1')).toBeNull()
  expect(wrapper.text()).toContain('已批准')
  wrapper.unmount()
})

test('issued fulfillment can be read and downloaded without issuer-profile access', async () => {
  api.forOrder.mockResolvedValue(record('issued'))
  api.download.mockResolvedValue(undefined)
  const wrapper = render()
  await openAndLookup(wrapper)
  expect(wrapper.text()).toContain('已签发')
  await wrapper.findAll('button').find(button => button.text() === '下载授权文件')!.trigger('click')
  await flushPromises()
  expect(api.download).toHaveBeenCalledOnce()
  expect(api.profiles).not.toHaveBeenCalled()
  wrapper.unmount()
})

test('an uncertain redelivery preserves the exact document operation but never the password', async () => {
  const issued = record('issued')
  api.forOrder.mockResolvedValue(issued)
  api.redeliver.mockRejectedValueOnce(new TypeError('响应丢失')).mockResolvedValueOnce({
    snapshot: { fulfillment_id: issued.snapshot.id, document_sha256: issued.document_sha256 }, sha256: '2'.repeat(64),
  })
  api.download.mockResolvedValue(undefined)
  let wrapper = render()
  await openAndLookup(wrapper)
  await wrapper.findAll('button').find(button => button.text() === '补发授权文件')!.trigger('click')
  await wrapper.get('textarea[aria-label="付费授权补发原因"]').setValue('客户重新确认收件地址')
  await wrapper.get('input[aria-label="确认补发原授权文件"]').setValue(true)
  await wrapper.get('form.fulfillment-redelivery').trigger('submit')
  await confirmPassword(wrapper, 'never-store-me')
  const first = structuredClone(api.redeliver.mock.calls[0])
  const stored = sessionStorage.getItem('aster.operations.pending.v1:fulfillment-redelivery:operator_1')!
  expect(stored).toContain(issued.document_sha256)
  expect(stored).toContain('客户重新确认收件地址')
  expect(stored).not.toContain('never-store-me')
  expect(wrapper.text()).toContain('结果尚未确认')
  wrapper.unmount()

  api.get.mockResolvedValue(issued)
  wrapper = render()
  expect(wrapper.get('button').text()).toBe('继续补发授权')
  await wrapper.get('button').trigger('click')
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith('fulfillment_1')
  expect(wrapper.find('input[aria-label="付费补发当前密码"]').exists()).toBe(false)
  await wrapper.get('form.fulfillment-redelivery').trigger('submit')
  await confirmPassword(wrapper, 'new-current-password')
  expect(api.redeliver.mock.calls[1][0]).toBe(first[0])
  expect(api.redeliver.mock.calls[1][1]).toEqual(first[1])
  expect(api.download).toHaveBeenCalledWith(issued)
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment-redelivery:operator_1')).toBeNull()
  wrapper.unmount()
})

test('a completed redelivery remains complete when only the exact-file download fails', async () => {
  const issued = record('issued')
  api.forOrder.mockResolvedValue(issued)
  api.redeliver.mockResolvedValue({ snapshot: { fulfillment_id: issued.snapshot.id, document_sha256: issued.document_sha256 }, sha256: '2'.repeat(64) })
  api.download.mockRejectedValue(new TypeError('下载连接中断'))
  const wrapper = render()
  await openAndLookup(wrapper)
  await wrapper.findAll('button').find(button => button.text() === '补发授权文件')!.trigger('click')
  await wrapper.get('textarea[aria-label="付费授权补发原因"]').setValue('客户需要再次下载')
  await wrapper.get('input[aria-label="确认补发原授权文件"]').setValue(true)
  await wrapper.get('form.fulfillment-redelivery').trigger('submit')
  await confirmPassword(wrapper, 'current-password')
  expect(wrapper.text()).toContain('补发记录已经完成')
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment-redelivery:operator_1')).toBeNull()
  wrapper.unmount()
})

test('an uncertain issue recovers the issued record with the fixed key after remount', async () => {
  api.forOrder.mockResolvedValue(record('approved'))
  api.profiles.mockResolvedValue([{ key_id: 'paid-v2', policy: { sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'] } }])
  api.issue.mockRejectedValueOnce(new TypeError('响应丢失'))
  let wrapper = render()
  await openAndLookup(wrapper)
  await wrapper.findAll('button').find(button => button.text() === '重新读取签发配置')!.trigger('click')
  await flushPromises()
  const select = wrapper.getComponent(ASelect)
  select.vm.$emit('update:modelValue', 'paid-v2')
  await wrapper.get('form.fulfillment-issue').trigger('submit')
  await confirmPassword(wrapper, 'never-store-me')
  const stored = sessionStorage.getItem('aster.operations.pending.v1:fulfillment-issue:operator_1')!
  expect(stored).toContain('paid-v2')
  expect(stored).not.toContain('never-store-me')
  wrapper.unmount()

  api.get.mockResolvedValue(record('issued'))
  wrapper = render()
  expect(wrapper.get('button').text()).toBe('继续签发交付')
  await wrapper.get('button').trigger('click')
  await flushPromises()
  expect(api.get).toHaveBeenCalledWith('fulfillment_1')
  expect(wrapper.text()).toContain('已签发')
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment-issue:operator_1')).toBeNull()
  expect(api.profiles).toHaveBeenCalledOnce()
  wrapper.unmount()
})

test.each(['prepared', 'issued'] as const)('a trusted %s record with another frozen key can explicitly end the stale local request', async status => {
  sessionStorage.setItem('aster.operations.pending.v1:fulfillment-issue:operator_1', JSON.stringify({
    schema: 'aster.operations.pending.v1', owner: 'operator_1', uncertain: true,
    input: { operation_id: 'issue_fulfillment_1', fulfillment_id: 'fulfillment_1', key_id: 'key-A' },
  }))
  api.get.mockResolvedValue(record(status, 'key-B'))
  const wrapper = render()
  expect(wrapper.get('button').text()).toBe('继续签发交付')
  await wrapper.get('button').trigger('click')
  await flushPromises()
  expect(wrapper.get('.fulfillment-conflict').text()).toContain('key-A')
  expect(wrapper.get('.fulfillment-conflict').text()).toContain('key-B')
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment-issue:operator_1')).toBeTruthy()
  if (status === 'prepared') expect(wrapper.findAll('button').find(button => button.text() === '重试原签发')!.attributes('disabled')).toBeDefined()
  else expect(wrapper.findAll('button').find(button => button.text() === '下载授权文件')!.attributes('disabled')).toBeDefined()
  await wrapper.findAll('button').find(button => button.text() === '确认采用服务器固定密钥')!.trigger('click')
  expect(sessionStorage.getItem('aster.operations.pending.v1:fulfillment-issue:operator_1')).toBeNull()
  expect(wrapper.text()).toContain('key-B')
  if (status === 'prepared') expect(wrapper.findAll('button').find(button => button.text() === '重试原签发')!.attributes('disabled')).toBeUndefined()
  else expect(wrapper.findAll('button').find(button => button.text() === '下载授权文件')!.attributes('disabled')).toBeUndefined()
  expect(api.issue).not.toHaveBeenCalled()
  wrapper.unmount()
})

async function openTransfer(wrapper: ReturnType<typeof render>) {
  await openAndLookup(wrapper);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "办理换机")!
    .trigger("click");
  await flushPromises();
}

test("an issued transfer recovered from an uncertain issue does not block the next transfer after reopening", async () => {
  const issuedFulfillment = record("issued");
  const issuedTransfer = transferRecord("issued");
  api.forOrder.mockResolvedValue(issuedFulfillment);
  api.getTransfer.mockResolvedValue(issuedTransfer);
  api.latestTransfer.mockResolvedValue(issuedTransfer);
  sessionStorage.setItem(
    "aster.operations.pending.v1:paid-transfer-issue:operator_1",
    JSON.stringify({
      schema: "aster.operations.pending.v1",
      owner: "operator_1",
      uncertain: true,
      input: {
        operation_id: "issue_transfer_1",
        transfer_id: "transfer_1",
        key_id: "paid-v2",
      },
    }),
  );
  const wrapper = render();
  await openTransfer(wrapper);
  expect(wrapper.get(".transfer-flow").text()).toContain("已签发");
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer-issue:operator_1",
    ),
  ).toBeNull();
  await wrapper
    .get(".transfer-flow")
    .findAll("button")
    .find((button) => button.text() === "关闭")!
    .trigger("click");
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "办理换机")!
    .trigger("click");
  await flushPromises();
  expect(wrapper.find('textarea[aria-label="付费授权换机原因"]').exists()).toBe(
    true,
  );
  expect(wrapper.get(".transfer-flow").text()).toContain("下载当前换机授权");
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "下载当前换机授权")!
    .trigger("click");
  expect(api.downloadTransfer).toHaveBeenCalledWith(issuedTransfer);
  wrapper.unmount();
});

test("an uncertain approval remains recoverable after it consumes the visible transfer limit", async () => {
  const issuedFulfillment = record("issued");
  const latest = transferRecord("approved", definition.transfer_limit);
  api.forOrder.mockResolvedValue(issuedFulfillment);
  api.latestTransfer.mockResolvedValue(latest);
  sessionStorage.setItem(
    "aster.operations.pending.v1:paid-transfer:operator_1",
    JSON.stringify({
      schema: "aster.operations.pending.v1",
      owner: "operator_1",
      uncertain: true,
      input: {
        operation_id: latest.snapshot.request.operation_id,
        fulfillment_id: "fulfillment_1",
        expected_current_document_sha256: "1".repeat(64),
        license_request_json: '{"same":true}',
        reason: "原换机请求",
      },
    }),
  );
  const wrapper = render();
  await openTransfer(wrapper);
  expect(wrapper.find('input[aria-label="换机批准当前密码"]').exists()).toBe(false);
  expect(wrapper.get('.transfer-flow').text()).toContain('重试原批准');
  expect(wrapper.get(".transfer-flow").text()).not.toContain(
    "换机次数已经用完",
  );
  expect(wrapper.get(".transfer-flow").text()).toContain(
    "上次批准结果尚未确认",
  );
  expect(wrapper.get(".transfer-flow").text()).toContain(
    "采用服务器已确认记录",
  );
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "采用服务器已确认记录")!
    .trigger("click");
  expect(
    sessionStorage.getItem("aster.operations.pending.v1:paid-transfer:operator_1"),
  ).toBeNull();
  expect(wrapper.find('input[aria-label="换机签发当前密码"]').exists()).toBe(false);
  expect(wrapper.get('.transfer-flow').text()).toContain('签发换机授权');
  wrapper.unmount();
});

test("a different trusted latest transfer cannot replace an uncertain approval without explicit choice", async () => {
  api.forOrder.mockResolvedValue(record("issued"));
  api.latestTransfer.mockResolvedValue(transferRecord("prepared", 1));
  sessionStorage.setItem(
    "aster.operations.pending.v1:paid-transfer:operator_1",
    JSON.stringify({
      schema: "aster.operations.pending.v1",
      owner: "operator_1",
      uncertain: true,
      input: {
        operation_id: "local_unknown_transfer",
        fulfillment_id: "fulfillment_1",
        expected_current_document_sha256: "1".repeat(64),
        license_request_json: '{"same":true}',
        reason: "本地未知请求",
      },
    }),
  );
  const wrapper = render();
  await openTransfer(wrapper);
  expect(wrapper.get(".transfer-conflict").text()).toContain(
    "local_unknown_transfer",
  );
  expect(wrapper.get(".transfer-conflict").text()).toContain("transfer_op_1");
  expect(wrapper.find('input[aria-label="换机签发当前密码"]').exists()).toBe(
    false,
  );
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer:operator_1",
    ),
  ).toBeTruthy();
  await wrapper.get(".transfer-conflict").find("button").trigger("click");
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer:operator_1",
    ),
  ).toBeNull();
  expect(wrapper.find('input[aria-label="换机签发当前密码"]').exists()).toBe(false);
  expect(wrapper.get('.transfer-flow').text()).toContain('重试原签发');
  expect(api.issueTransfer).not.toHaveBeenCalled();
  wrapper.unmount();
});

test.each(["prepared", "issued"] as const)(
  "a trusted transfer %s with another key blocks stale issue until explicitly adopted",
  async (status) => {
    const server = transferRecord(status);
    if (server.claims) server.claims.key_id = "key-B";
    api.forOrder.mockResolvedValue(record("issued"));
    api.getTransfer.mockResolvedValue(server);
    sessionStorage.setItem(
      "aster.operations.pending.v1:paid-transfer-issue:operator_1",
      JSON.stringify({
        schema: "aster.operations.pending.v1",
        owner: "operator_1",
        uncertain: true,
        input: {
          operation_id: "issue_transfer_1",
          transfer_id: "transfer_1",
          key_id: "key-A",
        },
      }),
    );
    const wrapper = render();
    await openTransfer(wrapper);
    expect(wrapper.get(".transfer-conflict").text()).toContain("key-A");
    expect(wrapper.get(".transfer-conflict").text()).toContain("key-B");
    expect(
      sessionStorage.getItem(
        "aster.operations.pending.v1:paid-transfer-issue:operator_1",
      ),
    ).toBeTruthy();
    await wrapper.get(".transfer-conflict").find("button").trigger("click");
    expect(
      sessionStorage.getItem(
        "aster.operations.pending.v1:paid-transfer-issue:operator_1",
      ),
    ).toBeNull();
    expect(api.issueTransfer).not.toHaveBeenCalled();
    wrapper.unmount();
  },
);

test("an unresolved transfer approval can be explicitly ended when the server still has no transfer", async () => {
  api.forOrder.mockResolvedValue(record("issued"));
  api.latestTransfer.mockRejectedValue(
    new OperationsAPIError("NOT_FOUND", "不存在", 404),
  );
  sessionStorage.setItem(
    "aster.operations.pending.v1:paid-transfer:operator_1",
    JSON.stringify({
      schema: "aster.operations.pending.v1",
      owner: "operator_1",
      uncertain: true,
      input: {
        operation_id: "missing_transfer",
        fulfillment_id: "fulfillment_1",
        expected_current_document_sha256: "1".repeat(64),
        license_request_json: '{"same":true}',
        reason: "未知请求",
      },
    }),
  );
  const wrapper = render();
  await openTransfer(wrapper);
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer:operator_1",
    ),
  ).toBeTruthy();
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "结束旧请求并重新核对")!
    .trigger("click");
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer:operator_1",
    ),
  ).toBeNull();
  expect(wrapper.find('textarea[aria-label="付费授权换机原因"]').exists()).toBe(
    true,
  );
  wrapper.unmount();
});

test("an uncertain issue key can be abandoned only before claims are frozen", async () => {
  api.forOrder.mockResolvedValue(record("issued"));
  api.getTransfer.mockResolvedValue(transferRecord("approved"));
  sessionStorage.setItem(
    "aster.operations.pending.v1:paid-transfer-issue:operator_1",
    JSON.stringify({
      schema: "aster.operations.pending.v1",
      owner: "operator_1",
      uncertain: true,
      input: {
        operation_id: "issue_transfer_1",
        transfer_id: "transfer_1",
        key_id: "retired-key",
      },
    }),
  );
  const wrapper = render();
  await openTransfer(wrapper);
  await wrapper
    .findAll("button")
    .find((button) => button.text() === "结束旧密钥请求并重新选择")!
    .trigger("click");
  expect(
    sessionStorage.getItem(
      "aster.operations.pending.v1:paid-transfer-issue:operator_1",
    ),
  ).toBeNull();
  expect(
    wrapper
      .findAll("button")
      .some((button) => button.text() === "读取签发配置"),
  ).toBe(true);
  expect(api.issueTransfer).not.toHaveBeenCalled();
  wrapper.unmount();
});
