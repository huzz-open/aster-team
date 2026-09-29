import { spawnSync } from 'node:child_process'
import { appendFileSync } from 'node:fs'
import { isAbsolute, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

export function npmCachePath(execute = spawnSync) {
  const result = execute(process.platform === 'win32' ? 'npm.cmd' : 'npm', ['config', 'get', 'cache'], {
    encoding: 'utf8', shell: process.platform === 'win32', windowsHide: true,
  })
  if (result.error || result.status !== 0) throw new Error('Cannot locate the npm download cache')
  const path = result.stdout.trim()
  if (!isAbsolute(path) || /[\r\n\0]/.test(path)) throw new Error('npm returned an invalid cache path')
  return path
}

if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const path = npmCachePath()
  console.log(`npm download cache: ${path}`)
  if (process.env.GITHUB_OUTPUT) appendFileSync(process.env.GITHUB_OUTPUT, `path=${path}\n`)
}
