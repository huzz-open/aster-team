import { createHash, generateKeyPairSync, sign } from 'node:crypto'
import { mkdir, readFile, symlink, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'

const canonical = value => JSON.stringify(value, (_key, item) => item && typeof item === 'object' && !Array.isArray(item)
  ? Object.fromEntries(Object.keys(item).sort().map(key => [key, item[key]])) : item)

// Only the isolated local-demo Control process trusts this ephemeral public key.
// The private key remains in memory; this is not a formal Customer release.
export async function prepareRelease(root, directory) {
  const source = process.env.ASTER_LICENSE_BROWSER_ASTERCTL
  if (!source) throw new Error('Run npm run test:license-delivery:downloads:windows to build the actual asterctl')
  const binary = await readFile(source)
  if (binary.subarray(0, 2).toString('ascii') !== 'MZ') throw new Error('Expected the built Windows asterctl executable')
  const { version } = JSON.parse(await readFile(join(root, 'package.json'), 'utf8'))
  const { paths } = JSON.parse(await readFile(join(root, 'contracts/install-layout.json'), 'utf8'))
  const install_root = join(directory, 'installation')
  const releaseRoot = join(install_root, paths.releases, version)
  const relativePath = 'client-tools/asterctl/windows-x86_64/asterctl.exe'
  const artifact_path = join(releaseRoot, relativePath)
  await mkdir(dirname(artifact_path), { recursive: true })
  await writeFile(artifact_path, binary)
  const sha256 = createHash('sha256').update(binary).digest('hex')
  const { privateKey, publicKey } = generateKeyPairSync('ed25519')
  const claims = {
    schema: 'aster.release-manifest.v1', key_id: 'browser-release-test', product: 'aster-team',
    version, platform: 'windows', architecture: 'amd64', runtime: 'msvc',
    created_at: '2026-09-07T00:00:00.000Z',
    files: [{ path: relativePath, size: binary.length, sha256, executable: false }],
  }
  const signature = sign(null, Buffer.from(canonical(claims)), privateKey).toString('base64url')
  const manifest_path = join(releaseRoot, 'RELEASE.json')
  await writeFile(manifest_path, JSON.stringify({ ...claims, signature }))
  await symlink(releaseRoot, join(install_root, paths.current), 'junction')
  return {
    install_root, artifact_path, manifest_path, version, sha256, size_bytes: binary.length,
    id: `asterctl-${version}-windows-x86_64`,
    trusted_keys: JSON.stringify([{ key_id: claims.key_id, public_key_spki: publicKey.export({ type: 'spki', format: 'der' }).toString('base64url') }]),
  }
}
