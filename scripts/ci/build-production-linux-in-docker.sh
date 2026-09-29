#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

usage() {
  cat <<'EOF'
Usage:
  bash scripts/ci/build-production-linux-in-docker.sh --version VERSION \
    --security-directory DIRECTORY --release-signing-key-id KEY_ID \
    --asterctl-windows-x64 FILE \
    [--output-root REPOSITORY_RELATIVE_DIRECTORY] [--preflight-only]

Runs the canonical production Linux amd64 builder inside the repository's
pinned Docker toolchain. The external signing directory is mounted read-only.
EOF
}

version=''
security_directory=''
release_signing_key_id=''
asterctl_windows_x64=''
output_root='dist'
preflight_only=0
while (($#)); do
  case "$1" in
    --version) version="${2:-}"; shift 2 ;;
    --security-directory) security_directory="${2:-}"; shift 2 ;;
    --release-signing-key-id) release_signing_key_id="${2:-}"; shift 2 ;;
    --asterctl-windows-x64) asterctl_windows_x64="${2:-}"; shift 2 ;;
    --output-root) output_root="${2:-}"; shift 2 ;;
    --preflight-only) preflight_only=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

version="${version#v}"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
  echo '--version is invalid.' >&2
  exit 2
}
[[ "$release_signing_key_id" =~ ^[A-Za-z0-9_.:-]{3,128}$ ]] || {
  echo '--release-signing-key-id is invalid.' >&2
  exit 2
}
[[ -n "$output_root" && "$output_root" != /* && "$output_root" != *\\* && "$output_root" != '..' && "$output_root" != ../* && "$output_root" != */../* && "$output_root" != */.. ]] || {
  echo '--output-root must be a repository-relative directory without traversal.' >&2
  exit 2
}

root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
host_system="$(uname -s)"
windows_docker_host=0
case "$host_system" in
  Linux) ;;
  Darwin) ;;
  MINGW*|MSYS*|CYGWIN*)
    windows_docker_host=1
    command -v cygpath >/dev/null 2>&1 || {
      echo 'Git Bash with cygpath is required on Windows.' >&2
      exit 1
    }
    security_directory="$(cygpath --unix "$security_directory")"
    ;;
  *) echo "Unsupported local release host: $host_system" >&2; exit 1 ;;
esac
host_architecture="$(uname -m)"
if [[ "$host_system" == Darwin ]]; then
  [[ "$host_architecture" == x86_64 || "$host_architecture" == arm64 ]] || {
    echo 'The Linux amd64 release container requires an x86_64 or Apple Silicon Mac.' >&2
    exit 1
  }
else
  [[ "$host_architecture" == x86_64 ]] || {
    echo 'The Linux amd64 release container requires an x86_64 host.' >&2
    exit 1
  }
fi
security_directory="$(cd -- "$security_directory" && pwd -P)"
for file in \
  license-v2.public-keyring.json \
  free-license.json \
  release-v1.public-keyring.json \
  release-v1.seed \
  plugin-v1.public-keyring.json \
  plugin-v1.seed; do
  [[ -f "$security_directory/$file" && ! -L "$security_directory/$file" ]] || {
    echo "Required release input is unavailable: $file" >&2
    exit 1
  }
done

command -v docker >/dev/null 2>&1 || {
  echo 'Docker is required on PATH. The local release entrypoint discovers Docker Desktop and configures its child PATH.' >&2
  exit 1
}
docker_command=(docker)
docker_cli() {
  if [[ $windows_docker_host -eq 1 ]]; then
    MSYS_NO_PATHCONV=1 "${docker_command[@]}" "$@"
  else
    "${docker_command[@]}" "$@"
  fi
}
host_sha256() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum
  else
    shasum -a 256
  fi
}
docker_host_path() {
  if [[ $windows_docker_host -eq 1 ]]; then
    cygpath --absolute --mixed "$1"
  else
    printf '%s\n' "$1"
  fi
}
docker_cli info >/dev/null 2>&1 || {
  echo 'Docker is installed, but its daemon is unavailable to the current user.' >&2
  exit 1
}

toolchain_image="$(node "$root/scripts/ci/ci-images.mjs" linux-builder)"
if [[ $preflight_only -eq 1 ]]; then
  echo 'Validating access to the prebuilt Linux release compiler...'
  docker_cli pull --platform linux/amd64 "$toolchain_image" >/dev/null
  echo 'Linux Docker release preflight passed.'
  exit 0
fi
[[ -n "$asterctl_windows_x64" ]] || { echo '--asterctl-windows-x64 is required.' >&2; exit 2; }
if [[ $windows_docker_host -eq 1 ]]; then
  asterctl_windows_x64="$(cygpath --unix "$asterctl_windows_x64")"
fi
asterctl_windows_x64="$(cd -- "$(dirname -- "$asterctl_windows_x64")" && pwd -P)/$(basename -- "$asterctl_windows_x64")"
[[ -f "$asterctl_windows_x64" && ! -L "$asterctl_windows_x64" ]] || { echo 'asterctl Windows x64 payload is unavailable.' >&2; exit 1; }
[[ "$(head -c 2 "$asterctl_windows_x64")" == MZ ]] || { echo 'asterctl Windows x64 payload is invalid.' >&2; exit 1; }
echo "Using prebuilt Linux release toolchain: $toolchain_image"
if ! docker_cli image inspect "$toolchain_image" >/dev/null 2>&1; then
  docker_cli pull --platform linux/amd64 "$toolchain_image"
fi

repository_identity="$(git -C "$root" config --get remote.origin.url || basename "$root")"
workspace_id="$(printf '%s\n%s' "$repository_identity" "$toolchain_image" | host_sha256 | awk '{print substr($1, 1, 16)}')"
node_modules_volume="aster-local-release-node-modules-$workspace_id"
target_volume="aster-local-release-target-$workspace_id"
cache_volume="aster-local-release-cache-$workspace_id"
# npm ci also cleans workspace-local dependencies; keep those writes off the host.
workspace_paths="$(node "$root/scripts/ci/npm-workspace-dependencies.mjs" "$root/package.json")"
workspace_dependency_volumes=()
workspace_dependency_mounts=()
workspace_dependency_directories=()
while IFS= read -r workspace_path; do
  workspace_path_id="$(printf '%s' "$workspace_path" | host_sha256 | awk '{print substr($1, 1, 16)}')"
  workspace_volume="${node_modules_volume}-workspace-${workspace_path_id}"
  workspace_directory="/workspace/$workspace_path/node_modules"
  workspace_dependency_volumes+=("$workspace_volume")
  workspace_dependency_mounts+=(--mount "type=volume,src=$workspace_volume,dst=$workspace_directory,volume-nocopy")
  workspace_dependency_directories+=("$workspace_directory")
done <<< "$workspace_paths"
for volume in "$node_modules_volume" "$target_volume" "$cache_volume" "${workspace_dependency_volumes[@]}"; do
  docker_cli volume create "$volume" >/dev/null
done
host_uid="$(id -u)"
host_gid="$(id -g)"
docker_cli run --rm --platform linux/amd64 --user 0:0 \
  --volume "$node_modules_volume:/workspace/node_modules:rw" \
  "${workspace_dependency_mounts[@]}" \
  --volume "$target_volume:/workspace/target:rw" \
  --volume "$cache_volume:/release-cache:rw" \
  "$toolchain_image" sh -c \
  'dependency_owner="$1:$2"; shift 2; chown -R "$dependency_owner" "$@"' sh "$host_uid" "$host_gid" \
  /workspace/node_modules /workspace/target /release-cache "${workspace_dependency_directories[@]}"

docker_root="$(docker_host_path "$root")"
docker_security_directory="$(docker_host_path "$security_directory")"
docker_asterctl_windows_x64="$(docker_host_path "$asterctl_windows_x64")"
output_directory="$root/$output_root"
mkdir -p "$output_directory"
docker_output_directory="$(docker_host_path "$output_directory")"
git_config_count=1
git_filemode_environment=()
if [[ $windows_docker_host -eq 1 ]]; then
  git_config_count=3
  git_filemode_environment=(
    --env GIT_CONFIG_KEY_1=core.filemode
    --env GIT_CONFIG_VALUE_1=false
    --env GIT_CONFIG_KEY_2=core.autocrlf
    --env GIT_CONFIG_VALUE_2=true
  )
fi
docker_cli run --rm --interactive --platform linux/amd64 \
  --user "$host_uid:$host_gid" \
  --env HOME=/release-cache/home \
  --env CARGO_HOME=/release-cache/cargo \
  --env RUSTUP_HOME=/usr/local/rustup \
  --env GOCACHE=/release-cache/go-build \
  --env GOMODCACHE=/release-cache/go-mod \
  --env npm_config_cache=/release-cache/npm \
  --env ASTER_RELEASE_DOWNLOAD_CACHE=/release-cache/downloads \
  --env "ASTER_RELEASE_OUTPUT_ROOT=/workspace/$output_root" \
  --env "GIT_CONFIG_COUNT=$git_config_count" \
  --env GIT_CONFIG_KEY_0=safe.directory \
  --env GIT_CONFIG_VALUE_0=/workspace \
  "${git_filemode_environment[@]}" \
  --volume "$docker_root:/workspace:rw" \
  --volume "$docker_security_directory:/release-security:ro" \
  --volume "$docker_asterctl_windows_x64:/client-tools/asterctl-windows-x86_64.exe:ro" \
  --volume "$node_modules_volume:/workspace/node_modules:rw" \
  "${workspace_dependency_mounts[@]}" \
  --volume "$target_volume:/workspace/target:rw" \
  --volume "$docker_output_directory:/workspace/$output_root:rw" \
  --volume "$cache_volume:/release-cache:rw" \
  --workdir /workspace \
  "$toolchain_image" bash -Eeuo pipefail -c '
    mkdir -p "$HOME" "$CARGO_HOME" "$GOCACHE" "$GOMODCACHE" "$npm_config_cache"
    npm ci
    bash scripts/build-linux-amd64.sh --version "$1" \
      --license-keyring /release-security/license-v2.public-keyring.json \
      --release-keyring /release-security/release-v1.public-keyring.json \
      --release-signing-key /release-security/release-v1.seed \
      --release-signing-key-id "$2" \
      --plugin-keyring /release-security/plugin-v1.public-keyring.json \
      --plugin-signing-key /release-security/plugin-v1.seed \
      --plugin-signing-key-id "$3" \
      --free-license /release-security/free-license.json \
      --asterctl-windows-x64 /client-tools/asterctl-windows-x86_64.exe
  ' bash "$version" "$release_signing_key_id" "$(node -e 'const fs = require("node:fs"); console.log(JSON.parse(fs.readFileSync(process.argv[1], "utf8"))[0].key_id)' "$security_directory/plugin-v1.public-keyring.json")"
