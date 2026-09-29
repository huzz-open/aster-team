import { existsSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'

export function createSystemCommandRunner(repositoryRoot, baseEnvironment = process.env) {
  function executable(name) {
    if (process.platform !== 'win32') return name
    if (name === 'docker') {
      const candidate = 'C:\\Program Files\\Docker\\Docker\\resources\\bin\\docker.exe'
      if (existsSync(candidate)) return candidate
    }
    if (name === 'bash') {
      const git = spawnSync('git', ['--exec-path'], { cwd: repositoryRoot, encoding: 'utf8' })
      if (git.status === 0) {
        const candidate = resolve(git.stdout.trim(), '..', '..', '..', 'bin', 'bash.exe')
        if (existsSync(candidate)) return candidate
      }
    }
    if (name === 'openssl') {
      const bash = executable('bash')
      const candidate = resolve(dirname(bash), '..', 'usr', 'bin', 'openssl.exe')
      if (existsSync(candidate)) return candidate
    }
    return name
  }

  const dockerDirectory = dirname(executable('docker'))
  const environment = {
    ...baseEnvironment,
    PATH: process.platform === 'win32'
      ? `${dockerDirectory};${baseEnvironment.PATH || ''}`
      : baseEnvironment.PATH,
  }

  function run(command, commandArgs, options = {}) {
    const result = spawnSync(command, commandArgs, {
      cwd: repositoryRoot,
      stdio: options.capture ? ['ignore', 'pipe', 'pipe'] : 'inherit',
      encoding: options.capture ? 'utf8' : undefined,
      env: { ...environment, ...options.env },
    })
    if (result.error) throw result.error
    if (result.status !== 0) {
      const detail = options.capture ? `\n${result.stdout || ''}${result.stderr || ''}` : ''
      throw new Error(`${command} ${commandArgs.join(' ')} exited with ${result.status}${detail}`)
    }
    return options.capture ? result.stdout : ''
  }

  function runNpm(commandArgs, options = {}) {
    if (baseEnvironment.npm_execpath) {
      return run(process.execPath, [baseEnvironment.npm_execpath, ...commandArgs], options)
    }
    return run(executable('npm'), commandArgs, options)
  }

  function bashPath(path) {
    if (process.platform !== 'win32') return path.replaceAll('\\', '/')
    const normalized = resolve(path).replaceAll('\\', '/')
    const match = normalized.match(/^([A-Za-z]):\/(.*)$/)
    return match ? `/${match[1].toLowerCase()}/${match[2]}` : normalized
  }

  return { bashPath, environment, executable, run, runNpm }
}
