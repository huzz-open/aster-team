#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  echo 'Usage: verify-linux-static-runtime.sh --bundle PATH --platform-manifest PATH --platform ID' >&2
}

bundle=''
platform_manifest=''
platform=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle) bundle=${2-}; shift 2 ;;
    --platform-manifest) platform_manifest=${2-}; shift 2 ;;
    --platform) platform=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -n "$bundle" && -d "$bundle/bin" ]] || { echo 'A bundle with bin/ is required.' >&2; exit 2; }
[[ -f "$platform_manifest" && -n "$platform" ]] || {
  echo 'Platform manifest and platform ID are required.' >&2
  exit 2
}

manifest_runtime="$(node -e '
  const fs = require("node:fs")
  const [manifestPath, id] = process.argv.slice(1)
  const manifest = JSON.parse(fs.readFileSync(manifestPath, "utf8"))
  const platform = manifest.platforms?.find((entry) => entry.id === id)
  if (platform?.rust_target !== "x86_64-unknown-linux-musl" || platform?.runtime !== "musl-static") process.exit(2)
  process.stdout.write(platform.runtime)
' "$platform_manifest" "$platform")" || {
  echo "Platform is not configured as x86_64-unknown-linux-musl with musl-static runtime: $platform" >&2
  exit 2
}

node "$(dirname -- "${BASH_SOURCE[0]}")/verify-static-elf.mjs" "$bundle"
command -v readelf >/dev/null || { echo 'readelf is required for static runtime validation.' >&2; exit 2; }
for binary in "$bundle/bin/aster-team-cli" "$bundle/bin/aster-control" "$bundle/bin/aster-runner" "$bundle/bin/caddy"; do
  [[ -x "$binary" ]] || { echo "Expected executable is missing: $binary" >&2; exit 1; }
  readelf -h "$binary" | grep -q 'Machine:.*Advanced Micro Devices X86-64' || {
    echo "Unexpected ELF architecture: $binary" >&2
    exit 1
  }
  if readelf -l "$binary" | grep -q 'INTERP'; then
    echo "Customer executable has a dynamic interpreter: $binary" >&2
    exit 1
  fi
  if readelf -d "$binary" 2>/dev/null | grep -q '(NEEDED)'; then
    echo "Customer executable has a dynamic library dependency: $binary" >&2
    exit 1
  fi
  if readelf --version-info "$binary" 2>/dev/null | grep -q 'GLIBC_'; then
    echo "Customer executable still imports GLIBC symbols: $binary" >&2
    exit 1
  fi
done

echo "Linux runtime verified: $manifest_runtime with no ELF interpreter or dynamic library dependencies."
