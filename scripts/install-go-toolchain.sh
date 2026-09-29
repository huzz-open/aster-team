#!/usr/bin/env bash
set -Eeuo pipefail

assume_yes=0
while (($#)); do
  case "$1" in
    --yes) assume_yes=1; shift ;;
    -h|--help)
      echo 'Usage: bash scripts/install-go-toolchain.sh [--yes]'
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 2 ;;
  esac
done

version="${ASTER_GO_VERSION:-1.25.14}"
download_base="${ASTER_GO_DOWNLOAD_BASE:-https://golang.google.cn/dl}"
invocation_dir="${INIT_CWD:-$PWD}"
invocation_dir="$(cd "$invocation_dir" && pwd -P)"
install_dir="${ASTER_GO_INSTALL_DIR:-$invocation_dir/.aster-tools/go}"
case "$install_dir" in
  /*) ;;
  *) install_dir="$invocation_dir/$install_dir" ;;
esac

kernel="$(uname -s)"
machine="$(uname -m)"
case "$kernel" in
  Linux*) os=linux; extension=tar.gz ;;
  Darwin*) os=darwin; extension=tar.gz ;;
  MINGW*|MSYS*|CYGWIN*) os=windows; extension=zip ;;
  *) echo "Unsupported operating system: $kernel" >&2; exit 1 ;;
esac
case "$machine" in
  x86_64|amd64) arch=amd64 ;;
  arm64|aarch64) arch=arm64 ;;
  *) echo "Unsupported CPU architecture: $machine" >&2; exit 1 ;;
esac

filename="go${version}.${os}-${arch}.${extension}"
go_binary="$install_dir/bin/go"
if [[ "$os" == windows ]]; then go_binary="${go_binary}.exe"; fi
if [[ -x "$go_binary" ]] && "$go_binary" version 2>/dev/null | grep -q "go${version}"; then
  echo "Go ${version} is already installed at: $install_dir"
  exit 0
fi

echo
echo "Aster Team local Go toolchain"
echo "  Version:  ${version}"
echo "  Platform: ${os}/${arch}"
echo "  Source:   ${download_base}/${filename}"
echo "  Install:  ${install_dir}"
if [[ -e "$install_dir" ]]; then
  echo "  Existing: the current directory will be moved aside before installation"
fi
echo
if [[ $assume_yes -eq 0 ]]; then
  if [[ ! -t 0 ]]; then
    echo 'Interactive confirmation is required; run npm run setup in a terminal or pass --yes.' >&2
    exit 1
  fi
  read -r -p 'Download and install this toolchain? [y/N] ' answer
  case "$answer" in
    y|Y|yes|YES) ;;
    *) echo 'Installation cancelled.'; exit 0 ;;
  esac
fi

for command in curl node; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required command: $command" >&2; exit 1; }
done
if [[ "$extension" == tar.gz ]]; then
  command -v tar >/dev/null 2>&1 || { echo 'Missing required command: tar' >&2; exit 1; }
else
  command -v unzip >/dev/null 2>&1 || { echo 'Missing required command: unzip' >&2; exit 1; }
fi

tools_dir="$(dirname "$install_dir")"
mkdir -p "$tools_dir"
temporary_dir="$(mktemp -d "$tools_dir/.go-install.XXXXXX")"
cleanup() {
  case "$temporary_dir" in
    "$tools_dir"/.go-install.*) rm -rf -- "$temporary_dir" ;;
  esac
}
trap cleanup EXIT

metadata="$temporary_dir/releases.json"
archive="$temporary_dir/$filename"
echo 'Reading the official Go release manifest...'
curl --fail --location --silent --show-error --retry 3 \
  "${download_base}/?mode=json&include=all" --output "$metadata"
expected_sha256="$(node - "$metadata" "go${version}" "$filename" <<'NODE'
const [metadataPath, version, filename] = process.argv.slice(2)
const releases = JSON.parse(require('node:fs').readFileSync(metadataPath, 'utf8'))
const release = releases.find(entry => entry.version === version && entry.stable === true)
const file = release?.files?.find(entry => entry.filename === filename && entry.kind === 'archive')
if (!file?.sha256) process.exit(1)
process.stdout.write(file.sha256)
NODE
)" || { echo "Official release manifest does not contain $filename" >&2; exit 1; }

echo "Downloading $filename..."
curl --fail --location --silent --show-error --retry 3 \
  "${download_base}/${filename}" --output "$archive"
actual_sha256="$(node - "$archive" <<'NODE'
const {createHash} = require('node:crypto')
const {readFileSync} = require('node:fs')
process.stdout.write(createHash('sha256').update(readFileSync(process.argv[2])).digest('hex'))
NODE
)"
if [[ "$actual_sha256" != "$expected_sha256" ]]; then
  echo 'Downloaded Go archive failed SHA-256 verification.' >&2
  exit 1
fi

extract_dir="$temporary_dir/extracted"
mkdir -p "$extract_dir"
if [[ "$extension" == tar.gz ]]; then
  tar -xzf "$archive" -C "$extract_dir"
else
  unzip -q "$archive" -d "$extract_dir"
fi
[[ -d "$extract_dir/go" ]] || { echo 'Downloaded Go archive has an unexpected layout.' >&2; exit 1; }

if [[ -e "$install_dir" ]]; then
  backup_dir="${install_dir}.backup.$(date +%Y%m%d%H%M%S)"
  mv -- "$install_dir" "$backup_dir"
  echo "Previous local toolchain moved to: $backup_dir"
fi
mv -- "$extract_dir/go" "$install_dir"

go_binary="$install_dir/bin/go"
if [[ "$os" == windows ]]; then go_binary="${go_binary}.exe"; fi
"$go_binary" version
echo
echo "Go ${version} installed successfully."
echo "  Directory: $install_dir"
echo '  Aster Team npm commands will use this local toolchain automatically.'
