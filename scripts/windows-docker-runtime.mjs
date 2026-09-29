import { spawnSync } from 'node:child_process'
import { existsSync, readFileSync, statSync } from 'node:fs'
import { delimiter, dirname, resolve } from 'node:path'

function pathKey(environment) {
  return Object.keys(environment).find(key => key.toLowerCase() === 'path') || 'Path'
}

function ordinaryFile(path) {
  try {
    return existsSync(path) && statSync(path).isFile()
  } catch {
    return false
  }
}

function whereCandidates(name, environment, execute) {
  const result = execute('where.exe', [name], {
    env: environment,
    encoding: 'utf8',
    windowsHide: true,
  })
  if (result.error || result.status !== 0) return []
  return String(result.stdout || '').split(/\r?\n/).map(value => value.trim()).filter(Boolean)
}

function dockerConfigurationPath(environment) {
  const configuredDirectory = String(environment.DOCKER_CONFIG || '').trim()
  if (configuredDirectory) return resolve(configuredDirectory, 'config.json')
  const profile = String(environment.USERPROFILE || '').trim()
  return profile ? resolve(profile, '.docker', 'config.json') : ''
}

export function configuredDockerCredentialStore(environment = process.env) {
  const configurationPath = dockerConfigurationPath(environment)
  if (!configurationPath || !ordinaryFile(configurationPath)) return { configurationPath, stores: [] }
  let configuration
  try {
    configuration = JSON.parse(readFileSync(configurationPath, 'utf8'))
  } catch (error) {
    throw new Error(`Docker configuration is invalid: ${configurationPath}: ${error.message}`)
  }
  const values = []
  if (configuration?.credsStore !== undefined) values.push(configuration.credsStore)
  if (configuration?.credHelpers !== undefined) {
    if (!configuration.credHelpers || typeof configuration.credHelpers !== 'object'
        || Array.isArray(configuration.credHelpers)) {
      throw new Error(`Docker credHelpers is invalid: ${configurationPath}`)
    }
    values.push(...Object.values(configuration.credHelpers))
  }
  for (const store of values) {
    if (typeof store !== 'string' || !/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(store)) {
      throw new Error(`Docker credential helper name is invalid: ${configurationPath}`)
    }
  }
  return { configurationPath, stores: [...new Set(values)] }
}

export function resolveWindowsDockerRuntime({
  environment = process.env,
  execute = spawnSync,
  requireEngine = true,
} = {}) {
  const programFiles = String(environment.ProgramFiles || 'C:\\Program Files')
  const candidates = [
    ...whereCandidates('docker.exe', environment, execute),
    resolve(programFiles, 'Docker', 'Docker', 'resources', 'bin', 'docker.exe'),
  ]
  const executable = candidates.find(ordinaryFile)
  if (!executable) {
    throw new Error('Docker Desktop CLI was not found on PATH or below ProgramFiles')
  }

  const binDirectory = dirname(executable)
  const key = pathKey(environment)
  const childEnvironment = {
    ...environment,
    [key]: `${binDirectory}${delimiter}${environment[key] || ''}`,
  }
  const { configurationPath, stores } = configuredDockerCredentialStore(childEnvironment)
  const credentialHelpers = []
  for (const store of stores) {
    const helperName = `docker-credential-${store}.exe`
    const credentialHelper = [
      resolve(binDirectory, helperName),
      ...whereCandidates(helperName, childEnvironment, execute),
    ].find(ordinaryFile) || ''
    if (!credentialHelper) {
      throw new Error(
        `Docker is configured with credsStore=${store}, but ${helperName} was not found beside docker.exe or on PATH`,
      )
    }
    credentialHelpers.push(credentialHelper)
  }

  if (requireEngine) {
    const result = execute(executable, ['info'], {
      env: childEnvironment,
      encoding: 'utf8',
      windowsHide: true,
      timeout: 8_000,
    })
    if (result.error || result.status !== 0) {
      throw new Error('Docker Desktop is installed, but its Engine is not ready for the current user')
    }
  }

  return {
    executable,
    binDirectory,
    credentialHelper: credentialHelpers[0] || '',
    credentialHelpers,
    configurationPath,
    environment: childEnvironment,
  }
}
