export type RunnerPlatform = 'linux' | 'windows'

type RunnerEnrollmentCommandInput = {
  platform: RunnerPlatform
  installerBaseURL: string
  controlURL: string
  token: string
  allowInsecureHTTP: boolean
}

function shellLiteral(value: string) {
  return `'${value.replaceAll("'", `'"'"'`)}'`
}

function powershellLiteral(value: string) {
  return `'${value.replaceAll("'", "''")}'`
}

export function buildRunnerEnrollmentCommand(input: RunnerEnrollmentCommandInput) {
  const insecure = input.allowInsecureHTTP ? ' --allow-insecure-http' : ''
  if (input.platform === 'windows') {
    const installerURL = new URL('/install-runner.ps1', input.installerBaseURL).href
    const allowInsecureHTTP = input.allowInsecureHTTP ? ' -AllowInsecureHttp' : ''
    return `& ([scriptblock]::Create((Invoke-RestMethod -UseBasicParsing -Uri ${powershellLiteral(installerURL)}))) -InstallerUrl ${powershellLiteral(installerURL)} -ControlUrl ${powershellLiteral(input.controlURL)} -Token ${powershellLiteral(input.token)}${allowInsecureHTTP}`
  }

  const installerURL = new URL('/install-runner.sh', input.installerBaseURL).href
  return `curl -fsSL ${shellLiteral(installerURL)} | sudo bash -s -- --control-url ${shellLiteral(input.controlURL)} --token ${shellLiteral(input.token)}${insecure}`
}
