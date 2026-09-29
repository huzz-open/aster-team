#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

usage() {
  cat <<'EOF'
Usage:
  bash scripts/build-linux-amd64.sh --version VERSION \
    --license-keyring FILE --release-keyring FILE \
    --release-signing-key FILE --release-signing-key-id KEY_ID \
    --plugin-keyring FILE --plugin-signing-key FILE --plugin-signing-key-id KEY_ID \
    --asterctl-windows-x64 FILE [options]

Builds the production Linux amd64 package. Customer runtime contains only the
Rust Control/Runner binaries and static Admin/Member assets.

Required key inputs:
  --license-keyring FILE       JSON array of trusted license public keys.
  --release-keyring FILE       JSON array of trusted release public keys.
  --release-signing-key FILE   Raw 32-byte Ed25519 release signing seed.
  --release-signing-key-id ID  key_id present in the release public keyring.
  --plugin-keyring FILE        JSON array of trusted official plugin publishers.
  --plugin-signing-key FILE    Raw 32-byte Ed25519 plugin signing seed.
  --plugin-signing-key-id ID   key_id present in the plugin public keyring.
  --asterctl-windows-x64 FILE  Native Windows x64 asterctl.exe client payload.

Options:
  --free-license FILE          Signed unbound v2 free-distribution license to bundle.
  --created-at TIME            Exact UTC release time. Defaults to source commit time.
  --allow-non-main             Permit a detached/tag or non-main CI build.
  --overwrite                  Replace this version's existing local artifacts.
  -h, --help                   Show this help.

The license signing private key is deliberately not accepted by this build.
The release signing private key must be stored outside the repository.
EOF
}

version=''
license_keyring=''
release_keyring=''
release_signing_key=''
release_signing_key_id=''
plugin_keyring=''
plugin_signing_key=''
plugin_signing_key_id=''
asterctl_windows_x64=''
free_license=''
created_at=''
allow_non_main=0
overwrite=0

while (($#)); do
  case "$1" in
    --version) version="${2:-}"; shift 2 ;;
    --license-keyring) license_keyring="${2:-}"; shift 2 ;;
    --release-keyring) release_keyring="${2:-}"; shift 2 ;;
    --release-signing-key) release_signing_key="${2:-}"; shift 2 ;;
    --release-signing-key-id) release_signing_key_id="${2:-}"; shift 2 ;;
    --plugin-keyring) plugin_keyring="${2:-}"; shift 2 ;;
    --plugin-signing-key) plugin_signing_key="${2:-}"; shift 2 ;;
    --plugin-signing-key-id) plugin_signing_key_id="${2:-}"; shift 2 ;;
    --asterctl-windows-x64) asterctl_windows_x64="${2:-}"; shift 2 ;;
    --free-license) free_license="${2:-}"; shift 2 ;;
    --created-at) created_at="${2:-}"; shift 2 ;;
    --allow-non-main) allow_non_main=1; shift ;;
    --overwrite) overwrite=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

version="${version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { echo '--version is invalid.' >&2; exit 2; }
[[ "$release_signing_key_id" =~ ^[A-Za-z0-9_.:-]{3,128}$ ]] || { echo '--release-signing-key-id is invalid.' >&2; exit 2; }

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
cd "$root"
for command in git node npm cargo rustc rustup musl-gcc curl tar; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing build dependency: $command" >&2; exit 1; }
done
rustup target list --installed | grep -qx 'x86_64-unknown-linux-musl' || {
  echo 'Missing Rust target: run rustup target add x86_64-unknown-linux-musl.' >&2
  exit 1
}
node --input-type=module - "$version" <<'NODE'
import { readFileSync } from 'node:fs'

const expected = process.argv[2]
const packageVersion = JSON.parse(readFileSync('package.json', 'utf8')).version
const cargoManifest = readFileSync('Cargo.toml', 'utf8')
const workspacePackage = cargoManifest.match(/\[workspace\.package\]([\s\S]*?)(?:\n\[|$)/)?.[1] || ''
const cargoVersion = workspacePackage.match(/^version\s*=\s*"([^"]+)"/m)?.[1]
if (packageVersion !== expected || cargoVersion !== expected) {
  throw new Error(`Release version ${expected} does not match package.json (${packageVersion}) and Cargo.toml (${cargoVersion || 'missing'})`)
}
NODE
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || { echo 'Build inside the pinned Linux amd64 environment.' >&2; exit 1; }
for file in "$license_keyring" "$release_keyring" "$release_signing_key" "$plugin_keyring" "$plugin_signing_key" "$asterctl_windows_x64"; do
  [[ -f "$file" ]] || { echo "Required key input is unavailable: $file" >&2; exit 1; }
done
[[ ! -L "$asterctl_windows_x64" ]] || { echo 'asterctl client payload must not be a symbolic link.' >&2; exit 1; }
[[ "$(head -c 2 "$asterctl_windows_x64")" == MZ ]] || { echo 'asterctl client payload is not a Windows executable.' >&2; exit 1; }
asterctl_size="$(wc -c < "$asterctl_windows_x64" | tr -d ' ')"
[[ "$asterctl_size" -gt 0 && "$asterctl_size" -le 67108864 ]] || { echo 'asterctl client payload has an invalid size.' >&2; exit 1; }
[[ "$(wc -c < "$release_signing_key" | tr -d ' ')" == 32 ]] || { echo 'Release signing key must contain exactly 32 raw bytes.' >&2; exit 1; }
if [[ -n "$free_license" ]]; then
  [[ -f "$free_license" && ! -L "$free_license" ]] || { echo 'Free license must be an ordinary file.' >&2; exit 1; }
  free_license_size="$(wc -c < "$free_license" | tr -d ' ')"
  [[ "$free_license_size" -gt 0 && "$free_license_size" -le 65536 ]] || { echo 'Free license must be between 1 byte and 64 KiB.' >&2; exit 1; }
fi
if git ls-files --error-unmatch -- "$release_signing_key" >/dev/null 2>&1; then
  echo 'Release signing private key must not be tracked in the source repository.' >&2
  exit 1
fi

if [[ -n "$(git status --porcelain --untracked-files=all)" ]]; then
  echo 'Refusing a production build from a dirty worktree.' >&2
  exit 1
fi
if [[ $allow_non_main -ne 1 ]]; then
  branch="$(git branch --show-current)"
  [[ "$branch" == main ]] || { echo "Production builds require main; current branch is '${branch:-detached HEAD}'." >&2; exit 1; }
  if git show-ref --verify --quiet refs/remotes/origin/main; then
    [[ "$(git rev-parse HEAD)" == "$(git rev-parse refs/remotes/origin/main)" ]] || {
      echo 'Local main does not match origin/main.' >&2
      exit 1
    }
  fi
fi

source_date_epoch="$(git show -s --format=%ct HEAD)"
if [[ -z "$created_at" ]]; then
  created_at="$(node -e 'process.stdout.write(new Date(Number(process.argv[1]) * 1000).toISOString())' "$source_date_epoch")"
fi
[[ "$created_at" =~ ^[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}\.[0-9]{3}Z$ ]] || { echo '--created-at is invalid.' >&2; exit 2; }

export ASTER_RELEASE_VERSION="$version"
export ASTER_LICENSE_TRUSTED_KEYS_JSON="$(<"$license_keyring")"
export ASTER_RELEASE_TRUSTED_KEYS_JSON="$(<"$release_keyring")"
export ASTER_PLUGIN_TRUSTED_KEYS_JSON="$(<"$plugin_keyring")"
export ASTER_PLUGIN_SIGNING_KEY_FILE="$(cd -- "$(dirname -- "$plugin_signing_key")" && pwd -P)/$(basename -- "$plugin_signing_key")"
export ASTER_PLUGIN_SIGNING_KEY_ID="$plugin_signing_key_id"
export ASTER_RELEASE_SIGNING_KEY_FILE="$(cd -- "$(dirname -- "$release_signing_key")" && pwd -P)/$(basename -- "$release_signing_key")"
export ASTER_RELEASE_SIGNING_KEY_ID="$release_signing_key_id"
export ASTER_RELEASE_CREATED_AT="$created_at"
export SOURCE_DATE_EPOCH="$source_date_epoch"
export ASTER_CLIENT_ASTERCTL_WINDOWS_X64="$(cd -- "$(dirname -- "$asterctl_windows_x64")" && pwd -P)/$(basename -- "$asterctl_windows_x64")"
if [[ -n "$free_license" ]]; then
  export ASTER_CUSTOMER_FREE_LICENSE_FILE="$(cd -- "$(dirname -- "$free_license")" && pwd -P)/$(basename -- "$free_license")"
fi
if [[ $overwrite -eq 1 ]]; then export ASTER_OVERWRITE=true; fi

node scripts/build-linux-bundle.mjs --arch=amd64 --version="$version"
