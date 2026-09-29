import { createPrivateKey, createPublicKey, randomBytes } from 'node:crypto'
import { spawnSync } from 'node:child_process'
import { chmodSync, existsSync, lstatSync, mkdirSync, readFileSync, renameSync, writeFileSync } from 'node:fs'
import { dirname, resolve } from 'node:path'
import { officialPluginProviders, officialPluginReleaseProfile, stageOfficialPlugins } from './official-plugin-release.mjs'

function run(command, args, root, environment) {
  const result = spawnSync(command, args, {
    cwd: root, env: environment, stdio: 'inherit', windowsHide: true,
    shell: command === 'cargo' && process.platform === 'win32',
  })
  if (result.error) throw result.error
  if (result.status !== 0) throw new Error(`本地官方插件准备失败：${command} 退出码 ${result.status ?? 'unknown'}`)
}

export function prepareLocalOfficialPlugin(root, environment) {
  if (environment.ASTER_PLUGIN_TRUSTED_KEYS_JSON?.trim()) return

  const seedFile = resolve(root, 'data/local/plugin-signing.seed')
  const installRoot = resolve(root, 'data')
  const layout = JSON.parse(readFileSync(resolve(root, 'contracts/install-layout.json'), 'utf8'))
  const incomingDirectory = resolve(installRoot, layout.paths.plugin_incoming_dir)
  const activeDirectory = resolve(installRoot, layout.paths.plugin_state_dir)
  if (!existsSync(seedFile) && officialPluginProviders.some(provider => existsSync(resolve(activeDirectory, `${provider}.json`)))) {
    throw new Error('本地插件签名密钥缺失，但已有活动插件记录；请恢复原密钥后再启动 Control')
  }
  mkdirSync(dirname(seedFile), { recursive: true, mode: 0o700 })
  if (!existsSync(seedFile)) writeFileSync(seedFile, randomBytes(32), { flag: 'wx', mode: 0o600 })
  const seedStat = lstatSync(seedFile)
  const seed = readFileSync(seedFile)
  if (!seedStat.isFile() || seedStat.isSymbolicLink() || seed.length !== 32) {
    throw new Error('本地插件签名密钥必须是 32 字节的普通文件')
  }
  chmodSync(seedFile, 0o600)
  const privateKey = createPrivateKey({
    key: Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed]),
    format: 'der', type: 'pkcs8',
  })
  const publicKey = createPublicKey(privateKey).export({ format: 'der', type: 'spki' }).subarray(-32).toString('base64')
  environment.ASTER_PLUGIN_TRUSTED_KEYS_JSON = JSON.stringify(officialPluginProviders.map(provider => ({
    key_id: `local-${provider}-v1`, bundle_id: `aster.${provider}`, public_key_base64: publicKey,
  })))
  const profile = officialPluginReleaseProfile({
    ASTER_PLUGIN_TRUSTED_KEYS_JSON: environment.ASTER_PLUGIN_TRUSTED_KEYS_JSON,
    ASTER_PLUGIN_SIGNING_KEY_FILE: seedFile,
  })

  run('cargo', ['build', '-p', 'aster-plugin-core', '--features', 'signing', '--bin', 'aster-sign-plugin'], root, environment)
  const targetDirectory = environment.CARGO_TARGET_DIR ? resolve(root, environment.CARGO_TARGET_DIR) : resolve(root, 'target')
  const signer = resolve(targetDirectory, 'debug', process.platform === 'win32' ? 'aster-sign-plugin.exe' : 'aster-sign-plugin')
  const staging = resolve(root, 'data/local/plugin-staging')
  mkdirSync(resolve(staging, 'plugins'), { recursive: true, mode: 0o700 })
  const version = JSON.parse(readFileSync(resolve(root, 'package.json'), 'utf8')).version
  stageOfficialPlugins({ run: (command, args) => run(command, args, root, environment), signer, root, bundle: staging, version, profile })
  mkdirSync(incomingDirectory, { recursive: true, mode: 0o700 })
  for (const provider of officialPluginProviders) {
    renameSync(resolve(staging, 'plugins', `${provider}.asterlua`), resolve(incomingDirectory, `${provider}.asterlua`))
  }
}
