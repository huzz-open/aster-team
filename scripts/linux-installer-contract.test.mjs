import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

const read = path => readFile(new URL(path, import.meta.url), 'utf8')
const windowsBackup = await read('../customer/backend/cli/src/windows_backup.rs')
const asterctlBuildScript = await read('../customer/backend/asterctl/build.rs')
const slotRunnerUnit = await read('../customer/deploy/systemd/aster-runner@.service')
const packageScripts = JSON.parse(await read('../package.json')).scripts

const [
  layoutContractSource, layoutSource, upgradeCore, upgradeExecutor, storage, mariadbStorage,
  installer, restore, serviceHealth, init, cli, controlUnit, runnerUnit,
  caddyUnit, upgradePath, upgradeService, controlSource, controlEntrypoints, maintenanceView,
  adminRouter, sdk, runnerSource, runnerAdmin, localRunnerLauncher, builder, bundledFreeLicense,
  releaseArchive, boundaries, verifyWorkflow, customerReleaseWorkflow,
  customerInstallSmoke, runnerInstallSmoke, supportedInstallSmoke,
  diagnosticCollector, linuxLab, linuxLabDockerfile, smokeMatrixEmitter,
  aptSystemdImage, dnfSystemdImage, staticRuntimeVerifier, releasePlatforms,
  cargoConfig, linuxBuildRuntime,
  windowsBuildRuntime, windowsBuildRuntimeSetup,
  windowsInstaller, windowsLauncher, windowsRestore, windowsBuilder,
  workspaceReleaseFixture, linuxUpgradeFixtureBuilder,
  windowsUpgradeFixtureBuilder, windowsUpgradeSmoke, windowsReleaseSmoke, windowsRunnerReleaseSmoke,
  macosInstaller, macosLauncher, macosRestore, macosBuilder,
  localRelease, localLinuxRelease, localControlLauncher, standaloneAsterctlBuilder,
  operationsInstaller, operationsApiUnit, operationsWebUnit, operationsEnvExample,
  releaseSecurityPreflight,
] = await Promise.all([
  read('../contracts/install-layout.json'),
  read('../customer/backend/crates/install-layout/src/lib.rs'),
  read('../customer/backend/crates/upgrade-core/src/lib.rs'),
  read('../customer/backend/cli/src/maintenance_executor.rs'),
  read('../customer/backend/crates/storage/src/lib.rs'),
  read('../customer/backend/crates/storage/src/mariadb.rs'),
  read('../customer/deploy/install.sh'),
  read('../customer/deploy/restore-backup.sh'),
  read('../customer/deploy/service-health.sh'),
  read('../customer/deploy/init.sh'),
  read('../customer/backend/cli/src/main.rs'),
  read('../customer/deploy/systemd/aster-control@.service'),
  read('../customer/deploy/systemd/aster-runner.service'),
  read('../customer/deploy/systemd/aster-caddy.service'),
  read('../customer/deploy/systemd/aster-upgrade.path'),
  read('../customer/deploy/systemd/aster-upgrade.service'),
  read('../customer/backend/control/src/lib.rs'),
  read('../customer/backend/control/src/entrypoints.rs'),
  read('../customer/admin/src/views/MaintenanceView.vue'),
  read('../customer/admin/src/router.ts'),
  read('../customer/sdk/src/index.ts'),
  read('../customer/backend/runner/src/main.rs'),
  read('../customer/admin/src/views/RunnerView.vue'),
  read('./run-rust-runner.mjs'),
  read('./build-linux-bundle.mjs'),
  read('./bundled-free-license.mjs'),
  read('./release-archive.mjs'),
  read('../tools/release-boundaries.json'),
  read('../.github/workflows/verify.yml'),
  read('../.github/workflows/customer-release.yml'),
  read('./ci/exercise-customer-install.sh'),
  read('./ci/exercise-runner-install.sh'),
  read('./ci/exercise-supported-linux-install.sh'),
  read('./ci/collect-linux-diagnostics.sh'),
  read('./ci/linux-lab.sh'),
  read('./ci/linux-lab.Dockerfile'),
  read('./ci/release-smoke-matrix.mjs'),
  read('./ci/systemd/apt.Dockerfile'),
  read('./ci/systemd/dnf.Dockerfile'),
  read('./ci/verify-linux-static-runtime.sh'),
  read('../contracts/release-platforms.json'),
  read('../.cargo/config.toml'),
  read('../tools/linux-build-runtime.json'),
  read('../tools/windows-build-runtime.json'),
  read('./ci/setup-windows-build-runtime.ps1'),
  read('../customer/deploy/windows/install.ps1'),
  read('../customer/deploy/windows/service-launch.ps1'),
  read('../customer/deploy/windows/restore-backup.ps1'),
  read('./build-windows-bundle.mjs'),
  read('./ci/workspace-release-fixture.mjs'),
  read('./ci/build-linux-upgrade-fixture.mjs'),
  read('./ci/build-windows-upgrade-fixture.mjs'),
  read('./ci/exercise-windows-upgrade.ps1'),
  read('./ci/exercise-windows-release.ps1'),
  read('./ci/exercise-windows-runner-install.ps1'),
  read('../customer/deploy/macos/install-macos.sh'),
  read('../customer/deploy/macos/service-launch.sh'),
  read('../customer/deploy/macos/restore-backup-macos.sh'),
  read('./build-macos-bundle.mjs'),
  read('./release-local.mjs'),
  read('./ci/build-production-linux-in-docker.sh'),
  read('./run-rust-control.mjs'),
  read('./build-asterctl-windows.mjs'),
  read('../operations/deploy/install.sh'),
  read('../operations/deploy/systemd/aster-operations-api.service'),
  read('../operations/deploy/systemd/aster-operations-web.service'),
  read('../operations/deploy/operations.env.example'),
  read('./run-release-security-preflight.mjs'),
])

const layoutContract = JSON.parse(layoutContractSource)
const releasePlatformContract = JSON.parse(releasePlatforms)

test('one versioned layout contract owns every product path on all target operating systems', () => {
  assert.equal(layoutContract.schema, 'aster.install-layout.v1')
  assert.deepEqual(Object.keys(layoutContract.default_roots).sort(), ['linux', 'macos', 'windows'])
  for (const path of Object.values(layoutContract.paths)) {
    assert.doesNotMatch(path, /^(?:[A-Za-z]:|[/\\])/)
    assert.doesNotMatch(path, /(?:^|[/\\])\.\.(?:[/\\]|$)/)
  }
  assert.equal(layoutContract.paths.database_file, 'data/database/aster-team.db')
  assert.equal(layoutContract.paths.maintenance_lock, 'staging/maintenance.lock')
  assert.match(layoutSource, /pub fn discover_or_default/)
  assert.match(layoutSource, /INSTALL_MARKER_SCHEMA/)
  assert.match(layoutSource, /service_registration_root/)
  assert.deepEqual(Object.keys(layoutContract.release_paths.install_engine).sort(), ['linux', 'macos', 'windows'])
  assert.deepEqual(Object.keys(layoutContract.release_paths.restore_engine).sort(), ['linux', 'macos', 'windows'])
})

test('Windows and macOS lifecycle adapters keep product-owned state below the configured root', () => {
  const linuxRuntime = JSON.parse(linuxBuildRuntime)
  const windowsRuntime = JSON.parse(windowsBuildRuntime)
  assert.equal(linuxRuntime.schema, 'aster.linux-build-runtime.v1')
  for (const image of [linuxRuntime.node_image, linuxRuntime.go_image, linuxRuntime.rust_image]) {
    assert.match(image, /@sha256:[0-9a-f]{64}$/)
  }
  assert.equal(windowsRuntime.schema, 'aster.windows-build-runtime.v1')
  assert.match(windowsRuntime.strawberry_perl.sha256, /^[0-9a-f]{64}$/)
  assert.match(windowsBuildRuntimeSetup, /Locale::Maketext::Simple/)
  assert.match(windowsBuildRuntimeSetup, /Security\.Cryptography\.SHA256/)
  assert.doesNotMatch(windowsBuildRuntimeSetup, /Get-FileHash/)
  assert.match(customerReleaseWorkflow, /setup-windows-build-runtime\.ps1/)
  assert.match(windowsInstaller, /--install-root/)
  assert.match(windowsInstaller, /--release-root/)
  assert.doesNotMatch(windowsInstaller, /Join-Path \$PSScriptRoot/)
  assert.match(windowsInstaller, /ConvertFrom-WindowsVerbatimPath/)
  assert.match(windowsInstaller, /StartsWith\('\\\\\?\\UNC\\'/)
  assert.match(windowsInstaller, /StartsWith\('\\\\\?\\'/)
  assert.match(windowsInstaller, /System\.Security\.Cryptography\.SHA256/)
  assert.doesNotMatch(windowsInstaller, /Get-FileHash/)
  assert.match(cli, /\.arg\("--release-root"\)[\s\S]*\.arg\(&selected\.release_root\)/)
  assert.match(cli, /install_layout\(\)\s*\.releases\(\)\s*\.canonicalize\(\)/)
  assert.match(windowsInstaller, /Join-Path \$installRoot 'releases'/)
  assert.match(windowsLauncher, /installation marker/i)
  assert.match(windowsLauncher, /current['"]?\).*aster-team-cli\.exe|current.*bin\\aster-team-cli\.exe/s)
  assert.match(windowsRestore, /prepare-windows-restore/)
  assert.match(windowsBackup, /verify_marker_bytes/)
  assert.match(windowsBackup, /verify_release_at/)
  assert.match(windowsRestore, /Register-ScheduledTask/)
  assert.match(windowsRestore, /New-ScheduledTaskAction/)
  assert.doesNotMatch(windowsRestore, /schtasks\.exe[^\r\n]*\/Create/i)
  assert.match(windowsRestore, /Start-DeferredWorkspaceCleanup/)
  assert.match(windowsRestore, /staging\\restores/)
  assert.match(windowsInstaller, /The existing installation is not a dedicated Runner/)
  assert.match(windowsInstaller, /Register-ScheduledTask/)
  assert.match(windowsInstaller, /New-ScheduledTaskAction/)
  assert.doesNotMatch(windowsInstaller, /schtasks\.exe[^\r\n]*\/Create/i)
  assert.doesNotMatch(windowsUpgradeSmoke, /schtasks\.exe/)
  assert.match(windowsInstaller, /\$options\.CertificateSource = 'none'/)
  assert.doesNotMatch(windowsBuilder, /run\(cargo, \['build', '--release', '--locked', '-p', 'asterctl'/)
  assert.match(windowsBuilder, /ASTER_CLIENT_ASTERCTL_WINDOWS_X64/)
  assert.match(windowsBuilder, /copyFileSync\(asterctlWindowsX64, resolve\(bundle, 'client-tools\/asterctl\/windows-x86_64\/asterctl\.exe'\)\)/)
  assert.match(windowsBuilder, /client-tools\/asterctl\/windows-x86_64\/asterctl\.exe/)
  assert.match(windowsBuilder, /target-feature=\+crt-static/)
  assert.match(windowsBuilder, /verifyStaticCRT/)
  assert.match(standaloneAsterctlBuilder, /target-feature=\+crt-static/)
  assert.match(standaloneAsterctlBuilder, /verifyStaticCRT/)
  assert.match(asterctlBuildScript, /if !path\.is_file\(\)/)
  assert.match(asterctlBuildScript, /register_git_path\(manifest_directory, "packed-refs"\)/)
  assert.match(localControlLauncher, /build-asterctl-windows\.mjs/)
  assert.match(localControlLauncher, /target\/client-tools\/asterctl-windows-x86_64\.exe/)
  assert.doesNotMatch(localControlLauncher, /target\/debug\/asterctl\.exe/)
  assert.match(localControlLauncher, /ASTER_LOCAL_DEV_MANAGER/)
  assert.match(localControlLauncher, /cargoPackageArguments/)
  assert.match(localControlLauncher, /spawnSync\('cargo', \['build', \.\.\.cargoPackageArguments\]/)
  assert.match(localControlLauncher, /@@ASTER_CUSTOMER_CONTROL_BUILD_READY@@/)
  const controlBuild = localControlLauncher.indexOf("spawnSync('cargo', ['build'")
  const controlReady = localControlLauncher.indexOf('console.log(controlBuildReadyMarker)')
  assert.match(localControlLauncher, /const controlBinary = resolve\(cargoTargetDirectory, 'debug'/)
  assert.match(localControlLauncher, /result = spawnSync\(controlBinary, command/)
  const controlRun = localControlLauncher.indexOf('result = spawnSync(controlBinary, command')
  assert.ok(controlBuild < controlReady && controlReady < controlRun)
  assert.match(windowsBuilder, /\['aster-control\.exe', 'aster-runner\.exe', 'aster-team-cli\.exe'\]/)
  assert.match(customerReleaseWorkflow, /npm run test:windows-release --/)
  assert.equal(packageScripts['test:windows-release'], 'pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ./scripts/ci/exercise-windows-release.ps1')
  assert.match(customerReleaseWorkflow, /npm run test:windows-runner-install --/)
  assert.equal(packageScripts['test:windows-runner-install'], 'pwsh -NoProfile -NonInteractive -ExecutionPolicy Bypass -File ./scripts/ci/exercise-windows-runner-install.ps1')
  assert.match(windowsReleaseSmoke, /current\\client-tools\\asterctl\\windows-x86_64\\asterctl\.exe/)
  assert.match(windowsReleaseSmoke, /Installed asterctl smoke test failed/)
  assert.match(windowsRunnerReleaseSmoke, /runner install/)
  assert.match(windowsRunnerReleaseSmoke, /Unconfigured Windows Runner task must remain disabled/)
  assert.match(localRelease, /npmCLI, 'run', 'test:windows-release', '--'/)
  assert.match(localRelease, /npmCLI, 'run', 'test:windows-runner-install', '--'/)
  assert.match(localRelease, /npm[\s\S]*run[\s\S]*verify/)
  assert.match(localRelease, /publishStagedArtifacts/)
  assert.match(localRelease, /--build-id/)
  assert.match(localRelease, /--platform windows\|linux\|all/)
  assert.match(localRelease, /build-production-linux-in-docker\.sh/)
  assert.match(localRelease, /requireWindowsSystemPerl/)
  assert.match(localRelease, /local releases do not download or install Perl/)
  assert.doesNotMatch(localRelease, /setup-windows-build-runtime\.ps1/)
  assert.match(customerReleaseWorkflow, /windows-install:[\s\S]*Swatinem\/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6/)
  assert.match(customerReleaseWorkflow, /windows-install:[\s\S]*actions\/cache\/restore@0057852bfaa89a56745cba8c7296529d2fc39830/)
  assert.match(customerReleaseWorkflow, /aster-windows-build-runtime-[^\r\n]*hashFiles\('tools\/windows-build-runtime\.json'\)/)
  assert.match(customerReleaseWorkflow, /aster-release-downloads-[^\r\n]*hashFiles\('tools\/caddy-runtime\.json'\)/)
  assert.match(localLinuxRelease, /bash scripts\/build-linux-amd64\.sh/)
  assert.doesNotMatch(localLinuxRelease, /build-linux-bundle\.mjs/)
  assert.match(localLinuxRelease, /\/release-security:ro/)
  assert.match(localLinuxRelease, /\/client-tools\/asterctl-windows-x86_64\.exe:ro/)
  assert.match(builder, /client-tools\/asterctl\/windows-x86_64\/asterctl\.exe/)
  assert.match(customerReleaseWorkflow, /release-asterctl-windows-x64/)
  assert.match(customerReleaseWorkflow, /windows-install:[\s\S]*needs: \[preflight, asterctl-windows\]/)
  assert.match(customerReleaseWorkflow, /windows-install:[\s\S]*Download shared Windows client tool/)
  assert.match(customerReleaseWorkflow, /release-gate:[\s\S]*needs:[\s\S]*windows-install/)
  assert.match(customerReleaseWorkflow, /windows-runner-install:[\s\S]*needs: \[preflight, windows-install\]/)
  assert.match(customerReleaseWorkflow, /release-gate:[\s\S]*windows-runner-install/)
  assert.match(localRelease, /ASTER_CLIENT_ASTERCTL_WINDOWS_X64: asterctlWindowsX64/)
  assert.match(localLinuxRelease, /ci-images\.mjs/)
  assert.doesNotMatch(localLinuxRelease, /docker_cli build/)
  assert.match(localLinuxRelease, /--preflight-only/)
  assert.match(localLinuxRelease, /ASTER_RELEASE_OUTPUT_ROOT/)
  assert.match(localLinuxRelease, /Darwin\) ;;/)
  assert.match(localLinuxRelease, /--platform linux\/amd64/)
  assert.match(localLinuxRelease, /free-license\.json/)
  assert.match(localLinuxRelease, /--free-license \/release-security\/free-license\.json/)
  assert.match(localRelease, /ASTER_CUSTOMER_FREE_LICENSE_FILE: security\.freeLicenseFile/)
  assert.match(customerReleaseWorkflow, /free_license_base64:/)
  assert.match(customerReleaseWorkflow, /ASTER_CUSTOMER_FREE_LICENSE_BASE64/)
  assert.match(customerReleaseWorkflow, /--free-license "\$RUNNER_TEMP\/aster-free-license\.json"/)
  assert.match(customerReleaseWorkflow, /ASTER_CUSTOMER_FREE_LICENSE_FILE:/)
  assert.match(localLinuxRelease, /ASTER_RELEASE_DOWNLOAD_CACHE/)
  assert.match(localLinuxRelease, /docker_output_directory="\$\(docker_host_path "\$output_directory"\)"/)
  assert.match(localLinuxRelease, /--volume "\$target_volume:\/workspace\/target:rw"[\s\S]*--volume "\$docker_output_directory:\/workspace\/\$output_root:rw"/)
  assert.match(localLinuxRelease, /repository_identity/)
  assert.doesNotMatch(localLinuxRelease, /Program Files\/Docker/)
  assert.match(localLinuxRelease, /windows_docker_host[\s\S]*GIT_CONFIG_KEY_1=core\.filemode[\s\S]*GIT_CONFIG_VALUE_1=false/)
  assert.match(localLinuxRelease, /windows_docker_host[\s\S]*GIT_CONFIG_KEY_2=core\.autocrlf[\s\S]*GIT_CONFIG_VALUE_2=true/)
  assert.doesNotMatch(localLinuxRelease, /--allow-non-main/)
  assert.match(windowsLauncher, /LinkType[\s\S]*Junction[\s\S]*SymbolicLink/)
  assert.match(windowsLauncher, /referenceItem\.Target/)
  assert.doesNotMatch(windowsLauncher, /GetRelativePath/)
  assert.match(windowsBuilder, /'--platform', 'windows'/)
  assert.match(windowsBuilder, /'--runtime', 'msvc'/)
  assert.match(windowsBuilder, /'aster-storage'[\s\S]*'sqlcipher'[\s\S]*'initializes_an_encrypted_final_schema'/)
  assert.doesNotMatch(storage, /cipher_memory_security\s*=\s*ON/i)
  assert.match(windowsBuilder, /`""\$\{developerShell\}" -arch=amd64/)
  assert.match(windowsBuilder, /windowsVerbatimArguments: true/)
  assert.match(workspaceReleaseFixture, /Cargo\.lock updated/)
  assert.match(workspaceReleaseFixture, /finally \{[\s\S]*writeFileSync\(path, originals\[name\]\)/)
  assert.match(windowsUpgradeFixtureBuilder, /withWorkspaceReleaseVersion/)
  assert.match(windowsUpgradeFixtureBuilder, /runCommand\(root, process\.execPath/)
  assert.doesNotMatch(windowsUpgradeFixtureBuilder, /npm\.cmd/)
  assert.match(windowsUpgradeSmoke, /\/api\/admin\/maintenance\/upgrade/)
  assert.match(windowsUpgradeSmoke, /Start-AvailabilityProbe/)
  assert.match(windowsUpgradeSmoke, /Assert-ScheduledTasksUseInstallRoot/)
  assert.match(windowsUpgradeSmoke, /installation escaped the configured root/)
  assert.match(windowsUpgradeSmoke, /active slot is/)
  assert.match(windowsUpgradeSmoke, /current version deletion was not refused/)
  assert.match(windowsUpgradeSmoke, /failed upgrade changed the active version/)
  assert.match(customerReleaseWorkflow, /npm run build:windows-upgrade-fixture/)
  assert.match(customerReleaseWorkflow, /npm run test:windows-release --[\s\S]*-CandidateArchive/)
  assert.match(windowsReleaseSmoke, /exercise-windows-upgrade\.ps1/)
  assert.doesNotMatch(windowsInstaller, /C:\\ProgramData|D:\\|\/opt\/aster-team/)

  assert.match(macosInstaller, /--install-root/)
  assert.match(macosInstaller, /\$install_root\/releases/)
  assert.match(macosLauncher, /installation marker/)
  assert.match(macosLauncher, /\$install_root\/bin\/aster-team-cli.*maintenance run-next/)
  assert.match(macosRestore, /backup belongs to another installation root or platform/)
  assert.match(macosInstaller, /previous_release/)
  assert.match(macosInstaller, /caddy" fmt --overwrite "\$install_root\/config\/caddy\/Caddyfile"[\s\S]*caddy" validate/)
  assert.match(macosBuilder, /'--platform', 'macos'/)
  assert.match(macosBuilder, /'--runtime', 'native'/)
  assert.doesNotMatch(macosInstaller, /\/opt\/aster-team|\/var\/lib\/aster-team|\/etc\/aster-team/)

  const platformIDs = releasePlatformContract.platforms.map(platform => platform.id)
  assert.deepEqual(platformIDs, ['linux-amd64', 'windows-amd64', 'macos-amd64', 'macos-arm64'])
  for (const platform of releasePlatformContract.platforms.filter(platform => platform.os === 'macos')) {
    assert.equal(platform.verification_status, 'implementation-only')
    assert.deepEqual(platform.smoke_targets, [])
  }
})

test('Linux bootstrap supports a custom root and installs only a command link outside it', () => {
  assert.match(init, /--install-root/)
  assert.match(init, /bootstrap --release-root/)
  assert.match(cli, /InstallLayout::new\(root\)/)
  assert.match(cli, /install_command_link\(&layout, &install_target\)/)
  assert.match(cli, /refused to replace non-link command entry/)
  assert.doesNotMatch(cli, /const CLI_PATH|\/usr\/bin\/aster-team-cli/)
  assert.match(customerInstallSmoke, /install_root='\/srv\/aster-team'/)
  assert.match(customerInstallSmoke, /--install-root "\$install_root"/)
  assert.match(customerInstallSmoke, /Installation escaped the configured root into legacy path/)
  assert.match(customerInstallSmoke, /cli_path='\/usr\/local\/bin\/aster-team-cli'/)
  assert.match(customerInstallSmoke, /sudo "\$cli_path" install/)
  assert.match(customerInstallSmoke, /--upgrade-bundle/)
  assert.match(customerInstallSmoke, /start_availability_probe/)
  assert.match(customerInstallSmoke, /releases\/\$upgrade_version/)
  assert.match(customerInstallSmoke, /rollback_version=/)
  assert.match(customerInstallSmoke, /runner_access_scheme='wss'/)
  assert.match(customerInstallSmoke, /runner_access_host="api\.\$access_host"/)
  assert.match(linuxUpgradeFixtureBuilder, /withWorkspaceReleaseVersion/)
  assert.match(customerReleaseWorkflow, /build-linux-upgrade-fixture\.mjs/)
  assert.match(customerReleaseWorkflow, /--upgrade-bundle "\$upgrade_bundle"/)
  assert.match(runnerInstallSmoke, /install_root='\/srv\/aster-team-runner'/)
  assert.match(runnerInstallSmoke, /getent group aster-team/)
  assert.match(runnerInstallSmoke, /sudo "\$cli_path" runner install/)
  assert.match(runnerInstallSmoke, /A same-version Runner candidate unexpectedly upgraded/)
})

test('internal Operations installation also keeps owned files below its configurable root', () => {
  assert.match(operationsInstaller, /--install-root/)
  assert.match(operationsInstaller, /--service-registration-root/)
  assert.match(operationsInstaller, /install_root='\/opt\/aster-operations'/)
  assert.match(operationsInstaller, /config_root="\$install_root\/config"/)
  assert.match(operationsInstaller, /data_root="\$install_root\/data"/)
  assert.match(operationsInstaller, /backup_root="\$install_root\/backups"/)
  assert.match(operationsInstaller, /ASTER_OPERATIONS_ARTIFACT_ROOT must resolve below <install-root>\/data\//)
  assert.match(operationsApiUnit, /@ASTER_OPERATIONS_ROOT@\/config\/operations-api\.env/)
  assert.match(operationsApiUnit, /ReadWritePaths=@ASTER_OPERATIONS_ROOT@/)
  assert.match(operationsWebUnit, /@ASTER_OPERATIONS_ROOT@\/current\/bin\/aster-webhost/)
  assert.match(operationsEnvExample, /ASTER_OPERATIONS_ARTIFACT_ROOT=data\/artifacts/)
  for (const source of [operationsInstaller, operationsApiUnit, operationsWebUnit]) {
    assert.doesNotMatch(source, /\/etc\/aster-team|\/var\/lib\/aster-operations/)
  }
  assert.doesNotMatch(operationsInstaller, /mktemp "\/etc\/systemd\/system/)
})

test('Linux installer keeps all owned content under the selected root', () => {
  for (const fragment of [
    'config/control', 'config/runner', 'config/caddy', 'config/keys',
    'config/license', 'data/database', 'data/caddy', 'state/upgrades',
    'staging/upgrades', 'backups/upgrades', 'logs',
  ]) assert.ok(layoutContractSource.includes(fragment), `${fragment} missing from layout`)
  assert.match(installer, /install_root="\$\{install_root%\/\}"/)
  assert.match(installer, /config_parent="\$install_root\/config"/)
  assert.match(installer, /config_root="\$config_parent\/control"/)
  assert.match(installer, /database_root="\$data_root\/database"/)
  assert.match(installer, /install -d -o root -g root -m 0755 "\$install_root" "\$release_root" "\$config_parent"/)
  assert.match(installer, /install -d -o root -g root -m 0755 "\$data_root"/)
  assert.match(installer, /install -d -o root -g aster-caddy -m 0750 "\$caddy_config_root" "\$tls_root"/)
  assert.doesNotMatch(installer, /install -d -o aster-team -g aster-team -m 0750 "\$data_root"/)
  assert.match(installer, /service_registration_root/)
  assert.doesNotMatch(installer, /\/(?:etc|var\/lib|var\/backups)\/aster-(?:team|runner|caddy)/)
  assert.doesNotMatch(installer, /\/etc\/systemd\/system/)
  assert.match(installer, /systemd\/aster-control@\.service/)
  assert.doesNotMatch(installer, /aster-control\.service/)
  assert.match(installer, /caddy" fmt --overwrite "\$caddy_config_root\/Caddyfile"[\s\S]*caddy" validate/)
  assert.match(windowsInstaller, /'fmt', '--overwrite',[\s\S]*config\\caddy\\Caddyfile[\s\S]*'validate', '--config'/)
})

test('Control runs in two internal slots behind one stable Caddy entry', () => {
  assert.match(upgradeCore, /Blue[\s\S]*?11_380[\s\S]*?11_381[\s\S]*?11_382/)
  assert.match(upgradeCore, /Green[\s\S]*?11_480[\s\S]*?11_481[\s\S]*?11_482/)
  assert.match(controlUnit, /EnvironmentFile=@ASTER_ROOT@\/config\/control\/control-%i\.env/)
  assert.match(controlUnit, /state\/slots\/%i-release\/bin\/aster-control serve/)
  assert.match(controlUnit, /TimeoutStopSec=120s/)
  assert.match(controlUnit, /ReadWritePaths=.*@ASTER_ROOT@\/config\/license/)
  assert.match(caddyUnit, /@ASTER_ROOT@\/config\/caddy\/Caddyfile/)
  assert.match(upgradePath, /state\/upgrades\/queued\/\*\.json/)
  assert.match(upgradeService, /@ASTER_ROOT@\/bin\/aster-team-cli maintenance run-next/)
})

test('local slot Runner launch uses its release and private material independently of Caddy', () => {
  for (const slot of ['blue', 'green']) {
    const rendered = slotRunnerUnit.replaceAll('@ASTER_ROOT@', '/opt/aster-team').replaceAll('%i', slot)
    const directory = `/opt/aster-team/config/runner/slots/${slot}`
    assert.ok(rendered.includes(`ExecStart=/opt/aster-team/state/slots/${slot}-release/bin/aster-runner serve-slot --slot ${slot} `))
    for (const leaf of ['runner.env', 'identity.json', 'task-keys.json']) {
      assert.ok(rendered.includes(`ConditionPathExists=${directory}/${leaf}`))
    }
    assert.ok(rendered.includes(`EnvironmentFile=${directory}/runner.env`))
    assert.equal(layoutContract.paths[`runner_${slot}_identity`], `config/runner/slots/${slot}/identity.json`)
    assert.equal(layoutContract.paths[`runner_${slot}_task_keys`], `config/runner/slots/${slot}/task-keys.json`)
    assert.equal(layoutContract.paths[`runner_${slot}_environment`], `config/runner/slots/${slot}/runner.env`)
    assert.equal(layoutContract.paths[`control_${slot}_runtime_token`], `config/control/runtime-${slot}.token`)
  }
  assert.doesNotMatch(slotRunnerUnit, /\/current\b|ASTER_RUNNER_CONTROL_WSS|aster-caddy|PartOf=|BindsTo=/)
  assert.match(slotRunnerUnit, /^Restart=on-failure$/m)
  assert.match(slotRunnerUnit, /User=aster-runner/)
  assert.match(slotRunnerUnit, /ProtectSystem=strict/)
  assert.match(slotRunnerUnit, /InaccessiblePaths=.*config\/keys.*config\/control/)
  assert.match(builder, /cpSync\(resolve\(root, 'customer\/deploy\/systemd'\), resolve\(bundle, 'systemd'\), \{ recursive: true \}\)/)
  // Fresh installation remains compatible; maintenance activates slots only
  // when the candidate declares the new runtime metadata and its probe passes.
  assert.match(runnerUnit, /current\/bin\/aster-runner serve --control-wss/)
  assert.match(installer, /install_unit 'aster-runner@\.service'/)
  assert.doesNotMatch(installer, /systemctl (?:enable|start|restart) [^\n]*aster-runner@/)
  assert.match(upgradeExecutor, /slot_preparation::prepare_if_supported/)
  assert.match(restore, /aster-runner@blue\.service aster-runner@green\.service/)
})

test('maintenance upgrade stops business services before starting a candidate and never downgrades data', () => {
  const verify = upgradeExecutor.indexOf('verify_release_at(&release_root)')
  const stop = upgradeExecutor.indexOf('quiesce_services()?', verify)
  const prepare = upgradeExecutor.indexOf('slot_preparation::prepare_if_supported', stop)
  const start = upgradeExecutor.indexOf('start_slot(candidate_slot)', prepare)
  const health = upgradeExecutor.indexOf('wait_for_slot(layout, candidate_slot', start)
  const runnerReady = upgradeExecutor.indexOf('restore_runner(layout, &candidate, runner_was_running)', health)
  const switchTraffic = upgradeExecutor.indexOf('MaintenanceStatus::SwitchingTraffic', health)
  const restore = upgradeExecutor.indexOf('let restore =', switchTraffic)
  const stopCandidate = upgradeExecutor.indexOf('quiesce_services()?', restore)
  const restartPrevious = upgradeExecutor.indexOf('run_service_action("start", &[previous_unit]', restore)
  assert.ok(verify >= 0 && stop > verify && prepare > stop && start > prepare && health > start && runnerReady > health && switchTraffic > runnerReady)
  assert.ok(restore > switchTraffic && stopCandidate > restore && restartPrevious > stopCandidate)
  assert.match(upgradeExecutor, /forward_only_no_automatic_restore/)
  assert.match(upgradeExecutor, /replace_directory_link\(layout, &layout\.current\(\), candidate_release\)/)
  assert.match(upgradeExecutor, /atomic_replace\(layout\.caddy_upstreams\(\)/)
  assert.match(upgradeExecutor, /candidate_is_active/)
  assert.match(upgradeExecutor, /reconcile_interrupted_upgrade/)
  assert.match(upgradeExecutor, /candidate version must be newer than the active version/)
  assert.match(upgradeExecutor, /different signed build already occupies the target version/)
  assert.match(upgradeExecutor, /old_selected_release/)
  assert.match(upgradeExecutor, /record_upgrade_audit\(candidate_release, "succeeded", version\)/)
  assert.match(upgradeExecutor, /record_upgrade_audit\(&previous_release, "failed", version\)/)
  assert.match(upgradeExecutor, /install_service_assets\(layout, candidate_release\)/)
  assert.match(upgradeExecutor, /install_stable_cli\(layout, candidate_release\)/)
  assert.match(upgradeExecutor, /run_service_action\("enable", &\[candidate_unit\]/)
  assert.match(upgradeExecutor, /run_service_action\("disable", &\[previous_unit\]/)
})

test('maintenance launchers use the stable CLI independently of the current release', () => {
  assert.match(upgradeService, /@ASTER_ROOT@\/bin\/aster-team-cli maintenance run-next/)
  const windowsMaintenance = windowsLauncher.slice(windowsLauncher.indexOf("        'maintenance' {"))
  assert.match(windowsMaintenance, /Join-Path \$AsterRoot 'bin\\aster-team-cli\.exe'/)
  assert.doesNotMatch(windowsMaintenance, /Resolve-AsterRelease/)
  assert.match(macosLauncher, /exec "\$install_root\/bin\/aster-team-cli" maintenance run-next/)
})

test('database migrations are embedded, checksummed, serialized and applied during startup', () => {
  assert.match(storage, /schema_migrations/)
  assert.match(storage, /checksum/)
  assert.match(storage, /SQLCIPHER_MIGRATIONS/)
  assert.match(storage, /MigrationCompatibility::RollingUpgradeSafe/)
  assert.match(storage, /identity-written-by-older-app/)
  assert.match(mariadbStorage, /GET_LOCK/)
  assert.match(storage, /TransactionBehavior::Immediate/)
  assert.doesNotMatch(storage, /unsupported; expected/)
})

test('Admin exposes upload, progress and non-current version deletion without manual rollback', () => {
  assert.match(controlEntrypoints, /post "\/api\/admin\/maintenance\/upgrade" => admin_queue_upgrade, Support\([^\n]+, false, limit = 1024 \* 1024 \* 1024;/)
  assert.match(controlEntrypoints, /delete "\/api\/admin\/maintenance\/versions\/\{version\}" => admin_queue_version_delete, Support\(/)
  assert.match(controlSource, /track_requests\(entrypoints::api_router\(state\.clone\(\)\), &state\)/)
  assert.match(controlSource, /MAX_ARCHIVE_BYTES/)
  assert.match(maintenanceView, /FormData/)
  assert.match(maintenanceView, /setTimeout/)
  assert.match(maintenanceView, /deleteVersion/)
  assert.doesNotMatch(maintenanceView, /rollback|回滚|切换到旧/)
  assert.match(adminRouter, /maintenance/)
  assert.match(sdk, /body instanceof FormData/)
})

test('backup and restore keep transaction state inside the configured root', () => {
  assert.match(cli, /resolve_backup_output/)
  assert.match(cli, /install_layout\(\)\.backups\(\)/)
  assert.match(cli, /create_backup_temporary/)
  assert.match(cli, /backup output inside the installation root must be below/)
  assert.match(cli, /--exclude=\{\}\/backups\/\*/)
  assert.match(cli, /--exclude=\{\}\/staging\/\*/)
  assert.match(restore, /Backup does not contain exactly the configured installation root/)
  assert.match(restore, /workspace="\$\(mktemp -d "\$install_root\/staging\/restores\/restore\.XXXXXX"\)"/)
  assert.match(restore, /safety="\$install_root\/backups\/aster-team-before-restore-/)
  assert.match(customerInstallSmoke, /sudo test -f "\$backup"/)
  assert.match(customerInstallSmoke, /-name 'aster-team-before-restore-\*\.tar\.gz'/)
  assert.match(restore, /managed_entries=\(bin releases config data state logs current install\.json\)/)
  assert.match(restore, /move_entries "\$install_root" "\$previous"/)
  assert.match(restore, /move_entries "\$prepared" "\$install_root"/)
  assert.match(restore, /--service-registration-root/)
  assert.match(restore, /install_release_units/)
  assert.match(restore, /chown -R root:aster-caddy "\$prepared\/config\/tls"/)
  assert.match(restore, /chmod 0644 "\$prepared\/config\/tls\/caddy-root\.crt"/)
  assert.match(cli, /\.arg\("--service-registration-root"\)/)
  assert.match(restore, /the original installation was restored/)
  assert.doesNotMatch(restore, /\$parent\/\.aster-team-restore|before-restore\.\$\$/)
  assert.doesNotMatch(restore, /\/etc\/aster-team|\/var\/lib\/aster-team|\/var\/backups\/aster-team/)
})

test('Runner files remain isolated below the same root and retain their service identity', () => {
  assert.match(runnerUnit, /@ASTER_ROOT@\/config\/runner\/runner\.env/)
  assert.match(runnerUnit, /@ASTER_ROOT@\/current\/bin\/aster-runner serve/)
  assert.match(installer, /install -o root -g aster-runner -m 0640/)
  assert.match(runnerSource, /token_file: PathBuf/)
  assert.doesNotMatch(runnerSource, /\#\[arg\(long\)\]\s+token: String/)
  assert.match(runnerAdmin, /buildRunnerEnrollmentCommand/)
})

test('Every Runner entry point allows all Control-owned upstream hosts', () => {
  const hosts = new Set([...controlSource.matchAll(
    /const [A-Z0-9_]+_ENDPOINT: &str\s*=\s*"(https:\/\/[^\"]+)"/g,
  )].map(([, endpoint]) => new URL(endpoint).hostname))
  assert.ok(hosts.has('auth.openai.com'))
  assert.ok(hosts.has('chatgpt.com'))
  for (const source of [runnerUnit, installer, cli, localRunnerLauncher]) {
    for (const host of hosts) assert.ok(source.includes(host), `${host} is missing from a Runner entry point`)
  }
})

test('readiness and diagnostics understand active slots and stable public access', () => {
  assert.match(serviceHealth, /state\/slots\/active\.json/)
  assert.match(serviceHealth, /ASTER_HEALTH_TIMEOUT_SECONDS=60/)
  assert.match(serviceHealth, /aster_wait_for_access_health/)
  assert.match(diagnosticCollector, /aster-control@blue\.service/)
  assert.match(diagnosticCollector, /aster-control@green\.service/)
  assert.match(diagnosticCollector, /command -v aster-team-cli/)
  assert.match(diagnosticCollector, /ASTER_INSTALL_ROOT/)
  assert.doesNotMatch(diagnosticCollector, /find \/opt\/aster-team/)
  assert.match(supportedInstallSmoke, /aster-control@blue\.service/)
})

test('Linux unified bundle verifies and installs an optional signed free license only on first install', () => {
  const layoutContract = JSON.parse(layoutContractSource)
  assert.equal(layoutContract.paths.license_state, 'state/license.json')
  assert.match(builder, /ASTER_CUSTOMER_FREE_LICENSE_FILE/)
  assert.match(linuxLab, /free_no_expiry/)
  assert.match(linuxLab, /ASTER_CUSTOMER_FREE_LICENSE_FILE=\/lab\/signing\/free-license\.json/)
  assert.match(linuxLab, /--expected-bundled-license-id test_free_no_expiry/)
  assert.match(linuxLab, /go build -trimpath[\s\S]*operations\/backend\/cmd\/lablicensesigner/)
  assert.match(linuxLab, /expected_license_args\+=\(--exercise-paid-license-switch\)/)
  assert.match(supportedInstallSmoke, /--exercise-paid-license-switch/)
  assert.match(supportedInstallSmoke, /--lab-paid-license-signer \/workspace\/package\/lablicensesigner/)
  assert.match(customerInstallSmoke, /Paid License switch verification requires a bundled free License/)
  assert.match(customerInstallSmoke, /--installation-profile "\$install_root\/config\/license\/installation\.json"/)
  assert.match(customerInstallSmoke, /api\/admin\/license\/preview/)
  assert.match(customerInstallSmoke, /api\/admin\/license"/)
  assert.match(customerInstallSmoke, /--cookie "\$owner_cookie_jar"/)
  assert.match(customerInstallSmoke, /grep -Fqx 'member seats: 20'/)
  assert.match(customerInstallSmoke, /Bundled free License unexpectedly replaced the paid License/)
  assert.match(customerInstallSmoke, /sudo chown root:root "\$install_root\/state\/license\.json\.lock"/)
  assert.match(customerInstallSmoke, /aster-team:aster-team:640/)
  assert.match(customerInstallSmoke, /expected_active_license_sha256/)
  assert.match(customerInstallSmoke, /assert_owner_login/)
  assert.match(customerInstallSmoke, /"password_change_required"\[\[:space:\]\]\*:/)
  assert.match(customerInstallSmoke, /api\/admin\/auth\/password/)
  assert.match(customerInstallSmoke, /owner_password_rotated=1/)
  assert.match(customerInstallSmoke, /Bundled free License unexpectedly replaced the paid License\.[\s\S]*probe_access\s+assert_owner_login/)
  assert.match(customerInstallSmoke, /expected_bundled_license_id/)
  assert.doesNotMatch(customerInstallSmoke, /test_free_no_expiry/)
  assert.match(customerInstallSmoke, /state\/license\.json\.lock/)
  assert.match(customerInstallSmoke, /tee "\$install_log"/)
  assert.match(customerInstallSmoke, /grep -Fq 'A license was installed during setup\.'/)
  assert.match(customerInstallSmoke, /grep -Fq 'license: active'/)
  assert.match(customerInstallSmoke, /grep -Eq '\^member seats: \[0-9\]\+\$'/)
  assert.match(customerInstallSmoke, /grep -Eq '\^expires: \(never\|/)
  assert.match(customerInstallSmoke, /Bundled free license installation unexpectedly generated a machine-bound license request/)
  assert.match(bundledFreeLicense, /constants\.O_RDONLY \| constants\.O_NOFOLLOW \| \(constants\.O_NONBLOCK \?\? 0\)/)
  assert.match(bundledFreeLicense, /MAX_BUNDLED_FREE_LICENSE_BYTES = 64 \* 1024/)
  assert.match(bundledFreeLicense, /Buffer\.allocUnsafe\(MAX_BUNDLED_FREE_LICENSE_BYTES \+ 1\)/)
  assert.match(bundledFreeLicense, /writeFileSync\(target, bytes, \{ flag: 'wx' \}\)/)
  assert.match(bundledFreeLicense, /resolve\(bundle, 'bin\/aster-control'\)/)
  assert.match(bundledFreeLicense, /\['verify-bundled-free-license', '--source', target\]/)
  assert.ok(builder.indexOf('stageBundledFreeLicense(bundle') < builder.indexOf('normalizeReleaseModes(bundle)'))
  assert.ok(builder.indexOf('normalizeReleaseModes(bundle)') < builder.indexOf('verifyStagedBundledFreeLicense('))
  assert.ok(builder.indexOf('verifyStagedBundledFreeLicense(') < builder.indexOf("'sign', '--root', bundle"))

  assert.match(installer, /bundled_free_license="\$bundle_root\/licenses\/free-license\.json"/)
  assert.match(installer, /Bundled free license must be an ordinary file/)
  assert.match(installer, /verify-bundled-free-license[\s\S]*--minimum-valid-for-seconds 900/)
  assert.match(installer, /Bundled free license bootstrap failed before account or database initialization/)
  assert.match(installer, /install -o root -g aster-team -m 0750 "\$verifier" "\$license_bootstrap_root\/aster-control"/)
  assert.match(installer, /install -o root -g aster-team -m 0640 "\$bundled_free_license" "\$license_bootstrap_root\/free-license.json"/)
  assert.match(installer, /if ! "\$license_bootstrap_root\/aster-control" install-license --source "\$license_bootstrap_root\/free-license.json"/)
  assert.doesNotMatch(installer, /runuser -u aster-team -- "\$license_bootstrap_root\/aster-control" install-license/)
  assert.match(installer, /restore_license_state_ownership\(\)[\s\S]*\$state_root\/license\.json[\s\S]*\$state_root\/license\.json\.pending[\s\S]*\$state_root\/license\.json\.lock[\s\S]*\$state_root\/license\.json\.staged[\s\S]*\$state_root\/license\.json\.activation[\s\S]*\$state_root\/license\.json\.mutation/)
  assert.equal((installer.match(/restore_license_state_ownership\n/g) || []).length, 2)
  assert.match(installer, /runner_only -eq 0 && \$first_install -eq 1 && \$recover_preserved -eq 0 && \( \$bundled_free_selected -eq 1 \|\| -n "\$license_source" \)/)
  assert.match(installer, /license_import_root="\$config_root\/\.license-import"/)
  assert.match(installer, /install -d -o root -g aster-team -m 0750 "\$license_import_root"/)
  assert.ok(!installer.includes('staged_explicit_license="$installer_temp_root'))
  assert.match(installer, /install -o root -g aster-team -m 0640 "\$license_source" "\$staged_explicit_license"/)
  for (const suffix of ['', '.pending', '.lock', '.staged', '.activation', '.mutation']) {
    assert.ok(installer.includes(`"$state_root/license.json${suffix}"`))
  }
  assert.ok(!installer.includes('$license_root/state.json'))
  assert.match(installer, /initial_license_source="\$staged_explicit_license"[\s\S]*-z "\$initial_license_source"[\s\S]*\$release_dir\/licenses\/free-license\.json/)
  assert.match(installer, /"\$release_dir\/bin\/aster-control" install-license --source "\$initial_license_source"/)
  assert.doesNotMatch(installer, /runuser -u aster-team -- "\$release_dir\/bin\/aster-control" install-license/)
  const firstInstall = installer.indexOf('if [[ $first_install -eq 1 ]]; then')
  const bundledPreflight = installer.indexOf('verify-bundled-free-license')
  const bundledBootstrap = installer.indexOf('"$license_bootstrap_root/aster-control" install-license --source "$license_bootstrap_root/free-license.json"')
  const accountConfiguration = installer.indexOf('control_env_temp=')
  const databaseInitialization = installer.indexOf('initialize-database')
  const licenseSelection = installer.indexOf('initial_license_source="$staged_explicit_license"')
  const localRunnerInitialization = installer.indexOf('initialize-local-runner')
  const upgradeBranch = installer.indexOf('Control upgrades must be submitted through')
  assert.ok(bundledPreflight >= 0 && bundledPreflight < bundledBootstrap)
  assert.ok(bundledBootstrap < accountConfiguration && bundledBootstrap < databaseInitialization)
  assert.ok(firstInstall >= 0 && firstInstall < licenseSelection && licenseSelection < upgradeBranch)
  assert.ok(licenseSelection < localRunnerInitialization)

  assert.match(cli, /fn complete_initial_license_setup\(\)[\s\S]*initial_license_is_installed[\s\S]*show_license_status\(\)[\s\S]*LicenseCommand::Request/)
  const installControl = cli.slice(cli.indexOf('fn install_control('), cli.indexOf('fn recover_preserved_control('))
  assert.equal((installControl.match(/complete_initial_license_setup\(\)/g) || []).length, 2)
  assert.doesNotMatch(installControl, /manage_license\(LicenseCommand::Request/)
})

test('supported Linux smoke matrix and static runtime policy remain intact', () => {
  const platform = releasePlatformContract.platforms.find(candidate => candidate.id === 'linux-amd64')
  assert.deepEqual(platform.smoke_targets.map(target => target.id), [
    'ubuntu-20.04', 'ubuntu-22.04', 'ubuntu-24.04', 'debian-12', 'debian-13', 'rocky-linux-9',
  ])
  assert.match(smokeMatrixEmitter, /platform\.smoke_targets/)
  assert.match(supportedInstallSmoke, /--privileged/)
  assert.match(supportedInstallSmoke, /--tmpfs \/sys:rw/)
  assert.match(supportedInstallSmoke, /chmod 0400 \/sys\/class\/dmi\/id\/product_uuid/)
  assert.match(aptSystemdImage, /systemd-sysv/)
  assert.match(dnfSystemdImage, /shadow-utils/)
  assert.match(builder, /x86_64-unknown-linux-musl/)
  assert.match(builder, /'--runtime', 'musl-static'/)
  assert.match(cargoConfig, /link-self-contained=yes/)
  assert.match(staticRuntimeVerifier, /readelf -l .*INTERP/)
  assert.match(staticRuntimeVerifier, /readelf -d .*NEEDED/)
})

test('release packaging keeps private engines internal and includes service templates', () => {
  assert.match(builder, /customer\/deploy\/init\.sh/)
  assert.match(builder, /libexec\/install\.sh/)
  assert.match(builder, /libexec\/restore-backup\.sh/)
  assert.match(builder, /customer\/deploy\/systemd/)
  assert.match(builder, /normalizeReleaseModes\(bundle\)/)
  assert.doesNotMatch(builder, /resolve\(bundle, 'install\.sh'\)/)
  assert.match(releaseArchive, /archivePathname\.endsWith\('\/restore-backup\.sh'\)/)
  assert.match(boundaries, /"libexec"/)
})

test('release workflows keep security and isolated systemd smoke coverage', () => {
  assert.match(verifyWorkflow, /on:\s*\n\s*workflow_dispatch:/)
  assert.doesNotMatch(verifyWorkflow, /\n\s*push:/)
  assert.match(customerReleaseWorkflow, /workflow_dispatch:/)
  assert.match(customerReleaseWorkflow, /\n\s*push:\s*\n\s*tags: \['v\*'\]/)
  for (const workflow of [verifyWorkflow, customerReleaseWorkflow]) {
    assert.match(workflow, /scripts\/ci\/release-smoke-matrix\.mjs linux-amd64/)
    assert.match(workflow, /scripts\/ci\/exercise-supported-linux-install\.sh/)
    assert.match(workflow, /npm run release:preflight/)
    assert.doesNotMatch(workflow, /cargo audit --deny warnings/)
    assert.doesNotMatch(workflow, /npm audit --omit=dev --audit-level=high/)
  }
  assert.match(releaseSecurityPreflight, /arguments: \['audit', '--deny', 'warnings'\]/)
  assert.match(releaseSecurityPreflight, /'--omit=dev',[\s\S]*'--audit-level=high'/)
  assert.ok(
    releaseSecurityPreflight.indexOf("id: 'production-node-dependency-audit'")
      < releaseSecurityPreflight.indexOf("id: 'rust-dependency-audit'"),
    'the inexpensive Node.js audit must fail before Rust audit setup',
  )
  assert.match(customerReleaseWorkflow, /preflight:[\s\S]*npm run release:preflight[\s\S]*asterctl-windows:[\s\S]*needs: preflight/)
  assert.match(linuxLab, /doctor\|quick\|full/)
  assert.match(linuxLab, /--reuse-package/)
  assert.match(linuxLab, /if \[\[ \$reuse_package -eq 0 \]\]; then\s+build_toolchain_image/)
  assert.match(linuxLab, /MINGW\*\|MSYS\*\|CYGWIN\*/)
  assert.match(linuxLab, /MSYS_NO_PATHCONV=1/)
  assert.match(linuxLab, /node_modules_volume/)
  assert.match(linuxLab, /target_volume/)
  assert.match(linuxLab, /dist_volume/)
  assert.match(linuxLab, /prepare_toolchain_state/)
  assert.match(linuxLab, /\/lab\/output/)
  assert.match(linuxLab, /'ubuntu-20\.04' 'ubuntu:20\.04'/)
  assert.match(supportedInstallSmoke, /docker_host_path/)
  assert.match(supportedInstallSmoke, /ci-images\.mjs" "\$target"/)
  assert.doesNotMatch(supportedInstallSmoke, /docker_cli build/)
  assert.match(supportedInstallSmoke, /package-input:ro/)
  assert.match(supportedInstallSmoke, /cp -a \/workspace\/package-input\/\. \/workspace\/package\//)
  assert.match(supportedInstallSmoke, /test -s \/etc\/machine-id/)
  assert.match(supportedInstallSmoke, /test -s \/sys\/class\/dmi\/id\/product_uuid/)
  assert.match(customerReleaseWorkflow, /Verify Linux primary \(ubuntu-20\.04\)/)
  assert.match(customerReleaseWorkflow, /runner-install:[\s\S]*needs: \[preflight, build, linux-primary\]/)
  assert.match(customerReleaseWorkflow, /--exclude ubuntu-20\.04/)
  assert.match(linuxLabDockerfile, /rust:1\.95\.0-slim-bookworm/)
})
