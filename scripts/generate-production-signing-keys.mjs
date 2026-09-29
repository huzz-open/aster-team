import {
  createHash,
  createPrivateKey,
  createPublicKey,
  generateKeyPairSync,
} from 'node:crypto'
import {
  chmodSync,
  existsSync,
  mkdirSync,
  readdirSync,
  readFileSync,
  realpathSync,
  lstatSync,
  writeFileSync,
} from 'node:fs'
import { dirname, isAbsolute, relative, resolve, sep } from 'node:path'
import { fileURLToPath } from 'node:url'
import { generateLicenseSigners, readLicenseSigningProfile, readUniqueJSON } from './license-signing-profile.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..')
const outputArgument = process.argv[2]

const policyArgument = process.argv[3]
if (process.argv.length !== 4 || !outputArgument || !isAbsolute(outputArgument) || !policyArgument || !isAbsolute(policyArgument)) {
  throw new Error('Usage: node scripts/generate-production-signing-keys.mjs ABSOLUTE_OUTPUT_DIRECTORY ABSOLUTE_ISSUER_POLICY_JSON')
}

const output = resolve(outputArgument)
// Resolve existing ancestors to reject a directory symlink back into the repo.
let ancestor = output
while (!existsSync(ancestor)) ancestor = dirname(ancestor)
const resolvedOutput = resolve(realpathSync(ancestor), relative(ancestor, output))
const repositoryRelative = relative(realpathSync(root), resolvedOutput)
if (repositoryRelative === '' || (repositoryRelative !== '..' && !repositoryRelative.startsWith(`..${sep}`) && !isAbsolute(repositoryRelative))) {
  throw new Error('Production private keys must be generated outside the source repository')
}
if (existsSync(output) && readdirSync(output).length > 0) {
  throw new Error(`Refusing to overwrite non-empty key directory: ${output}`)
}

const ed25519PKCS8Prefix = Buffer.from('302e020100300506032b657004220420', 'hex')
const releaseKeyID = 'release-v1'
const release = generateKeyPairSync('ed25519')
const releasePrivatePKCS8 = release.privateKey.export({ format: 'der', type: 'pkcs8' })
const releasePublicSPKI = release.publicKey.export({ format: 'der', type: 'spki' })

if (releasePrivatePKCS8.length !== ed25519PKCS8Prefix.length + 32 ||
    !releasePrivatePKCS8.subarray(0, ed25519PKCS8Prefix.length).equals(ed25519PKCS8Prefix)) {
  throw new Error('The Node.js runtime returned an unsupported Ed25519 PKCS#8 encoding')
}
const releaseSeed = releasePrivatePKCS8.subarray(ed25519PKCS8Prefix.length)
const reconstructedReleasePublic = createPublicKey(createPrivateKey({
  key: Buffer.concat([ed25519PKCS8Prefix, releaseSeed]),
  format: 'der',
  type: 'pkcs8',
})).export({ format: 'der', type: 'spki' })
if (!reconstructedReleasePublic.equals(releasePublicSPKI)) {
  throw new Error('Generated signing keys failed their independence or round-trip check')
}

const policyFile = lstatSync(policyArgument)
if (!policyFile.isFile() || policyFile.isSymbolicLink()) throw new Error('Issuer policies must be an ordinary file')
const policies = readUniqueJSON(readFileSync(policyArgument, 'utf8'), 'Issuer policies')
const releasePublicJSON = JSON.stringify([{ key_id: releaseKeyID, public_key_spki: releasePublicSPKI.toString('base64url') }])
const profile = readLicenseSigningProfile(JSON.stringify(generateLicenseSigners(policies)), releasePublicJSON)

mkdirSync(output, { recursive: true, mode: 0o700 })
const writePrivate = (name, value) => writeFileSync(resolve(output, name), value, { flag: 'wx', mode: 0o600 })
const writePublic = (name, value) => writeFileSync(resolve(output, name), value, { flag: 'wx', mode: 0o644 })
const publicKeyring = (keyID, publicKey) => `${JSON.stringify([{
  key_id: keyID,
  public_key_spki: publicKey.toString('base64url'),
}], null, 2)}\n`
const fingerprint = (value) => `SHA256:${createHash('sha256').update(value).digest('base64')}`

writePrivate('license-v2.signers.json', `${JSON.stringify(JSON.parse(profile.licenseSignersJSON), null, 2)}\n`)
writePrivate('release-v1.seed', releaseSeed)
writePublic('license-v2.public-keyring.json', `${JSON.stringify(profile.licenseTrustedKeys, null, 2)}\n`)
writePublic('release-v1.public-keyring.json', publicKeyring(releaseKeyID, releasePublicSPKI))
writePublic('public-fingerprints.txt', [
  ...profile.licenseTrustedKeys.map(entry => `${entry.key_id} ${fingerprint(Buffer.from(entry.public_key_spki, 'base64url'))}`),
  `${releaseKeyID} ${fingerprint(releasePublicSPKI)}`,
  '',
].join('\n'))
writePrivate('KEY-CUSTODY.md', `# Aster Team production signing keys

This directory is the local recovery copy and must never be committed or shared as a whole.

## Secret files that must be backed up and must not leak

- \`license-v2.signers.json\`: exact value for Operations
  \`ASTER_OPERATIONS_LICENSE_V2_SIGNERS_JSON\`.
- \`release-v1.seed\`: raw 32-byte value used to sign Customer releases. GitHub stores
  its standard Base64 encoding as \`ASTER_RELEASE_SIGNING_SEED_BASE64\`.

Loss of the License private key prevents future license issuance. Its disclosure permits
forged licenses. Loss of the Release seed prevents normal trusted upgrades. Its disclosure
permits forged installation and upgrade packages.

## Public files that are safe to copy

- \`license-v2.public-keyring.json\`
- \`release-v1.public-keyring.json\`
- \`public-fingerprints.txt\`

Public keyrings are embedded into Customer binaries and stored in GitHub Environment
variables. Keep them with the private-key backup so a restore can be verified.

Maintain at least two encrypted backups in separate locations. GitHub Secrets are not a
recoverable backup because their values cannot be read back after creation.
`)

for (const name of ['license-v2.signers.json', 'release-v1.seed', 'KEY-CUSTODY.md']) {
  chmodSync(resolve(output, name), 0o600)
}

console.log(`Generated independent scoped License and Release signing keys in ${output}`)
console.log('Secret recovery files: license-v2.signers.json, release-v1.seed')
console.log('Public files: license-v2.public-keyring.json, release-v1.public-keyring.json, public-fingerprints.txt')
