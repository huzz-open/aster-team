import { existsSync, readFileSync } from 'node:fs'
import { homedir } from 'node:os'
import { posix, resolve, win32 } from 'node:path'

const pathApi = platform => platform === 'win32' ? win32 : posix
const executableName = (command, platform = process.platform) => platform === 'win32' ? `${command}.exe` : command

export function configuredWindowsToolsRoot(repositoryRoot, platform = process.platform) {
  if (platform !== 'win32') return null
  try {
    const settings = JSON.parse(readFileSync(resolve(repositoryRoot, '.aster-tools', 'windows-setup.json'), 'utf8'))
    if (!/^[a-z]:[\\/]/i.test(settings.installRoot || '')) return null
    return win32.normalize(settings.installRoot)
  } catch { return null }
}

export function sharedToolsRoot(repositoryRoot, {
  platform = process.platform, environment = process.env, homeDirectory = homedir(),
} = {}) {
  const paths = pathApi(platform)
  if (environment.ASTER_TOOLS_ROOT) return paths.resolve(environment.ASTER_TOOLS_ROOT)
  const windowsRoot = configuredWindowsToolsRoot(repositoryRoot, platform)
  if (windowsRoot) return windowsRoot
  if (platform === 'darwin') return paths.join(homeDirectory, 'Library', 'Application Support', 'AsterDev')
  if (platform === 'linux') return paths.join(environment.XDG_DATA_HOME || paths.join(homeDirectory, '.local', 'share'), 'aster-dev')
  return null
}

export function preferredGoRoot(repositoryRoot, options = {}) {
  const platform = options.platform || process.platform
  const environment = options.environment || process.env
  const paths = pathApi(platform)
  if (environment.ASTER_GO_ROOT) return paths.resolve(environment.ASTER_GO_ROOT)
  const toolsRoot = sharedToolsRoot(repositoryRoot, { ...options, platform, environment })
  return toolsRoot ? paths.join(toolsRoot, 'Go') : paths.resolve(repositoryRoot, '.aster-tools', 'go')
}

export function localGoRoots(repositoryRoot, options = {}) {
  const platform = options.platform || process.platform
  const environment = options.environment || process.env
  const paths = pathApi(platform)
  const candidates = [preferredGoRoot(repositoryRoot, { ...options, platform, environment })]
  if (environment.INIT_CWD) candidates.push(paths.resolve(environment.INIT_CWD, '.aster-tools', 'go'))
  candidates.push(paths.resolve(repositoryRoot, '.aster-tools', 'go'))
  return [...new Set(candidates)]
}

export function resolveGoCommand(command, repositoryRoot) {
  for (const goRoot of localGoRoots(repositoryRoot)) {
    const candidate = resolve(goRoot, 'bin', executableName(command))
    if (existsSync(candidate)) return candidate
  }
  return command
}
