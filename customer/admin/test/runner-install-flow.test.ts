import { flushPromises, mount } from '@vue/test-utils'
import { beforeEach, describe, expect, it, vi } from 'vitest'
import { request } from '@aster/sdk'
import RunnerView from '../src/views/RunnerView.vue'

vi.mock('../src/license-status', () => ({ useLicensedFeature: () => true }))
vi.mock('@aster/sdk', async importOriginal => {
  const original = await importOriginal<typeof import('@aster/sdk')>()
  return { ...original, request: vi.fn(), copyText: vi.fn() }
})

describe('Runner installation flow', () => {
  beforeEach(() => {
    vi.mocked(request).mockReset()
    vi.mocked(request).mockImplementation(async (path, options) => {
      if (path === '/api/admin/runners') return { items: [] }
      if (path === '/api/admin/runners/connection') return { public_api_base_url: 'http://10.213.40.40:21080' }
      if (path === '/api/admin/maintenance') return { platform: 'linux' }
      if (path === '/api/admin/runners/enrollments' && options?.method === 'POST') {
        return {
          enrollment_id: 'enrollment-1', runner_name: 'office-runner', token: 'aren_test-token',
          expires_at: '2026-09-21T03:13:00Z', notice: '',
        }
      }
      throw new Error(`Unexpected request: ${path}`)
    })
  })

  it('shows platform tabs and one complete copyable command without implementation narration', async () => {
    const wrapper = mount(RunnerView, { global: { stubs: { Teleport: true } } })
    await flushPromises()

    await wrapper.findAll('button').find(button => button.text() === '新增 Runner')!.trigger('click')
    await wrapper.get('input[placeholder="例如：office-runner"]').setValue('office-runner')
    await wrapper.get('form').trigger('submit')
    await flushPromises()

    expect(wrapper.text()).toContain('选择 Runner 所在平台，复制并运行安装命令，然后确认安装目录。')
    expect(wrapper.text()).toContain('Linux')
    expect(wrapper.text()).toContain('Windows')
    expect(wrapper.text()).toContain('安装命令')
    expect(wrapper.findAll('.a-copy-code')).toHaveLength(1)
    expect(wrapper.get('.a-copy-code').classes()).toContain('a-copy-code--block')
    expect(wrapper.get('.a-copy-code code').text()).toContain(`/install-runner.sh' | sudo bash -s -- --control-url 'http://10.213.40.40:21080'`)
    expect(wrapper.get('.a-copy-code code').text()).not.toContain('--version')
    expect(wrapper.get('.a-copy-code code').text()).not.toContain('\n')
    await wrapper.findAll('button').find(button => button.text() === 'Windows')!.trigger('click')
    expect(wrapper.get('.a-copy-code code').text()).toContain('Invoke-RestMethod')
    expect(wrapper.get('.a-copy-code code').text()).not.toMatch(/\$p|try \{|finally|Remove-Item/)
    expect(wrapper.get('.a-copy-code code').text()).not.toContain('-Version')
    expect(wrapper.get('.a-copy-code code').text()).not.toContain('\n')
    expect(wrapper.text()).not.toContain('当前控制端使用内网 HTTP')
    expect(wrapper.text()).not.toContain('安全目录')
    expect(wrapper.text()).not.toContain('我已完成安装')
    expect(wrapper.text()).not.toContain('注册 Token')
  })
})
