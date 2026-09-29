import { createPrivateKey, createPublicKey } from 'node:crypto'
import { copyFileSync, existsSync, lstatSync, mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

export const officialPluginProviders = ['openai', 'deepseek', 'glm']
const keyIDPattern = /^[A-Za-z0-9_.:-]{3,128}$/

function ordinaryFile(path, label, expectedSize) {
  if (!existsSync(path)) throw new Error(`${label} is missing`)
  const stat = lstatSync(path)
  if (!stat.isFile() || stat.isSymbolicLink()) throw new Error(`${label} must be an ordinary file`)
  if (expectedSize !== undefined && stat.size !== expectedSize) {
    throw new Error(`${label} must contain exactly ${expectedSize} bytes`)
  }
}

export function officialPluginReleaseProfile(environment = process.env) {
  let entries
  try { entries = JSON.parse(environment.ASTER_PLUGIN_TRUSTED_KEYS_JSON || '') }
  catch { throw new Error('ASTER_PLUGIN_TRUSTED_KEYS_JSON is invalid') }
  if (!Array.isArray(entries) || entries.length !== officialPluginProviders.length) {
    throw new Error('The plugin keyring must contain one publisher per provider')
  }
  const signingKeyFile = resolve(environment.ASTER_PLUGIN_SIGNING_KEY_FILE || '')
  ordinaryFile(signingKeyFile, 'Plugin signing seed', 32)
  const seed = readFileSync(signingKeyFile)
  const privateKey = createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]),
    format: 'der', type: 'pkcs8',
  })
  const publicKey = createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64')
  const keys = {}
  const ids = new Set()
  for (const entry of entries) {
    const provider = officialPluginProviders.find(name => entry?.bundle_id === `aster.${name}`)
    if (!provider || keys[provider] || !keyIDPattern.test(entry.key_id)
      || ids.has(entry.key_id) || entry.public_key_base64 !== publicKey) {
      throw new Error('Plugin publisher entry does not match the signing seed and provider list')
    }
    keys[provider] = entry.key_id
    ids.add(entry.key_id)
  }
  if (officialPluginProviders.some(provider => !keys[provider])) {
    throw new Error('The plugin keyring is missing a provider')
  }
  if (environment.ASTER_PLUGIN_SIGNING_KEY_ID
    && environment.ASTER_PLUGIN_SIGNING_KEY_ID !== keys.openai) {
    throw new Error('ASTER_PLUGIN_SIGNING_KEY_ID must identify the OpenAI publisher')
  }
  return { keyringJSON: JSON.stringify(entries), signingKeyFile, signingKeyID: keys.openai, keys }
}

export function stageOfficialPlugins({ run, signer, root, bundle, version, profile }) {
  const temporary = mkdtempSync(join(tmpdir(), 'aster-plugin-sources-'))
  try {
    const shared = resolve(root, 'customer/plugins/shared')
    const rules = JSON.parse(readFileSync(resolve(shared, 'public-rules.json'), 'utf8'))
    const channels = JSON.parse(readFileSync(resolve(shared, 'channels.json'), 'utf8'))
    const outputDirectory = resolve(bundle, 'plugins')
    mkdirSync(outputDirectory, { recursive: true })
    for (const provider of officialPluginProviders) {
      const source = resolve(temporary, provider)
      const metadata = resolve(root, 'customer/plugins', provider)
      mkdirSync(source)
      for (const name of ['entrypoints.json', 'plugin-info.json']) {
        copyFileSync(resolve(metadata, name), resolve(source, name))
      }
      copyFileSync(resolve(shared, 'gateway.lua'), resolve(source, 'gateway.lua'))
      writeFileSync(resolve(source, 'provider.json'), JSON.stringify({ provider }))
      writeFileSync(resolve(source, 'channels.json'), JSON.stringify(Object.fromEntries(
        Object.entries(channels).filter(([, channel]) => channel.provider === provider),
      )))
      const selected = { schema: rules.schema, revision: rules.revision }
      for (const section of ['models', 'image_models']) {
        selected[section] = Object.fromEntries(Object.entries(rules[section] || {})
          .filter(([, rule]) => rule.provider === provider))
      }
      writeFileSync(resolve(source, 'public-rules.json'), JSON.stringify(selected))
      const output = resolve(outputDirectory, `${provider}.asterlua`)
      const pluginVersion = JSON.parse(readFileSync(resolve(metadata, 'plugin-version.json'), 'utf8')).version
      if (typeof pluginVersion !== 'string' || !/^\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?$/.test(pluginVersion)) {
        throw new Error(`${provider} plugin version is invalid`)
      }
      run(signer, [source, output, profile.signingKeyFile, profile.keys[provider],
        `aster.${provider}`, pluginVersion, version, '2'])
      ordinaryFile(output, `Signed ${provider} plugin`)
    }
  } finally {
    rmSync(temporary, { recursive: true, force: true })
  }
}
