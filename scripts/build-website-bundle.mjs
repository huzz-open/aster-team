import { createHash } from 'node:crypto'
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { spawnSync } from 'node:child_process'
import { fileURLToPath } from 'node:url'
import { createTarGz } from './release-archive.mjs'
import { verifyArtifact, verifySources } from './verify-release-boundaries.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const npm = process.platform === 'win32' ? 'npm.cmd' : 'npm'
const version = (process.env.ASTER_WEBSITE_RELEASE_VERSION || '0.1.0').replace(/^website-v/, '').replace(/^v/, '')
if (!/^[0-9A-Za-z._-]+$/.test(version)) throw new Error('ASTER_WEBSITE_RELEASE_VERSION is invalid')
const contactEmail = String(process.env.VITE_ASTER_CONTACT_EMAIL || '').trim()
const contactWechat = String(process.env.VITE_ASTER_CONTACT_WECHAT || '').trim()
if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(contactEmail)) {
  throw new Error('VITE_ASTER_CONTACT_EMAIL must be a valid public contact email')
}
if (!contactWechat || /[\r\n]/.test(contactWechat)) {
  throw new Error('VITE_ASTER_CONTACT_WECHAT must be a non-empty public contact identifier')
}
const result = spawnSync(npm, ['run', 'build', '--workspace', '@aster/website'], { cwd: root, stdio: 'inherit', shell: process.platform === 'win32' })
if (result.error) throw result.error
if (result.status !== 0) process.exit(result.status ?? 1)
const sourceFailures = verifySources('website')
if (sourceFailures.length) throw new Error(sourceFailures.join('\n'))
const bundleName = `aster-website-${version}`
const outputRoot = resolve(root, 'dist', 'website-release')
const bundle = resolve(outputRoot, bundleName)
rmSync(bundle, { recursive: true, force: true })
mkdirSync(bundle, { recursive: true })
cpSync(resolve(root, 'website/dist'), bundle, { recursive: true })
const failures = verifyArtifact('website', bundle)
if (failures.length) throw new Error(failures.join('\n'))
const archive = resolve(outputRoot, `${bundleName}.tar.gz`)
rmSync(archive, { force: true })
createTarGz(bundle, archive, bundleName)
const checksum = createHash('sha256').update(readFileSync(archive)).digest('hex')
writeFileSync(`${archive}.sha256`, `${checksum}  ${bundleName}.tar.gz\n`)
console.log(`Website bundle: ${archive}`)
console.log(`SHA-256: ${checksum}`)
