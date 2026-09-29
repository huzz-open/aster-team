import assert from 'node:assert/strict'
import { appendFile, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { spawnSync } from 'node:child_process'
import { PassThrough } from 'node:stream'
import { createHash } from 'node:crypto'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import test from 'node:test'
import { createMsiLogReader, createMsiProgress, initializeDatabaseRootPassword, installerNeedsRestart, mariaDBElevatedScript, prepareMariaDBInstaller } from './setup-windows-environment.mjs'
import { askSecret, collectNativeChoices, nativePackageOverride, parseSetupArguments, readSetupSettings, resolveRootPassword, rootPasswordSql, selectInstallRoot, setupEnvironment, validateInstallRoot, windowsInstallPaths, writeSetupSettings } from './windows-setup-options.mjs'
import { configuredWindowsToolsRoot, preferredGoRoot } from '../tools/toolchains/go-toolchain.mjs'
import { enableWindowsFeature, createSetupLogProgress, downloadProgress, downloadVerifiedMariaDB, executeWithProgress, installerSucceeded, linuxLabPackages, nativePackages, renderDownloadBar, versionAtLeast } from './setup-windows-environment.mjs'
import { acquireSetupLock, githubEnvironment, githubRepository, prepareGitHubAuthentication, readGitHubAuth, runNativeSetupPhases } from './windows-setup-github.mjs'

test('Windows feature setup accepts DISM success and reboot-required results without restarting', () => {
  for (const status of [0, 3010]) {
    const messages = []
    const restartRequired = enableWindowsFeature('VirtualMachinePlatform', {
      featureState: () => false,
      run: (command, args, options) => {
        assert.equal(command, 'dism.exe')
        assert.deepEqual(args, ['/Online', '/Enable-Feature', '/FeatureName:VirtualMachinePlatform', '/All', '/NoRestart'])
        assert.equal(options.capture, false)
        return { status }
      },
      log: message => messages.push(message),
    })
    assert.equal(restartRequired, true)
    if (status === 3010) assert.match(messages.at(-1), /已启用.*需要重启 Windows.*3010/)
  }
})

test('Windows feature setup preserves actual failures and includes diagnostics', () => {
  for (const [result, expected] of [
    [{ status: 5 }, /退出码 5/],
    [{ status: 87 }, /退出码 87/],
    [{ status: null }, /退出码 unknown/],
    [{ status: 3010, error: new Error('spawn failed') }, /spawn failed/],
  ]) {
    assert.throws(() => enableWindowsFeature('VirtualMachinePlatform', {
      featureState: () => false, run: () => result, log: () => {},
    }), expected)
  }
})

test('Windows feature setup skips features already enabled or inferred from working WSL', () => {
  for (const state of [true, null]) {
    assert.equal(enableWindowsFeature('VirtualMachinePlatform', {
      featureState: () => state, isWslReady: () => true,
      run: () => assert.fail('DISM must not run for an enabled feature'), log: () => {},
    }), false)
  }
})

test('Windows native and optional Linux lab dependencies stay separate', () => {
  assert.deepEqual(nativePackages.map(value => value.id), [
    'Git.Git', 'GitHub.cli', 'Python.Python.3.13', 'Rustlang.Rustup', 'Microsoft.VisualStudio.BuildTools', 'MariaDB.Server',
  ])
  assert.deepEqual(linuxLabPackages.map(value => value.id), ['Microsoft.WSL', 'Docker.DockerDesktop'])
  assert.equal(nativePackages.some(value => /WSL|Docker/.test(value.id)), false)
})

test('Windows native setup pins the required compiler components and supports unattended Go setup', async () => {
  const setup = await readFile(new URL('./setup-windows-environment.mjs', import.meta.url), 'utf8')
  const go = await readFile(new URL('./install-go-toolchain.sh', import.meta.url), 'utf8')
  const launcher = await readFile(new URL('./run-go-toolchain-setup.mjs', import.meta.url), 'utf8')
  assert.match(setup, /Microsoft\.VisualStudio\.Component\.VC\.Tools\.x86\.x64/)
  assert.match(setup, /Microsoft\.VisualStudio\.Component\.Windows11SDK\.26100/)
  assert.match(setup, /install-go-toolchain\.sh', '--yes'/)
  assert.match(go, /--yes\) assume_yes=1/)
  assert.match(launcher, /gitBashPath/)
  assert.doesNotMatch(launcher, /System32.*bash/i)
})

test('Linux lab setup enables WSL 2 features but never mixes them into local database initialization', async () => {
  const setup = await readFile(new URL('./setup-windows-environment.mjs', import.meta.url), 'utf8')
  const local = await readFile(new URL('./setup-local-stack.mjs', import.meta.url), 'utf8')
  assert.match(setup, /Microsoft-Windows-Subsystem-Linux/)
  assert.match(setup, /VirtualMachinePlatform/)
  assert.match(setup, /--set-default-version', '2'/)
  assert.doesNotMatch(local, /Docker\.DockerDesktop|Microsoft\.WSL|wsl\.exe/)
})

test('Node minimum version comparison follows semantic version ordering', () => {
  assert.equal(versionAtLeast('22.19.0', [22, 19, 0]), true)
  assert.equal(versionAtLeast('22.18.9', [22, 19, 0]), false)
  assert.equal(versionAtLeast('23.0.0', [22, 19, 0]), true)
})

test('silent child processes emit progress and preserve nonzero exit codes', async () => {
  const messages = []
  const cursorWrites = []
  const result = await executeWithProgress(process.execPath, ['-e', 'setTimeout(() => process.exit(7), 150)'], {
    intervalMs: 20, label: 'test installer', log: message => messages.push(message), progress: () => 'downloading compiler',
    output: { isTTY: true, write: value => cursorWrites.push(value) },
  })
  assert.equal(result.status, 7)
  assert.deepEqual(cursorWrites, ['\x1b[?25l', '\x1b[?25h'])
  assert.ok(messages.some(message => message.includes('[等待') && message.includes('downloading compiler')))
  const count = messages.length
  await new Promise(resolve => setTimeout(resolve, 60))
  assert.equal(messages.length, count, 'heartbeat stops when the child exits')
})

test('failure to spawn is reported and stops the heartbeat', async () => {
  const messages = []
  const result = await executeWithProgress('aster-nonexistent-installer-501f', [], {
    intervalMs: 10, log: message => messages.push(message),
  })
  assert.equal(result.error.code, 'ENOENT')
  const count = messages.length
  await new Promise(resolve => setTimeout(resolve, 30))
  assert.equal(messages.length, count)
})

test('MSVC progress follows only new session logs and filters command lines', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-setup-progress-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  await writeFile(join(directory, 'dd_setup_20260908090000.log'), '[a][2026-09-08T09:00:00] Completed: Installing OLD\n')
  const progress = createSetupLogProgress(directory)
  assert.match(progress(), /等待安装器/)
  const file = join(directory, 'dd_setup_20260908110000.log')
  await writeFile(file, '[a][2026-09-08T11:00:00] Command line: secret\n[a][2026-09-08T11:00:01] BEGIN: Downloading package "Compiler"\n')
  assert.match(progress(), /Downloading package "Compiler"/)
  assert.doesNotMatch(progress(), /secret|OLD/)
  await appendFile(file, '[a][2026-09-08T11:00:02] Completed: Installing Compiler\n')
  assert.match(progress(), /Completed: Installing Compiler/)
  assert.match(progress(), /无新日志/)
})

test('MariaDB retries a failed download at the archive and verifies bytes before installation', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-download-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const destination = join(directory, 'installer.msi')
  const payload = Buffer.from('installer fixture')
  const urls = ['https://mirror.example/installer.msi', 'https://archive.example/installer.msi']
  const calls = []
  await downloadVerifiedMariaDB(destination, {
    download: { version: 'test', urls, sha256: createHash('sha256').update(payload).digest('hex') }, log: () => {},
    run: async (command, args) => {
      assert.equal(command, 'curl.exe')
      assert.ok(args.includes('--silent'))
      assert.ok(args.includes('--show-error'))
      assert.equal(args.includes('--progress-bar'), false, 'curl must not compete with line-based progress output')
      calls.push(args.at(-1))
      if (calls.length === 1) { await writeFile(destination, '403 partial response'); return { status: 22 } }
      await assert.rejects(readFile(destination), { code: 'ENOENT' })
      await writeFile(destination, payload)
      return { status: 0 }
    },
  })
  assert.deepEqual(calls, urls)
  assert.deepEqual(await readFile(destination), payload)
})

test('MariaDB refuses a successful HTTP response with the wrong checksum', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-download-hash-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const destination = join(directory, 'installer.msi')
  await assert.rejects(downloadVerifiedMariaDB(destination, {
    download: { version: 'test', urls: ['https://mirror.example/installer.msi'], sha256: '0'.repeat(64) }, log: () => {},
    run: async () => { await writeFile(destination, 'invalid'); return { status: 0 } },
  }), /SHA-256/)
  await assert.rejects(readFile(destination), { code: 'ENOENT' })
})

test('MariaDB reports exhaustion of all download sources', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-download-failure-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  await assert.rejects(downloadVerifiedMariaDB(join(directory, 'installer.msi'), {
    download: { version: 'test', urls: ['https://mirror.example/installer.msi'] },
    log: () => {}, run: async () => ({ status: 22 }),
  }), /官方下载源均失败/)
})

test('MSI reboot-required success differs from failure and cancellation', () => {
  for (const status of [0, 3010, 1641]) assert.equal(installerSucceeded({ status }), true)
  for (const status of [null, 1602, 1603, 1618]) assert.equal(installerSucceeded({ status }), false)
  assert.equal(installerSucceeded({ status: 0, error: new Error('spawn failed') }), false)
})

test('download progress prints a plain complete line without cursor movement or carriage returns', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-download-progress-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const file = join(directory, 'installer.msi')
  assert.equal(downloadProgress(file, 1048576), '0.0% | 0.0 / 1.0 MiB')
  await writeFile(file, Buffer.alloc(524288))
  assert.equal(downloadProgress(file, 1048576), '50.0% | 0.5 / 1.0 MiB')
  assert.doesNotMatch(downloadProgress(file, 1048576), /[\r\n\x1b]/)
})

test('download bar fits narrow terminals and represents actual byte progress', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-download-bar-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const file = join(directory, 'installer.msi')
  await writeFile(file, Buffer.alloc(524288))
  for (const columns of [8, 20, 40, 80, 120]) {
    const line = renderDownloadBar(file, 1048576, 20000, columns)
    assert.ok(line.length < columns, `${columns} columns must not wrap`)
    assert.match(line, /50\.0%/)
  }
  assert.match(renderDownloadBar(file, 1048576, 20000, 80), /\[=+> +\] 50\.0% \| 0\.5\/1\.0 MiB \| 20s/)
  await writeFile(file, Buffer.alloc(1048576))
  assert.match(renderDownloadBar(file, 1048576, 25000, 80), /\[=+\] 100\.0%/)
})

test('interactive downloads refresh one line and print errors only after restoring the cursor', async () => {
  const events = []
  const result = await executeWithProgress(process.execPath, ['-e', 'setTimeout(() => {console.error("download failed"); process.exit(7)}, 550)'], {
    label: 'download', log: message => events.push(message),
    output: { isTTY: true, columns: 80, write: value => events.push(value) },
    progressBar: () => '[====>     ] 40.0%',
  })
  assert.equal(result.status, 7)
  assert.ok(events.filter(value => value.startsWith('\r\x1b[2K')).length >= 3)
  assert.equal(events.some(value => value.includes('[等待')), false)
  assert.equal(events.filter(value => value === '\n').length, 1)
  assert.ok(events.indexOf('download failed') > events.indexOf('\x1b[?25h'))
})

test('redirected downloads keep plain logs without ANSI animation', async () => {
  const messages = []
  const result = await executeWithProgress(process.execPath, ['-e', 'setTimeout(() => process.exit(0), 100)'], {
    intervalMs: 20, log: message => messages.push(message),
    output: { isTTY: false, write: () => assert.fail('no terminal control in redirected output') },
    progress: () => '50.0%', progressBar: () => assert.fail('no animation in redirected output'),
  })
  assert.equal(result.status, 0)
  assert.ok(messages.some(message => message.includes('50.0%')))
})

test('MariaDB recovers old verified downloads and reuses the cache on every retry', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-cache-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const legacyDirectory = join(directory, 'legacy')
  const oldRun = join(legacyDirectory, 'aster-mariadb-Ab12cd')
  await mkdir(oldRun, { recursive: true })
  const payload = Buffer.from('verified installer')
  const download = { version: 'test', bytes: payload.length, sha256: createHash('sha256').update(payload).digest('hex') }
  const oldFile = join(oldRun, 'mariadb-test-winx64.msi')
  await writeFile(oldFile, payload)
  const options = { cacheDirectory: join(directory, 'cache'), legacyDirectory, download, log: () => {}, run: async () => assert.fail('cached installer must not download again') }
  const installer = await prepareMariaDBInstaller(options)
  assert.deepEqual(await readFile(installer), payload)
  assert.deepEqual(await readFile(oldFile), payload, 'migration must not move a running installer')
  await rm(oldFile)
  assert.equal(await prepareMariaDBInstaller(options), installer)
})

test('MariaDB rejects corrupt cache and incomplete legacy files before redownloading', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-cache-invalid-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const legacyDirectory = join(directory, 'legacy')
  const oldRun = join(legacyDirectory, 'aster-mariadb-Ab12cd')
  const cacheDirectory = join(directory, 'cache')
  await mkdir(oldRun, { recursive: true })
  await mkdir(cacheDirectory)
  const fileName = 'mariadb-test-winx64.msi'
  await writeFile(join(cacheDirectory, fileName), 'bad cache')
  await writeFile(join(oldRun, fileName), 'partial')
  const payload = Buffer.from('valid replacement')
  let downloads = 0
  const installer = await prepareMariaDBInstaller({
    cacheDirectory, legacyDirectory, log: () => {},
    download: { version: 'test', bytes: payload.length, sha256: createHash('sha256').update(payload).digest('hex'), urls: ['https://example.com/installer.msi'] },
    run: async (_command, args) => { downloads += 1; await writeFile(args[args.indexOf('--output') + 1], payload); return { status: 0 } },
  })
  assert.equal(downloads, 1)
  assert.deepEqual(await readFile(installer), payload)
})

test('MSI status distinguishes UAC, an authorized launch, and UTF-16 installer actions', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-msi-status-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const log = join(directory, 'install.log')
  const marker = join(directory, 'started.txt')
  const progress = createMsiProgress(log, marker, true)
  assert.match(progress(), /等待 Windows 权限确认/)
  await writeFile(marker, '123')
  assert.match(progress(), /已获管理员权限/)
  await writeFile(log, '\ufeffProperty(S): PASSWORD = secret\r\nAction start 11:00:00: InstallFiles.\r\n', 'utf16le')
  assert.match(progress(), /MSI 安装已启动；Action start.*InstallFiles/)
  assert.doesNotMatch(progress(), /secret/)
})

test('Windows MSI launch handles spaced paths and exits without a hidden help dialog', { skip: process.platform !== 'win32' }, async t => {
  const directory = await mkdtemp(join(tmpdir(), "aster msi args user's-"))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const marker = join(directory, 'started.txt')
  // A missing MSI validates the real argument parser without installing anything or asking for UAC.
  const script = mariaDBElevatedScript(join(directory, 'missing package.msi'), join(directory, 'install.log'), marker, {
    installRoot: join(directory, 'Unified Tools'), elevate: false,
  })
  const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-EncodedCommand', Buffer.from(script, 'utf16le').toString('base64')], { encoding: 'utf8', windowsHide: true, timeout: 10000 })
  t.after(async () => {
    if (!result.error) return
    try { process.kill(Number(await readFile(marker, 'utf8'))) } catch { /* Diagnostic child already exited. */ }
  })
  assert.equal(result.error, undefined)
  assert.equal(result.status, 1619, result.stderr)
  assert.match(await readFile(marker, 'utf8'), /^\d+$/)
})

test('MSI log streaming handles split UTF-16 writes without duplicates and flushes the final result', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-msi-stream-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const file = join(directory, 'install.log')
  const readLines = createMsiLogReader(file)
  assert.deepEqual(readLines(), [])
  const first = 'Action start 11:00:00: InstallFiles.'
  const last = 'MSI (s): Product: MariaDB -- 安装成功。'
  const bytes = Buffer.from(`\ufeff${first}\r\nProperty(S): PASSWORD = secret\r\n${last}`, 'utf16le')
  await writeFile(file, bytes.subarray(0, bytes.length - 5))
  assert.deepEqual(readLines(), [first])
  assert.deepEqual(readLines(), [])
  await appendFile(file, bytes.subarray(bytes.length - 5))
  assert.deepEqual(readLines(), [])
  assert.deepEqual(readLines(true), [last])
  assert.deepEqual(readLines(true), [])
})

test('MSI UTF-8 errors reach the console while credential properties stay in the log file', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-msi-errors-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  const file = join(directory, 'install.log')
  await writeFile(file, 'Error 1920. Service failed to start.\nMSI: PROPERTY CHANGE: PASSWORD = secret\nMSI: MainEngineThread is returning 1603\n')
  const lines = createMsiLogReader(file)()
  assert.deepEqual(lines, ['Error 1920. Service failed to start.', 'MSI: MainEngineThread is returning 1603'])
})

test('installer streams new lines while running and drains final lines before the end message', async () => {
  const messages = []
  let sent = false
  const result = await executeWithProgress(process.execPath, ['-e', 'setTimeout(() => process.exit(0), 650)'], {
    label: 'MSI test', log: value => messages.push(value),
    readLogLines: flush => {
      if (flush) return ['Product: MariaDB -- Installation completed successfully.']
      if (sent) return []
      sent = true
      return ['Action start: InstallFiles.']
    },
  })
  assert.equal(result.status, 0)
  assert.equal(messages.filter(value => value.includes('Action start: InstallFiles.')).length, 1)
  const completion = messages.findIndex(value => value.includes('Installation completed successfully.'))
  assert.ok(completion > 0 && completion < messages.findIndex(value => value.startsWith('[结束]')))
})

test('shared root prefers a fixed non-C repository drive and falls back only when needed', () => {
  const drives = [{ DeviceID: 'C:', FreeSpace: 900 }, { DeviceID: 'D:', FreeSpace: 800 }, { DeviceID: 'E:', FreeSpace: 700 }]
  assert.equal(selectInstallRoot(drives, 'E:\\repo'), 'E:\\AsterDev')
  assert.equal(selectInstallRoot(drives, 'C:\\repo'), 'D:\\AsterDev')
  assert.equal(selectInstallRoot(drives.slice(0, 2), 'C:\\repo', 'D:'), 'D:\\AsterDev')
  assert.equal(selectInstallRoot([{ DeviceID: 'D:', FreeSpace: 1, DriveType: 2 }, drives[0]], 'C:\\repo'), 'C:\\AsterDev')
  assert.throws(() => selectInstallRoot([], 'C:\\repo'), /固定磁盘/)
})

test('setup validates explicit roots and never accepts a password CLI argument', () => {
  assert.equal(validateInstallRoot('D:/Aster Tools/'), 'D:\\Aster Tools')
  assert.equal(parseSetupArguments(['--yes', '--install-root', 'D:\\Aster Tools']).installRoot, 'D:\\Aster Tools')
  for (const path of ['D:\\', 'relative', 'D:\\a"b', 'D:\\CON', '\\\\server\\share']) assert.throws(() => validateInstallRoot(path))
  assert.throws(() => parseSetupArguments(['--password', 'secret']), /未知参数/)
})

test('all native installer locations use the selected root while existing Rust environment is preserved', () => {
  const paths = windowsInstallPaths('D:\\Aster Tools')
  assert.match(nativePackageOverride({ key: 'bash' }, paths), /\/DIR="D:\\Aster Tools\\Git"/)
  assert.equal(nativePackageOverride({ key: 'gh' }, paths), '/qn /norestart INSTALLDIR="D:\\Aster Tools\\GitHubCLI"')
  assert.match(nativePackageOverride({ key: 'python' }, paths), /TargetDir="D:\\Aster Tools\\Python"/)
  assert.match(nativePackageOverride({ key: 'msvc', override: '--quiet --wait' }, paths), /--installPath "D:\\Aster Tools\\BuildTools"/)
  const existing = setupEnvironment({ installRoot: paths.root }, { Path: 'C:\\existing', CARGO_HOME: 'C:\\existing-cargo' })
  assert.equal(existing.CARGO_HOME, 'C:\\existing-cargo')
  const fresh = setupEnvironment({ installRoot: paths.root, managedRust: true, managedGo: true }, { Path: 'C:\\Windows' })
  assert.equal(fresh.CARGO_HOME, 'D:\\Aster Tools\\Rust\\cargo')
  assert.equal(fresh.RUSTUP_HOME, 'D:\\Aster Tools\\Rust\\rustup')
  assert.equal(fresh.ASTER_TOOLS_ROOT, 'D:\\Aster Tools')
  assert.equal(fresh.ASTER_GO_ROOT, undefined)
  assert.ok(fresh.Path.split(';').includes(paths.gh))
  assert.doesNotMatch(fresh.Path, /Git\\usr\\bin/)
  const script = mariaDBElevatedScript('D:\\package.msi', 'D:\\install.log', 'D:\\started.txt', { installRoot: paths.root, elevate: false })
  assert.match(script, /INSTALLDIR="D:\\Aster Tools\\MariaDB" DATADIR="D:\\Aster Tools\\MariaDB\\data"/)
  assert.doesNotMatch(script, /PASSWORD=|Verb RunAs/)
})

test('Go tooling discovers the recorded shared Windows installation root', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-go-root-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  await mkdir(join(directory, '.aster-tools'), { recursive: true })
  await writeFile(join(directory, '.aster-tools', 'windows-setup.json'), JSON.stringify({ installRoot: 'D:\\Aster Tools', managedGo: false }))
  assert.equal(configuredWindowsToolsRoot(directory, 'win32'), 'D:\\Aster Tools')
  assert.equal(preferredGoRoot(directory, { platform: 'win32', environment: {} }), 'D:\\Aster Tools\\Go')
  assert.equal(configuredWindowsToolsRoot(directory, 'darwin'), null)
})

test('setup settings persist directories and recovery state but never password answers', async t => {
  const directory = await mkdtemp(join(tmpdir(), 'aster-settings-test-'))
  t.after(() => rm(directory, { recursive: true, force: true }))
  writeSetupSettings(directory, { installRoot: 'D:\\AsterDev', managedRust: true, pendingDatabasePassword: true, password: 'DoNotStoreThis!' })
  const saved = await readFile(join(directory, '.aster-tools', 'windows-setup.json'), 'utf8')
  assert.doesNotMatch(saved, /DoNotStoreThis/)
  assert.equal(readSetupSettings(directory).pendingDatabasePassword, true)
  assert.equal(readSetupSettings(directory).managedRust, true)
  assert.equal(Object.hasOwn(JSON.parse(saved), 'password'), false)
})

test('password prompts support user choice, confirmation, random defaults, and unattended setup', async () => {
  const password = 'User-chosen-Pass!'
  let questions = 0
  const chosen = await collectNativeChoices({ defaultRoot: 'D:\\AsterDev', needsDatabasePassword: true,
    ask: async () => 'E:\\Tools', secret: async () => { questions += 1; return password } })
  assert.equal(chosen.installRoot, 'E:\\Tools')
  assert.equal(chosen.password, password)
  assert.equal(questions, 2)
  const automatic = await collectNativeChoices({ defaultRoot: 'D:\\AsterDev', needsDatabasePassword: true, assumeYes: true,
    ask: () => assert.fail('unattended setup cannot prompt'), secret: () => assert.fail('unattended setup cannot prompt') })
  assert.ok(automatic.password.length >= 24)
  assert.notEqual(automatic.password, resolveRootPassword())
  const existing = await collectNativeChoices({ defaultRoot: 'D:\\AsterDev', needsDatabasePassword: false, assumeYes: true })
  assert.equal(existing.password, undefined)
  let first = true
  await assert.rejects(collectNativeChoices({ defaultRoot: 'D:\\AsterDev', needsDatabasePassword: true, ask: async () => '',
    secret: async () => { const answer = first ? password : 'different-password'; first = false; return answer } }), /两次密码不一致/)
  assert.throws(() => resolveRootPassword('short'), /12–128/)
})

test('password input is hidden and password SQL contains only the native verifier', async () => {
  const input = new PassThrough()
  const output = new PassThrough()
  let display = ''
  output.on('data', data => { display += data.toString() })
  const answer = askSecret('Password: ', { input, output })
  input.write('Hidden-Pass-123!\r')
  assert.equal(await answer, 'Hidden-Pass-123!')
  assert.doesNotMatch(display, /Hidden-Pass/)
  input.destroy(); output.destroy()
  assert.equal(rootPasswordSql('mariadb'), "ALTER USER 'root'@'localhost' IDENTIFIED BY PASSWORD '*54958E764CE10E50764C2EECBB71D01F08549980';\n")
})

test('root initialization checks the instance and sends the password verifier over stdin only', () => {
  const calls = []
  const messages = []
  const password = 'My-secret-中文$!'
  initializeDatabaseRootPassword('client.exe', password, 'D:\\AsterDev\\MariaDB\\data', {
    run: (command, args, options) => { calls.push({ command, args, options }); return { status: 0, stdout: 'D:/AsterDev/MariaDB/data/\n' } },
    log: message => messages.push(message),
  })
  assert.equal(calls.length, 2)
  assert.doesNotMatch(JSON.stringify(calls), /My-secret/)
  assert.match(calls[1].options.input, /IDENTIFIED BY PASSWORD '\*[A-F0-9]{40}'/)
  assert.equal(messages.filter(message => message.includes(password)).length, 1)
  let attempts = 0
  assert.throws(() => initializeDatabaseRootPassword('client.exe', password, 'D:\\expected', {
    run: () => { attempts += 1; return { status: 0, stdout: 'C:\\another-database' } }, log: () => assert.fail('must not print an unapplied password'),
  }), /未修改现有账号/)
  assert.equal(attempts, 1)
})

function githubFixture({ remote = 'https://github.com/owner/repo.git', authenticated = true, source = 'keyring', access = true, gitAccess = true } = {}) {
  const calls = []
  const state = { authenticated, source, access, gitAccess }
  return {
    calls, state,
    options: {
      gh: 'gh.exe', git: 'git.exe', cwd: process.cwd(), environment: {}, log: () => {},
      login: () => assert.fail('unexpected browser login'), ask: () => assert.fail('unexpected question'),
      run: (command, args, options) => {
        calls.push({ command, args, options })
        if (args[0] === 'remote') return { status: 0, stdout: remote }
        if (args[0] === 'config') return { status: 1 }
        if (args[0] === 'auth' && args[1] === 'status') return { status: 0, stdout: JSON.stringify({ hosts: {
          'github.com': [{ active: true, state: state.authenticated ? 'success' : 'error', login: 'developer', tokenSource: state.source, token: 'secret-must-never-be-logged' }],
        } }) }
        if (args[0] === 'api') return { status: state.access ? 0 : 403, stderr: 'secret-must-never-be-logged' }
        if (args[0] === 'ls-remote') return { status: state.gitAccess ? 0 : 128 }
        if (args[0] === 'auth' && args[1] === 'setup-git') { state.gitAccess = true; return { status: 0 } }
        assert.fail(`unexpected command: ${command} ${args.join(' ')}`)
      },
    },
  }
}

test('GitHub origin parsing supports HTTPS and existing SSH without accepting embedded credentials', () => {
  assert.deepEqual(githubRepository('https://github.com/owner/repo.git'), { host: 'github.com', owner: 'owner', repository: 'repo', protocol: 'https' })
  assert.deepEqual(githubRepository('git@github.com:owner/repo.git'), { host: 'github.com', owner: 'owner', repository: 'repo', protocol: 'ssh' })
  assert.equal(githubRepository('ssh://git@git.example.com/owner/repo').host, 'git.example.com')
  for (const remote of ['https://user:secret@github.com/owner/repo', 'https://github.com/owner/repo?token=secret', 'http://github.com/owner/repo', 'file:///repo']) {
    assert.throws(() => githubRepository(remote))
  }
})

test('GitHub JSON exit zero is not success when the account is invalid or storage is plaintext', () => {
  const fixture = githubFixture({ authenticated: false })
  assert.equal(readGitHubAuth('gh.exe', 'github.com', fixture.options).ok, false)
  fixture.state.authenticated = true
  fixture.state.source = 'hosts.yml'
  assert.equal(readGitHubAuth('gh.exe', 'github.com', fixture.options).storageError, true)
  fixture.state.source = 'GH_TOKEN'
  assert.deepEqual(readGitHubAuth('gh.exe', 'github.com', fixture.options), { ok: true, login: 'developer', source: 'GH_TOKEN' })
  assert.equal(readGitHubAuth('gh.exe', 'github.com', { run: () => ({ status: 0, stdout: 'bad JSON secret' }) }).ok, false)
})

test('valid GitHub and Git authentication is reused without login or configuration changes', async () => {
  const fixture = githubFixture()
  const messages = []
  const account = await prepareGitHubAuthentication({ ...fixture.options, assumeYes: true, log: line => messages.push(line) })
  assert.equal(account.login, 'developer')
  assert.deepEqual(fixture.calls.map(call => call.args[0]), ['remote', 'auth', 'api', 'ls-remote'])
  assert.doesNotMatch(messages.join('\n'), /secret-must/)
  for (const call of fixture.calls) {
    assert.equal(call.options.env.GIT_TERMINAL_PROMPT, '0')
    assert.equal(call.options.env.GCM_INTERACTIVE, 'Never')
    assert.ok(call.options.timeout <= 30000)
    assert.ok(!call.args.includes('--show-token'))
  }
})

test('first GitHub login happens in the foreground and is verified before returning', async () => {
  const fixture = githubFixture({ authenticated: false })
  let loggedIn = false
  await prepareGitHubAuthentication({ ...fixture.options, login: (_command, args, options) => {
    assert.deepEqual(args, ['auth', 'login', '--hostname', 'github.com', '--web', '--git-protocol', 'https', '--skip-ssh-key'])
    assert.equal(options.stdio, 'inherit')
    assert.equal(options.env.GH_PROMPT_DISABLED, undefined)
    assert.equal(options.env.GH_DEBUG, '')
    fixture.state.authenticated = true
    loggedIn = true
    return { status: 0 }
  } })
  assert.equal(loggedIn, true)
  assert.equal(fixture.calls.filter(call => call.args[1] === 'status').length, 2)
})

test('unattended setup and invalid environment tokens never open a browser', async () => {
  const fixture = githubFixture({ authenticated: false })
  await assert.rejects(prepareGitHubAuthentication({ ...fixture.options, assumeYes: true }), /--yes 要求已有可用认证/)
  assert.equal(fixture.calls.some(call => call.args[0] === 'api'), false)
  await assert.rejects(prepareGitHubAuthentication({ ...fixture.options, environment: { GH_TOKEN: 'do-not-print' } }), /环境中的 GitHub token/)
})

test('HTTPS Git authentication is repaired only when its remote probe fails', async () => {
  const fixture = githubFixture({ gitAccess: false })
  await prepareGitHubAuthentication(fixture.options)
  assert.equal(fixture.calls.filter(call => call.args[0] === 'ls-remote').length, 2)
  assert.deepEqual(fixture.calls.find(call => call.args[1] === 'setup-git').args, ['auth', 'setup-git', '--hostname', 'github.com'])
})

test('SSH and repository permission failures are resolved before unattended installation', async () => {
  const ssh = githubFixture({ remote: 'git@github.com:owner/repo.git', gitAccess: false })
  await assert.rejects(prepareGitHubAuthentication({ ...ssh.options, assumeYes: true }), /SSH 访问失败/)
  assert.equal(ssh.calls.some(call => call.args[1] === 'setup-git'), false)
  assert.equal(ssh.calls.some(call => call.args[0] === 'ls-remote' && /BatchMode=yes/.test(call.options.env.GIT_SSH_COMMAND)), true)
  const denied = githubFixture({ access: false })
  let asked = false
  await assert.rejects(prepareGitHubAuthentication({ ...denied.options, ask: async () => { asked = true; return 'q' } }), /已退出/)
  assert.equal(asked, true)
  assert.equal(denied.calls.some(call => call.args[0] === 'ls-remote'), false)
})

test('authentication retry rechecks repaired permissions without collecting credentials', async () => {
  const fixture = githubFixture({ access: false })
  await prepareGitHubAuthentication({ ...fixture.options, ask: async () => { fixture.state.access = true; return '' } })
  assert.equal(fixture.calls.filter(call => call.args[0] === 'api').length, 2)
  const env = githubEnvironment({ GH_DEBUG: 'api', DEBUG: '1', GIT_TRACE: '1', GIT_SSH_COMMAND: 'custom-ssh', GH_TOKEN: 'not-printed' })
  assert.equal(env.GH_DEBUG, '')
  assert.equal(env.GIT_TRACE, '0')
  assert.equal(env.GH_TOKEN, 'not-printed')
  assert.match(env.GIT_SSH_COMMAND, /^custom-ssh .*BatchMode=yes/)
})

test('native setup installs only Git and gh until authentication succeeds', async () => {
  const packages = ['msvc', 'bash', 'mariadb', 'gh'].map(key => ({ key }))
  const events = []
  const options = { packages, install: async entry => events.push(entry.key), log: () => {},
    authenticate: async () => { events.push('auth'); throw new Error('auth blocked') },
    automatic: async remaining => events.push(...remaining.map(entry => entry.key)) }
  await assert.rejects(runNativeSetupPhases(options), /auth blocked/)
  assert.deepEqual(events, ['bash', 'gh', 'auth'])
  events.length = 0
  await runNativeSetupPhases({ ...options, authenticate: async () => events.push('auth') })
  assert.deepEqual(events, ['bash', 'gh', 'auth', 'msvc', 'mariadb'])
})

test('a second setup cannot acquire the same OS lock and closing releases it', async t => {
  const endpoint = process.platform === 'win32' ? `\\\\.\\pipe\\aster-setup-test-${process.pid}` : join(await mkdtemp(join(tmpdir(), 'aster-lock-')), 'socket')
  const first = await acquireSetupLock(endpoint)
  try { await assert.rejects(acquireSetupLock(endpoint), /已有 setup/) } finally { await first.release() }
  const next = await acquireSetupLock(endpoint)
  await next.release()
  if (process.platform !== 'win32') t.after(() => rm(join(endpoint, '..'), { recursive: true, force: true }))
})

test('setup recognizes MSI and signed or unsigned winget restart results', () => {
  for (const status of [3010, 1641, 0x8a150109, 0x8a15010a, 0x8a15010b, -1978334967, -1978334966, -1978334965]) {
    assert.equal(installerNeedsRestart({ status }), true)
  }
  for (const status of [0, null, 1603, 1618]) assert.equal(installerNeedsRestart({ status }), false)
  assert.equal(installerNeedsRestart({ status: 3010, error: new Error('spawn failed') }), false)
})

test('existing repository SSH commands are retained and checked without interactive prompts', async () => {
  const fixture = githubFixture({ remote: 'git@github.com:owner/repo.git' })
  const result = await prepareGitHubAuthentication({ ...fixture.options, run: (command, args, options) => {
    if (args[0] === 'config') return { status: 0, stdout: 'ssh -i existing-key' }
    return fixture.options.run(command, args, options)
  } })
  assert.match(result.gitSshCommand, /^ssh -i existing-key .*BatchMode=yes/)
  assert.equal(githubEnvironment({ GIT_SSH_COMMAND: result.gitSshCommand }).GIT_SSH_COMMAND, result.gitSshCommand)
  assert.equal(githubEnvironment({ GIT_SSH_COMMAND: 'plink.exe' }).GIT_SSH_COMMAND, 'plink.exe -batch')
})
