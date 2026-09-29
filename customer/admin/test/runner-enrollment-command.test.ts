import { describe, expect, it } from 'vitest'
import { buildRunnerEnrollmentCommand } from '../src/runner-enrollment-command'

const input = {
  installerBaseURL: 'http://10.213.40.40:21082',
  controlURL: 'http://10.213.40.40:21080',
  token: 'aren_test-token',
  allowInsecureHTTP: true,
} as const

describe('Runner enrollment commands', () => {
  it('builds a complete Linux command without editable placeholders', () => {
    const platform = 'linux'
    const command = buildRunnerEnrollmentCommand({ ...input, platform })
    expect(command).toContain("curl -fsSL 'http://10.213.40.40:21082/install-runner.sh' | sudo bash -s --")
    expect(command).toContain("--control-url 'http://10.213.40.40:21080' --token 'aren_test-token'")
    expect(command).toContain('--allow-insecure-http')
    expect(command).not.toContain('--version')
    expect(command).not.toContain('\n')
    expect(command).not.toMatch(/<安全目录>|<安装根>/)
  })

  it('builds a complete Windows command without editable placeholders', () => {
    const command = buildRunnerEnrollmentCommand({ ...input, platform: 'windows' })
    expect(command).toContain("& ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing -Uri 'http://10.213.40.40:21082/install-runner.ps1'))) -InstallerUrl 'http://10.213.40.40:21082/install-runner.ps1' -ControlUrl 'http://10.213.40.40:21080' -Token 'aren_test-token'")
    expect(command).toContain('-AllowInsecureHttp')
    expect(command).not.toContain('-Version')
    expect(command).not.toContain('\n')
    expect(command).not.toMatch(/\$p|try \{|finally|Remove-Item/)
    expect(command).not.toMatch(/<安全目录>|<安装根>/)
  })

  it('quotes shell and PowerShell values without exposing syntax', () => {
    expect(buildRunnerEnrollmentCommand({ ...input, platform: 'linux', token: "a'b" }))
      .toContain("'a'\"'\"'b'")
    expect(buildRunnerEnrollmentCommand({ ...input, platform: 'windows', token: "a'b" }))
      .toContain("'a''b'")
  })
})
