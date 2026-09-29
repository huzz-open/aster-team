import { flushPromises, mount } from '@vue/test-utils'
import { request } from '@aster/sdk'
import { AButton, AFilePicker } from '@aster/ui'
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import MaintenanceView from '../src/views/MaintenanceView.vue'

vi.mock('@aster/sdk', async (importOriginal) => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn() }
})

const snapshot = (driver = 'sqlcipher') => ({
  jobs: [], versions: [], current_version: '2.0.1',
  upgrade_capabilities: { database_driver: driver, supported_modes: driver === 'sqlcipher' ? ['maintenance'] : [], unavailable_reason: '' },
})

describe('maintenance upgrade policy', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    vi.mocked(request).mockReset().mockResolvedValue(snapshot())
  })
  afterEach(() => vi.useRealTimers())

  it('warns about SQLite interruptions and submits an explicit maintenance mode', async () => {
    const view = mount(MaintenanceView)
    await flushPromises()
    expect(view.text()).toContain('不支持不停服升级')
    expect(view.text()).toContain('流式调用可能中断')
    view.getComponent(AFilePicker).vm.$emit('update:modelValue', new File(['package'], 'release.tar.gz'))
    await view.vm.$nextTick()
    await view.getComponent(AButton).trigger('click')
    await flushPromises()
    expect(request).toHaveBeenCalledWith('/api/admin/maintenance/upgrade?mode=maintenance', expect.objectContaining({ method: 'POST', body: expect.any(FormData) }))
    view.unmount()
  })

  it('checks the queue after losing the upload response instead of submitting twice', async () => {
    const view = mount(MaintenanceView)
    await flushPromises()
    vi.mocked(request).mockRejectedValueOnce(new Error('connection reset')).mockResolvedValue({ ...snapshot(), jobs: [{ id: 'accepted', operation: { type: 'upgrade' }, status: 'queued', upgrade_mode: 'maintenance', message: '已入队', updated_at: '2026-09-07T00:00:00Z' }] })
    view.getComponent(AFilePicker).vm.$emit('update:modelValue', new File(['package'], 'release.tar.gz'))
    await view.vm.$nextTick()
    await view.getComponent(AButton).trigger('click')
    await flushPromises()
    expect(view.getComponent(AButton).props('disabled')).toBe(true)
    expect(view.text()).toContain('已入队')
    expect(vi.mocked(request).mock.calls.filter(([, options]) => options?.method === 'POST')).toHaveLength(1)
    view.unmount()
  })

  it('keeps submissions disabled while a terminal job still needs executor finalization', async () => {
    vi.mocked(request).mockResolvedValue({ ...snapshot(), busy: true, jobs: [{ id: 'pending-cli', operation: { type: 'upgrade' }, status: 'failed', message: '稳定 CLI 同步失败', updated_at: '2026-09-07T00:00:00Z' }] })
    const view = mount(MaintenanceView)
    await flushPromises()
    expect(view.getComponent(AFilePicker).props('disabled')).toBe(true)
    expect(view.getComponent(AButton).props('disabled')).toBe(true)
    expect(view.text()).toContain('维护任务尚未结束')
    view.unmount()
  })

  it('uses backend maintenance capability for external databases without claiming continuity', async () => {
    vi.mocked(request).mockResolvedValue({ ...snapshot('mariadb'), upgrade_capabilities: { database_driver: 'mariadb', supported_modes: ['maintenance'], unavailable_reason: 'blue_green_runtime_not_available' } })
    const view = mount(MaintenanceView)
    await flushPromises()
    expect(view.getComponent(AFilePicker).props('disabled')).toBe(false)
    expect(view.text()).toContain('当前外部数据库支持维护升级')
    expect(view.text()).toContain('完整蓝绿切换与排空能力尚未开放')
    view.unmount()
  })

  it.each(['mariadb', 'unknown', 'legacy'])('disables unsupported or missing capabilities: %s', async (driver) => {
    vi.mocked(request).mockResolvedValue(driver === 'legacy' ? { jobs: [], versions: [], current_version: '2.0.0' } : snapshot(driver))
    const view = mount(MaintenanceView)
    await flushPromises()
    expect(view.getComponent(AFilePicker).props('disabled')).toBe(true)
    expect(view.getComponent(AButton).props('disabled')).toBe(true)
    expect(view.text()).toContain('升级暂不可用')
    view.unmount()
  })

  it('retains the submitted job through an outage and resumes polling without re-uploading', async () => {
    const running = { ...snapshot(), jobs: [{ id: 'upgrade', operation: { type: 'upgrade' }, upgrade_mode: 'maintenance', status: 'stopping_services', message: '停止旧服务', updated_at: '2026-09-07T00:00:00Z' }] }
    vi.mocked(request).mockResolvedValueOnce(running).mockRejectedValueOnce(new Error('503')).mockResolvedValue(snapshot())
    const view = mount(MaintenanceView)
    await flushPromises()
    await vi.advanceTimersByTimeAsync(1500)
    await flushPromises()
    expect(view.text()).toContain('请勿重复上传')
    expect(view.text()).toContain('停止旧服务')
    await vi.advanceTimersByTimeAsync(1500)
    await flushPromises()
    expect(view.text()).not.toContain('请勿重复上传')
    expect(vi.mocked(request).mock.calls.every(([url]) => url === '/api/admin/maintenance')).toBe(true)
    view.unmount()
    const count = vi.mocked(request).mock.calls.length
    await vi.advanceTimersByTimeAsync(10000)
    expect(request).toHaveBeenCalledTimes(count)
  })
})
