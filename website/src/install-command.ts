export type InstallChoices = { version: string; email: string; protocol: 'http' | 'https'; host: string }

const emailPattern = /^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$/
const hostPattern = /^[A-Za-z0-9][A-Za-z0-9.:-]*$/

export function buildInstallCommand(choices: InstallChoices, availableVersions: readonly string[], siteOrigin: string): string | null {
  let scriptUrl: URL
  try {
    scriptUrl = new URL('/install.sh', siteOrigin)
    if (scriptUrl.origin !== siteOrigin || !['http:', 'https:'].includes(scriptUrl.protocol)) return null
  } catch { return null }
  const baseCommand = `curl -fsSL ${scriptUrl.href} | bash`
  const email = choices.email.trim()
  const host = choices.host.trim()
  if ((choices.version !== 'latest' && !availableVersions.includes(choices.version))
    || (email && !emailPattern.test(email)) || (host && !hostPattern.test(host))) return null
  const args: string[] = []
  if (choices.version !== 'latest') args.push('--version', choices.version)
  if (email) args.push('--email', email)
  if (choices.protocol === 'https') args.push('--protocol', 'https')
  else if (choices.protocol !== 'http') return null
  if (host) args.push('--host', host)
  return args.length ? `${baseCommand} -s -- ${args.join(' ')}` : baseCommand
}
