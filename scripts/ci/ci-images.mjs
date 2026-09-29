import { readFileSync } from 'node:fs'
import { resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import { maintenanceTargets, recipeHash } from './base-images.mjs'

const root = fileURLToPath(new URL('../../', import.meta.url))
export function validateLock(lock, targets) {
  if (lock.schema_version !== 1 || lock.platform !== 'linux/amd64'
    || !Array.isArray(lock.images) || lock.images.length !== targets.length) throw new Error('Incomplete CI image lock; publish every base locally first')
  return targets.map(target => {
    const matches = lock.images.filter(item => item.target === target.id)
    const item = matches[0]
    if (matches.length !== 1 || !item.image?.startsWith(`${target.repository}@`)
      || !/^sha256:[a-f0-9]{64}$/.test(item.image.slice(target.repository.length + 1))
      || !/^[a-f0-9]{64}$/.test(item.recipe) || !/^sha256:[a-f0-9]{64}$/.test(item.image_id)) {
      throw new Error(`Invalid pinned image for ${target.id}`)
    }
    // The manifest defines role/source metadata; the lock only supplies provenance
    // and the published immutable image. A resolved upstream digest is not the
    // original smoke-matrix tag (for example ubuntu:20.04).
    return { ...target, target: target.id, image: item.image, recipe: item.recipe,
      image_id: item.image_id, upstream_base: item.base }
  })
}

export function lockedImages() {
  const manifest = JSON.parse(readFileSync(resolve(root, 'contracts/release-platforms.json'), 'utf8'))
  let lock
  try { lock = JSON.parse(readFileSync(resolve(root, 'tools/ci-base-images.lock.json'), 'utf8')) }
  catch { throw new Error('No published CI image lock. Run npm run ci:images:update locally; no build fallback is allowed.') }
  const targets = maintenanceTargets(manifest)
  const images = validateLock(lock, targets)
  const maintainer = readFileSync(resolve(root, 'scripts/ci/base-images.mjs'), 'utf8')
  for (const target of targets) {
    const file = target.kind === 'builder' ? 'scripts/ci/linux-lab.Dockerfile' : `scripts/ci/systemd/${target.family}.Dockerfile`
    const validator = `scripts/ci/systemd/validate-${target.kind === 'builder' ? 'toolchain' : 'base'}.sh`
    const expected = recipeHash(target, readFileSync(resolve(root, file), 'utf8'), readFileSync(resolve(root, validator), 'utf8'), maintainer)
    if (images.find(item => item.target === target.id).recipe !== expected) throw new Error(`CI image recipe changed: ${target.id}; publish the validated update before using it`)
  }
  return images
}

export function resolveCiImage(id, kind) {
  const image = lockedImages().find(item => item.target === id)
  if (!image || (kind && image.kind !== kind)) throw new Error(`Wrong or unknown CI image kind: ${id}`)
  return image
}

if (resolve(process.argv[1] || '') === fileURLToPath(import.meta.url)) {
  try {
    if (process.argv[2] === '--check' && process.argv.length === 3) console.log(`Validated ${lockedImages().length} pinned CI images`)
    else if ([3, 5].includes(process.argv.length)) {
      const image = resolveCiImage(process.argv[2])
      if (process.argv.length === 5 && (process.argv[3] !== image.base || process.argv[4] !== image.family)) {
        throw new Error('Smoke target source metadata does not match the pinned runtime image')
      }
      console.log(image.image)
    }
    else throw new Error('Usage: node scripts/ci/ci-images.mjs TARGET|--check')
  } catch (error) { console.error(error.message); process.exitCode = 1 }
}
