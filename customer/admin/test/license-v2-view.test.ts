import { enableAutoUnmount, flushPromises, mount } from '@vue/test-utils'
import { copyText, request } from '@aster/sdk'
import { AButton, AFilePicker, AInfoTip, AModal } from '@aster/ui'
import { nextTick } from 'vue'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import LicenseView from '../src/views/LicenseView.vue'

enableAutoUnmount(afterEach)

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, copyText: vi.fn(), request: vi.fn() }
})

function status(expiresAt: string | null, licensed: number | null) {
  return {
    state: 'active',
    license: {
      schema: 'aster.admin-license-view.v1',
      protocol_schema: 'aster.license.v2',
      license_id: 'test_free_fixed',
      serial: 'TEST-V2',
      customer_ref: null,
      key_id: 'test-only-v2',
      edition: 'test-only',
      plan_id: 'test_plan',
      features: ['gateway', 'member', 'runner'],
      quotas: {
        member_seats: licensed,
        runners: licensed === null ? null : 1,
        upstream_accounts: licensed === null ? null : 1,
        api_keys_per_member: licensed === null ? null : 1,
      },
      binding: licensed === null ? 'installation' : 'unbound',
      minimum_version: '1.2.3-test.1',
      transfer_sequence: 0,
      issued_at: '2026-09-01T00:00:00.000Z',
      not_before: '2026-09-01T00:00:00.000Z',
      expires_at: expiresAt,
    },
    scheduled_license: null,
    scheduled_state: 'none',
    seat_usage: { occupied: 2, licensed },
    online_runners: 1,
  }
}

describe('v2 license management view', () => {
  beforeEach(() => {
    vi.mocked(copyText).mockReset()
    vi.mocked(request).mockReset()
  })

  it('renders the stable free v2 projection without reading raw claims', async () => {
    vi.mocked(request).mockResolvedValue(status('2027-09-01T00:00:00.000Z', 3))
    const wrapper = mount(LicenseView)
    await flushPromises()

    expect(wrapper.text()).toContain('成员席位2 / 3')
    expect(wrapper.text()).toContain('Runner1 / 1')
    expect(wrapper.text()).toContain('授权周期')
    expect(wrapper.text()).toContain('剩余')
    expect(wrapper.text()).toContain('到期时间')
    expect(wrapper.find('.license-state-icon svg').exists()).toBe(true)
    expect(wrapper.text()).not.toContain('授权有效')
    expect(wrapper.text()).not.toContain('正在使用')
    expect(wrapper.findAll('.feature-tags b').map(node => node.text())).toEqual(['gateway', 'member', 'runner'])
    expect(wrapper.text()).toContain('TEST-V2')
    expect(wrapper.text()).not.toContain('系统会先校验并对比权益')
    expect(wrapper.text()).not.toContain('仅支持 JSON，最大 64 KiB')
    expect(wrapper.text()).not.toContain('使用服务器命令导入')
    expect(wrapper.text()).not.toContain('切换免费版')
    await wrapper.get('button[aria-label="复制套餐标识"]').trigger('click')
    expect(vi.mocked(copyText)).toHaveBeenCalledWith('test_plan')
    const importHelp = wrapper.getComponent(AInfoTip)
    expect(importHelp.props('text')).toBe('查看文件要求和服务器导入命令')
    expect(importHelp.props('interactive')).toBe(true)
    expect(wrapper.getComponent(AFilePicker).props('hint')).toBe('')
    await wrapper.get('.a-info-tip').trigger('mouseenter')
    await flushPromises()
    expect(document.body.textContent).toContain('sudo aster-team-cli license install --source ./license.json')
  })

  it('renders no-expiry and unlimited quota modes without throwing', async () => {
    vi.mocked(request).mockResolvedValue(status(null, null))
    const wrapper = mount(LicenseView)
    await flushPromises()

    expect(wrapper.text()).toContain('成员席位2 / 不限')
    expect(wrapper.text()).toContain('Runner1 / 不限')
    expect(wrapper.text()).toContain('授权周期长期有效')
    expect(wrapper.text()).toContain('到期时间—')
  })

  it('keeps the current license visible while showing the scheduled replacement', async () => {
    const current = status('2027-09-01T00:00:00.000Z', 3)
    current.scheduled_state = 'waiting'
    current.scheduled_license = {
      ...current.license,
      license_id: 'test_paid_renewal',
      serial: 'TEST-PAID-RENEWAL',
      plan_id: 'team_20',
      quotas: { ...current.license.quotas, member_seats: 20 },
      not_before: '2026-10-01T00:00:00.000Z',
      expires_at: '2027-10-01T00:00:00.000Z',
    }
    vi.mocked(request).mockResolvedValue(current)
    const wrapper = mount(LicenseView)
    await flushPromises()

    expect(wrapper.text()).toContain('下一份授权')
    expect(wrapper.text()).toContain('已安排自动生效')
    expect(wrapper.text()).toContain('生效时间')
    expect(wrapper.text()).toContain('test_paid_renewal')
    expect(wrapper.text()).toContain('TEST-V2')
    expect(wrapper.text()).toContain('20')
  })

  it('shows a scheduled first license alongside the installation recovery workflow', async () => {
    const pending = status('2027-09-01T00:00:00.000Z', 3)
    pending.state = 'missing'
    pending.scheduled_state = 'waiting'
    pending.scheduled_license = { ...pending.license, license_id: 'first_scheduled' }
    pending.license = null as never
    vi.mocked(request).mockResolvedValue(pending)
    const wrapper = mount(LicenseView)
    await flushPromises()

    expect(wrapper.text()).toContain('下一份授权')
    expect(wrapper.text()).toContain('first_scheduled')
    expect(wrapper.text()).toContain('获取授权')
    expect(wrapper.text()).toContain('免费授权')
    expect(wrapper.text()).toContain('商业授权')
    expect(wrapper.find('.license-dashboard.is-unlicensed').exists()).toBe(true)
    expect(wrapper.find('.workflow-card').exists()).toBe(false)
    expect(wrapper.findAllComponents(AFilePicker)).toHaveLength(1)
  })

  it('shows an unreadable first stage without hiding recovery controls', async () => {
    const pending = status(null, 0)
    pending.state = 'missing'
    pending.scheduled_state = 'unavailable'
    pending.license = null as never
    vi.mocked(request).mockResolvedValue(pending)
    const wrapper = mount(LicenseView)
    await flushPromises()

    expect(wrapper.text()).toContain('预存许可证无法读取')
    expect(wrapper.text()).toContain('获取授权')
    expect(wrapper.text()).not.toContain('生成机器授权申请')
    expect(wrapper.find('.workflow-card').exists()).toBe(false)
  })

  it('previews entitlement changes before an immediate replacement', async () => {
    const current = status('2027-09-01T00:00:00.000Z', 3)
    const next = {
      ...current.license,
      license_id: 'paid_20',
      edition: 'commercial',
      plan_id: 'team_20',
      features: [...current.license.features, 'audit'],
      quotas: { ...current.license.quotas, member_seats: 20, runners: null },
    }
    vi.mocked(request)
      .mockResolvedValueOnce(current)
      .mockResolvedValueOnce({ activation: 'active', license: next })
    const wrapper = mount(LicenseView)
    await flushPromises()

    wrapper.getComponent(AFilePicker).vm.$emit('update:modelValue', new File(['{}'], 'license.json', { type: 'application/json' }))
    await nextTick()
    await wrapper.findAllComponents(AButton).find(button => button.text().includes('校验并对比'))!.trigger('click')
    await flushPromises()

    expect(vi.mocked(request)).toHaveBeenNthCalledWith(2, '/api/admin/license/preview', { method: 'POST', body: '{}' })
    expect(wrapper.getComponent(AModal).props('open')).toBe(true)
    expect(document.body.textContent).toContain('权益提升')
    expect(document.body.textContent).toContain('确认后立即替换当前授权')
    expect(document.body.textContent).toContain('立即切换')
    expect(document.body.textContent).toContain('audit')
  })

  it('explains that a future signed license can only be saved for renewal', async () => {
    const current = status('2027-09-01T00:00:00.000Z', 3)
    const next = { ...current.license, license_id: 'renewal_20', plan_id: 'team_20', not_before: '2026-10-01T00:00:00.000Z', quotas: { ...current.license.quotas, member_seats: 20 } }
    vi.mocked(request)
      .mockResolvedValueOnce(current)
      .mockResolvedValueOnce({ activation: 'scheduled', license: next })
    const wrapper = mount(LicenseView)
    await flushPromises()

    wrapper.getComponent(AFilePicker).vm.$emit('update:modelValue', new File(['{}'], 'license.json', { type: 'application/json' }))
    await nextTick()
    await wrapper.findAllComponents(AButton).find(button => button.text().includes('校验并对比'))!.trigger('click')
    await flushPromises()

    expect(document.body.textContent).toContain('许可证签名约定的生效时间尚未到达')
    expect(document.body.textContent).toContain('保存续期授权')
    expect(document.body.textContent).not.toContain('立即切换')
  })
})
