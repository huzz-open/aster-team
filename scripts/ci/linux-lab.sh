#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

usage() {
  cat <<'EOF'
Usage:
  bash scripts/ci/linux-lab.sh doctor
  bash scripts/ci/linux-lab.sh quick [--jobs N] [--reuse-package]
  bash scripts/ci/linux-lab.sh full  [--jobs N] [--reuse-package]

Profiles:
  doctor  Check the persistent Linux build host without changing it.
  quick   Build one test-signed package, then validate Customer and Runner in
          two isolated Ubuntu systemd containers.
  full    Run npm verify, build once, then validate Customer and Runner across
          every supported Linux smoke target in isolated systemd containers.

Options:
  --jobs N          Maximum concurrent install containers. Defaults to 2 for
                    quick and up to 4 for full, bounded by available CPUs.
  --reuse-package   Skip compilation and use the current version's existing
                    dist/linux archive. Intended only for repeated runtime tests.
  --export-bundle DIRECTORY
                    Copy the freshly built unpacked bundle to a new directory for
                    same-run E2E metadata, including on Windows Docker hosts.
  --export-v2-signers FILE
                    Export the ephemeral paid v2 signer and its exact trusted
                    scope for same-run Operations. Delete FILE after use.
  --asterctl-windows-x64 FILE
                    Include a real native Windows tool built from this source.
                    Without this option, the lab uses a non-runnable fixture.
  -h, --help        Show this help.

The host only needs Docker and common shell tools. Linux and Windows Git Bash
hosts are supported. Pinned Node, Go, Rust and musl toolchains run inside a
reusable build container. The script never uses a production signing key and
never installs Aster onto the host. Generated signing material and install
roots live in temporary files or disposable privileged Docker containers.

Toolchain and runtime images are pulled by digest from the public GHCR
packages used by Actions. Update the images separately
with npm run ci:images:update; tests never build or repair their base images.
ASTER_LINUX_LAB_CARGO_INDEX,
ASTER_LINUX_LAB_GO_PROXY and ASTER_LINUX_LAB_NPM_REGISTRY may replace language
package registries inside toolchain containers.
EOF
}

profile="${1:-}"
[[ -n "$profile" ]] || { usage >&2; exit 2; }
shift
case "$profile" in
  doctor|quick|full) ;;
  -h|--help) usage; exit 0 ;;
  *) echo "Unknown profile: $profile" >&2; usage >&2; exit 2 ;;
esac

jobs=0
reuse_package=0
export_v2_signers=''
export_bundle=''
asterctl_windows_x64=''
while (($#)); do
  case "$1" in
    --jobs) jobs="${2:-}"; shift 2 ;;
    --reuse-package) reuse_package=1; shift ;;
    --export-v2-signers) export_v2_signers="${2:-}"; shift 2 ;;
    --export-bundle) export_bundle="${2:-}"; shift 2 ;;
    --asterctl-windows-x64)
      [[ -n "${2:-}" ]] || { echo '--asterctl-windows-x64 requires a file.' >&2; exit 2; }
      asterctl_windows_x64="$2"; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done
if [[ -n "$asterctl_windows_x64" ]]; then
  [[ $reuse_package -eq 0 && -f "$asterctl_windows_x64" && ! -L "$asterctl_windows_x64" ]] || {
    echo '--asterctl-windows-x64 requires a fresh build and a regular tool file.' >&2
    exit 2
  }
  asterctl_windows_x64="$(cd -- "$(dirname -- "$asterctl_windows_x64")" && pwd -P)/$(basename -- "$asterctl_windows_x64")"
fi
if [[ -n "$export_v2_signers" ]]; then
  [[ $reuse_package -eq 0 && -d "$(dirname -- "$export_v2_signers")" && ! -e "$export_v2_signers" && ! -L "$export_v2_signers" ]] || {
    echo '--export-v2-signers requires a fresh build and a new file with an existing parent.' >&2
    exit 2
  }
  export_v2_signers="$(cd -- "$(dirname -- "$export_v2_signers")" && pwd -P)/$(basename -- "$export_v2_signers")"
fi
if [[ -n "$export_bundle" ]]; then
  [[ $reuse_package -eq 0 && -d "$(dirname -- "$export_bundle")" && ! -e "$export_bundle" && ! -L "$export_bundle" ]] || {
    echo '--export-bundle requires a fresh build and a new destination with an existing parent.' >&2
    exit 2
  }
  export_bundle="$(cd -- "$(dirname -- "$export_bundle")" && pwd -P)/$(basename -- "$export_bundle")"
fi

[[ "$jobs" =~ ^[0-9]+$ ]] || { echo '--jobs must be a positive integer.' >&2; exit 2; }
if [[ $jobs -eq 0 ]]; then
  if [[ "$profile" == quick ]]; then
    jobs=2
  else
    cpu_count="$(nproc 2>/dev/null || echo 2)"
    jobs=$((cpu_count / 2))
    [[ $jobs -ge 1 ]] || jobs=1
    [[ $jobs -le 4 ]] || jobs=4
  fi
fi
[[ $jobs -ge 1 && $jobs -le 12 ]] || { echo '--jobs must be between 1 and 12.' >&2; exit 2; }

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
cd "$root"
host_system="$(uname -s)"
windows_docker_host=0
case "$host_system" in
  Linux) ;;
  MINGW*|MSYS*|CYGWIN*) windows_docker_host=1 ;;
  *)
    echo "The Linux lab does not support this host: $host_system" >&2
    exit 1
    ;;
esac
[[ "$(uname -m)" == x86_64 ]] || {
  echo 'The Linux lab requires an x86_64 host.' >&2
  exit 1
}
if [[ $windows_docker_host -eq 1 ]]; then
  command -v cygpath >/dev/null 2>&1 || {
    echo 'Windows Linux validation must run from Git Bash with cygpath available.' >&2
    exit 1
  }
fi
if [[ ${EUID} -eq 0 ]]; then
  echo 'Warning: running as root; use only on a dedicated disposable validation host.' >&2
fi

required=(awk bash cp date docker git grep head id mkdir mktemp node rm sed sha256sum tail tar tr uname)
missing=()
for command in "${required[@]}"; do
  command -v "$command" >/dev/null 2>&1 || missing+=("$command")
done
if [[ ${#missing[@]} -gt 0 ]]; then
  printf 'Missing Linux lab dependencies: %s\n' "${missing[*]}" >&2
  echo 'The Linux lab does not install host dependencies.' >&2
  exit 1
fi
docker_command=(docker)
docker_cli() {
  if [[ $windows_docker_host -eq 1 ]]; then
    MSYS_NO_PATHCONV=1 "${docker_command[@]}" "$@"
  else
    "${docker_command[@]}" "$@"
  fi
}
docker_host_path() {
  if [[ $windows_docker_host -eq 1 ]]; then
    cygpath --absolute --mixed "$1"
  else
    printf '%s\n' "$1"
  fi
}
if ! docker_cli info >/dev/null 2>&1; then
  if command -v sudo >/dev/null 2>&1 && sudo -n docker info >/dev/null 2>&1; then
    docker_command=(sudo -n docker)
  else
    echo 'Docker is installed, but this user cannot access its daemon directly or through passwordless sudo.' >&2
    exit 1
  fi
fi
git rev-parse --show-toplevel >/dev/null 2>&1 || {
  echo 'Run the Linux lab from an Aster Team Git worktree.' >&2
  exit 1
}

echo 'Aster Linux lab environment'
echo "  Host:       $(uname -srmo)"
echo "  Docker:     $(docker_cli version --format '{{.Server.Version}}')"
echo '  Toolchains: pinned build container (Node 22.19.0, Go 1.25.14, Rust 1.95.0)'
echo "  Parallel:   $jobs"
if [[ "$profile" == doctor ]]; then
  echo 'Result: Linux lab doctor passed.'
  exit 0
fi

version="$(sed -n 's/^[[:space:]]*"version"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' package.json | head -n 1)"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
  echo 'package.json contains an invalid version.' >&2
  exit 1
}
run_id="$(date -u +%Y%m%dT%H%M%SZ)-$$"
state_root="$root/target/linux-lab"
log_root="$state_root/runs/$run_id"
diagnostic_root="$log_root/diagnostics"
mkdir -p -- "$log_root" "$diagnostic_root"
cache_root="${ASTER_LINUX_LAB_CACHE_ROOT:-${XDG_CACHE_HOME:-$HOME/.cache}/aster-linux-lab}"
mkdir -p -- "$cache_root"
cache_root="$(cd -- "$cache_root" && pwd -P)"
lab_root="$(mktemp -d "${TMPDIR:-/tmp}/aster-linux-lab.XXXXXX")"
cleanup() {
  rm -rf -- "$lab_root"
}
trap cleanup EXIT

archive="$root/dist/linux/aster-team-${version}-linux-amd64.tar.gz"
checksum="$archive.sha256"
bundle="$root/dist/linux/aster-team-${version}-linux-amd64"
toolchain_image="$(node scripts/ci/ci-images.mjs linux-builder)"
for legacy in ASTER_LINUX_LAB_NODE_IMAGE ASTER_LINUX_LAB_GO_IMAGE ASTER_LINUX_LAB_RUST_IMAGE \
  ASTER_LINUX_LAB_DEBIAN_MIRROR ASTER_LINUX_LAB_DEBIAN_SECURITY_MIRROR \
  ASTER_LINUX_LAB_UBUNTU_MIRROR ASTER_LINUX_LAB_UBUNTU_SECURITY_MIRROR ASTER_LINUX_LAB_RUSTUP_DIST_SERVER; do
  [[ -z "${!legacy:-}" ]] || { echo "$legacy cannot override pinned CI images; update them separately." >&2; exit 2; }
done
cargo_index="${ASTER_LINUX_LAB_CARGO_INDEX:-}"
go_proxy="${ASTER_LINUX_LAB_GO_PROXY:-}"
npm_registry="${ASTER_LINUX_LAB_NPM_REGISTRY:-}"
mkdir -p -- \
  "$cache_root/cargo" \
  "$cache_root/go-build" \
  "$cache_root/go-mod" \
  "$cache_root/npm" \
  "$cache_root/home"

docker_root="$(docker_host_path "$root")"
docker_cache_root="$(docker_host_path "$cache_root")"
docker_lab_root="$(docker_host_path "$lab_root")"
git_common_dir="$(git rev-parse --path-format=absolute --git-common-dir)"
git_worktree_dir="$(git rev-parse --path-format=absolute --git-dir)"
git_dir_suffix="${git_worktree_dir#"$git_common_dir"}"
docker_git_common="$(docker_host_path "$git_common_dir")"
git_state_mounts=()
if [[ -f "$root/.git" ]]; then
  # Override only this checkout's pointer. Global GIT_DIR/GIT_WORK_TREE would
  # redirect Git commands in tests' temporary repositories into this worktree.
  printf 'gitdir: /aster-git%s\n' "$git_dir_suffix" > "$lab_root/container.git"
  git_state_mounts+=(--volume "$(docker_host_path "$lab_root/container.git"):/workspace/.git:ro")
fi
toolchain_state_mounts=()
if [[ $windows_docker_host -eq 1 ]]; then
  workspace_id="$(printf '%s' "$root" | sha256sum | awk '{print substr($1, 1, 16)}')"
  node_modules_volume="aster-linux-lab-node-modules-$workspace_id"
  target_volume="aster-linux-lab-target-$workspace_id"
  dist_volume="aster-linux-lab-dist-$workspace_id"
  docker_cli volume create "$node_modules_volume" >/dev/null
  docker_cli volume create "$target_volume" >/dev/null
  docker_cli volume create "$dist_volume" >/dev/null
  toolchain_state_mounts+=(--volume "$node_modules_volume:/workspace/node_modules:rw")
  toolchain_state_mounts+=(--volume "$target_volume:/workspace/target:rw")
  toolchain_state_mounts+=(--volume "$dist_volume:/workspace/dist:rw")
fi

build_toolchain_image() {
  echo "Using prebuilt Linux toolchain: $toolchain_image"
  if ! docker_cli image inspect "$toolchain_image" >/dev/null 2>&1; then
    docker_cli pull --platform linux/amd64 "$toolchain_image"
  fi
}

prepare_toolchain_state() {
  [[ $windows_docker_host -eq 1 ]] || return 0
  docker_cli run --rm --user 0:0 \
    --volume "$node_modules_volume:/workspace/node_modules:rw" \
    --volume "$target_volume:/workspace/target:rw" \
    --volume "$dist_volume:/workspace/dist:rw" \
    "$toolchain_image" sh -c \
    'chown "$1:$2" /workspace/node_modules /workspace/target /workspace/dist' sh "$(id -u)" "$(id -g)"
}

run_toolchain() {
  local optional_env=()
  [[ -z "$cargo_index" ]] || optional_env+=(--env "CARGO_REGISTRIES_CRATES_IO_INDEX=$cargo_index")
  [[ -z "$go_proxy" ]] || optional_env+=(--env "GOPROXY=$go_proxy")
  if [[ -n "$npm_registry" ]]; then
    optional_env+=(--env "npm_config_registry=$npm_registry")
    optional_env+=(--env npm_config_replace_registry_host=always)
  fi
  docker_cli run --rm --interactive \
    --user "$(id -u):$(id -g)" \
    --env HOME=/linux-lab-cache/home \
    --env CARGO_HOME=/linux-lab-cache/cargo \
    --env RUSTUP_HOME=/usr/local/rustup \
    --env GOCACHE=/linux-lab-cache/go-build \
    --env GOMODCACHE=/linux-lab-cache/go-mod \
    --env npm_config_cache=/linux-lab-cache/npm \
    --env GIT_CONFIG_COUNT=1 \
    --env GIT_CONFIG_KEY_0=safe.directory \
    --env GIT_CONFIG_VALUE_0=/workspace \
    "${optional_env[@]}" \
    --volume "$docker_root:/workspace:rw" \
    --volume "$docker_git_common:/aster-git:ro" \
    "${git_state_mounts[@]}" \
    --volume "$docker_cache_root:/linux-lab-cache:rw" \
    --volume "$docker_lab_root:/lab:rw" \
    "${toolchain_state_mounts[@]}" \
    --workdir /workspace \
    "$toolchain_image" "$@"
}

install_node_dependencies() {
  local stamp_dir="$cache_root"
  local stamp="$stamp_dir/package-lock.sha256"
  local expected
  expected="$(sha256sum package-lock.json | awk '{print $1}')"
  if [[ -f "$stamp" && "$(tr -d '\r\n' < "$stamp")" == "$expected" ]] &&
     run_toolchain test -f node_modules/.package-lock.json; then
    echo 'Node dependencies: cached'
    return
  fi
  echo 'Installing locked Node dependencies...'
  run_toolchain npm ci
  mkdir -p -- "$stamp_dir"
  printf '%s\n' "$expected" > "$stamp"
}

build_test_package() {
  local signing_root="$lab_root/signing"
  mkdir -p -- "$signing_root"
  run_toolchain node - /lab/signing <<'NODE'
const { createPrivateKey, createPublicKey, generateKeyPairSync, randomBytes } = require('node:crypto')
const { join } = require('node:path')
const { readFileSync, writeFileSync } = require('node:fs')
const root = process.argv[2]
const seed = randomBytes(32)
const pkcs8 = Buffer.concat([Buffer.from('302e020100300506032b657004220420', 'hex'), seed])
const releasePublic = createPublicKey(createPrivateKey({ key: pkcs8, format: 'der', type: 'pkcs8' }))
  .export({ format: 'der', type: 'spki' }).toString('base64url')
const paidLicense = generateKeyPairSync('ed25519')
const paidLicensePublic = paidLicense.publicKey.export({ format: 'der', type: 'spki' }).toString('base64url')
const paidLicensePrivate = paidLicense.privateKey.export({ format: 'der', type: 'pkcs8' }).toString('base64url')
const v2Trust = JSON.parse(readFileSync('/workspace/contracts/test-vectors/license-trust.v1.json', 'utf8'))
const v2Vectors = JSON.parse(readFileSync('/workspace/contracts/test-vectors/license.v2.json', 'utf8'))
const freeKey = v2Trust.keyring.find(entry => entry.key_id === 'test-only-v2')
const freeCase = v2Vectors.cases.find(entry => entry.name === 'free_no_expiry')
if (!freeKey || !freeCase) throw new Error('test-only bundled free license fixture is missing')
writeFileSync(join(root, 'release.seed'), seed, { mode: 0o600 })
writeFileSync(join(root, 'license-keys.json'), JSON.stringify([
  {
    key_id: 'lab-paid-v2',
    public_key_spki: paidLicensePublic,
    policy: {
      sources: ['commercial_order'], bindings: ['installation'], expiries: ['fixed'],
      entitlement_ceiling: {
        catalog_version: 1,
        features: ['gateway', 'member', 'runner'],
        quotas: [
          { id: 'member_seats', limit: { mode: 'limited', value: 20 } },
          { id: 'runners', limit: { mode: 'unlimited' } },
          { id: 'upstream_accounts', limit: { mode: 'unlimited' } },
          { id: 'api_keys_per_member', limit: { mode: 'unlimited' } },
        ],
      },
    },
  },
  freeKey,
]))
writeFileSync(join(root, 'paid-license-private.pkcs8.base64url'), paidLicensePrivate, { mode: 0o600 })
const paidTrust = JSON.parse(readFileSync(join(root, 'license-keys.json'), 'utf8')).find(entry => entry.key_id === 'lab-paid-v2')
writeFileSync(join(root, 'v2-signers.json'), JSON.stringify([
  { key_id: paidTrust.key_id, private_key_pkcs8: paidLicensePrivate, policy: paidTrust.policy },
]), { mode: 0o600 })
writeFileSync(join(root, 'free-license.json'), JSON.stringify(freeCase.document), { mode: 0o600 })
writeFileSync(join(root, 'release-keys.json'), JSON.stringify([{ key_id: 'lab-release-v1', public_key_spki: releasePublic }]))
writeFileSync(join(root, 'asterctl.exe'), Buffer.from('MZ Aster test-only asterctl fixture'))
NODE

  if [[ -n "$asterctl_windows_x64" ]]; then
    cp -- "$asterctl_windows_x64" "$signing_root/asterctl.exe"
  fi

  run_toolchain bash -Eeuo pipefail -c '
    CGO_ENABLED=0 GOOS=linux GOARCH=amd64 go build -trimpath \
      -o /lab/signing/lablicensesigner ./operations/backend/cmd/lablicensesigner
  '

  run_toolchain bash -Eeuo pipefail -c '
    version="$1"
    export ASTER_LICENSE_TRUSTED_KEYS_JSON="$(</lab/signing/license-keys.json)"
    export ASTER_RELEASE_TRUSTED_KEYS_JSON="$(</lab/signing/release-keys.json)"
    export ASTER_RELEASE_SIGNING_KEY_FILE=/lab/signing/release.seed
    export ASTER_RELEASE_SIGNING_KEY_ID=lab-release-v1
    export ASTER_CUSTOMER_FREE_LICENSE_FILE=/lab/signing/free-license.json
    export ASTER_RELEASE_CREATED_AT="$(node -e '\''process.stdout.write(new Date().toISOString())'\'')"
    export SOURCE_DATE_EPOCH="$(git show -s --format=%ct HEAD)"
    export ASTER_RELEASE_VERSION="$version"
    export ASTER_OVERWRITE=true
    export ASTER_CLIENT_ASTERCTL_WINDOWS_X64=/lab/signing/asterctl.exe
    if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
      echo "Notice: building the current dirty worktree with an ephemeral lab signature."
    fi
    echo "Building test-signed Linux amd64 package for $version..."
    node scripts/build-linux-bundle.mjs --arch=amd64 --version="$version"
    bash scripts/ci/verify-linux-static-runtime.sh \
      --bundle "dist/linux/aster-team-${version}-linux-amd64" \
      --platform-manifest contracts/release-platforms.json \
      --platform linux-amd64
    mkdir -p /lab/output
    cp -- \
      "dist/linux/aster-team-${version}-linux-amd64.tar.gz" \
      "dist/linux/aster-team-${version}-linux-amd64.tar.gz.sha256" \
      /lab/output/
    if [[ "$2" == export-bundle ]]; then
      cp -R -- "dist/linux/aster-team-${version}-linux-amd64" /lab/output/customer-bundle
    fi
  ' bash "$version" "${export_bundle:+export-bundle}"
  mkdir -p -- "$(dirname -- "$archive")"
  cp -- \
    "$lab_root/output/$(basename -- "$archive")" \
    "$lab_root/output/$(basename -- "$checksum")" \
    "$(dirname -- "$archive")/"
  if [[ -n "$export_bundle" ]]; then
    [[ ! -e "$export_bundle" && ! -L "$export_bundle" ]] || { echo 'Bundle export destination already exists.' >&2; exit 1; }
    cp -R -- "$lab_root/output/customer-bundle" "$export_bundle"
  fi

  if [[ -n "$export_v2_signers" ]]; then
    install -m 0600 -- "$signing_root/v2-signers.json" "$export_v2_signers"
  fi
}

if [[ $reuse_package -eq 0 ]]; then
  build_toolchain_image
  prepare_toolchain_state
  run_toolchain bash -Eeuo pipefail -c '
    [[ "$(node --version)" == v22.19.0 ]]
    [[ "$(go version)" == "go version go1.25.14 linux/amd64" ]]
    [[ "$(rustc --version)" == rustc\ 1.95.0* ]]
    rustup target list --installed | grep -qx x86_64-unknown-linux-musl
  '
  install_node_dependencies
  run_toolchain bash -Eeuo pipefail -c \
    'bash -n customer/deploy/init.sh customer/deploy/install.sh customer/deploy/restore-backup.sh scripts/ci/*.sh'
  if [[ "$profile" == full ]]; then
    run_toolchain npm run verify
  else
    run_toolchain npm run test:boundaries
  fi
  build_test_package
else
  echo 'Reusing the existing test package; source changes are not rebuilt.'
fi
[[ -f "$archive" && -f "$checksum" ]] || {
  echo "Linux package is unavailable: $archive" >&2
  exit 1
}
(
  cd -- "$(dirname -- "$archive")"
  sha256sum --check --strict "$(basename -- "$checksum")"
)

case_file="$lab_root/cases.tsv"
if [[ "$profile" == quick ]]; then
  printf '%s\t%s\t%s\t%s\n' \
    'ubuntu-20.04' 'ubuntu:20.04' 'apt' 'customer' \
    'ubuntu-20.04' 'ubuntu:20.04' 'apt' 'runner' > "$case_file"
else
  node --input-type=module - > "$case_file" <<'NODE'
import { readFileSync } from 'node:fs'
const manifest = JSON.parse(readFileSync('contracts/release-platforms.json', 'utf8'))
const platform = manifest.platforms.find(candidate => candidate.id === 'linux-amd64')
if (!platform) throw new Error('linux-amd64 release platform is missing')
for (const target of platform.smoke_targets) {
  const image = target.image || `ubuntu:${target.id.replace(/^ubuntu-/, '')}`
  const family = target.package_family || 'apt'
  for (const role of ['customer', 'runner']) {
    process.stdout.write(`${target.id}\t${image}\t${family}\t${role}\n`)
  }
}
NODE
fi

run_case() {
  local target="$1" image="$2" package_family="$3" role="$4"
  local label="${target}-${role}"
  local package_copy="$lab_root/packages/$label"
  local log="$log_root/$label.log"
  local expected_license_args=()
  if [[ "$role" == customer ]]; then
    expected_license_args=(--expected-bundled-license-id test_free_no_expiry)
  fi
  mkdir -p -- "$package_copy"
  cp -- "$archive" "$checksum" "$package_copy/"
  if [[ "$role" == customer && $reuse_package -eq 0 ]]; then
    cp -- "$lab_root/signing/lablicensesigner" "$lab_root/signing/paid-license-private.pkcs8.base64url" "$package_copy/"
    expected_license_args+=(--exercise-paid-license-switch)
  fi
  echo "[$label] starting ($image)"
  SMOKE_DIAGNOSTIC_ROOT="$diagnostic_root" \
  GITHUB_RUN_ID="$run_id" \
  GITHUB_JOB="$label" \
  GITHUB_RUN_ATTEMPT="${run_id}-${label}" \
    bash scripts/ci/exercise-supported-linux-install.sh \
      --target "$target" \
      --mode container \
      --role "$role" \
      --package-dir "$package_copy" \
      --version "$version" \
      --image "$image" \
      --package-family "$package_family" \
      "${expected_license_args[@]}" > "$log" 2>&1
}

active_pids=()
active_labels=()
failures=0
wait_batch() {
  local index pid label log
  for index in "${!active_pids[@]}"; do
    pid="${active_pids[$index]}"
    label="${active_labels[$index]}"
    log="$log_root/$label.log"
    if wait "$pid"; then
      echo "[$label] passed"
    else
      echo "[$label] failed; recent output follows:" >&2
      tail -n 120 "$log" >&2 || true
      failures=$((failures + 1))
    fi
  done
  active_pids=()
  active_labels=()
}

while IFS=$'\t' read -r target image package_family role; do
  [[ -n "$target" ]] || continue
  run_case "$target" "$image" "$package_family" "$role" &
  active_pids+=("$!")
  active_labels+=("${target}-${role}")
  if [[ ${#active_pids[@]} -ge $jobs ]]; then
    wait_batch
  fi
done < "$case_file"
[[ ${#active_pids[@]} -eq 0 ]] || wait_batch

if [[ $failures -ne 0 ]]; then
  echo "Result: $failures Linux lab case(s) failed." >&2
  echo "Logs: $log_root" >&2
  exit 1
fi

echo "Result: Aster Linux lab $profile profile passed."
echo "Package: $archive"
echo "Logs:    $log_root"
