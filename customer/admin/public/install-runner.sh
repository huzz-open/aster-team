#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

fail() {
  printf 'Aster Team Runner install: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: curl -fsSL <script-url> | sudo bash -s -- --control-url URL --token TOKEN [options]

Options:
  --control-url URL       Public URL of the Aster Team Control
  --token TOKEN           One-time Runner enrollment token
  --allow-insecure-http   Allow an HTTP Control URL
EOF
}

control_url=''
enrollment_token=''
allow_insecure_http=0
while (($#)); do
  case "$1" in
    --control-url|--token|--version)
      (($# >= 2)) || fail "Missing value for $1"
      case "$1" in
        --control-url) control_url="$2" ;;
        --token) enrollment_token="$2" ;;
        --version) : ;; # Accepted for commands copied before Control became the package source.
      esac
      shift 2
      ;;
    --allow-insecure-http) allow_insecure_http=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) fail "Unknown argument: $1" ;;
  esac
done

[[ ${EUID} -eq 0 ]] || fail 'Run this command with sudo.'
[[ "$control_url" =~ ^https://[^[:space:]]+$ || ( $allow_insecure_http -eq 1 && "$control_url" =~ ^http://[^[:space:]]+$ ) ]] || fail 'Control URL must use HTTPS, or HTTP with --allow-insecure-http.'
[[ -n "$enrollment_token" && "$enrollment_token" != *[[:space:]]* ]] || fail 'Enrollment token is invalid.'

[[ "$(uname -s):$(uname -m)" == Linux:x86_64 ]] || fail 'This installer supports Linux x86-64.'
platform=linux
architecture=amd64
initializer=init.sh
invocation_root="$(pwd -P)" || fail 'Could not determine the command directory.'
if [[ "$invocation_root" == / ]]; then install_root=/aster-team
else install_root="${invocation_root%/}/aster-team"
fi

for command in curl find mktemp rmdir tar tee wc; do
  command -v "$command" >/dev/null 2>&1 || fail "Missing required command: $command"
done
command -v sha256sum >/dev/null 2>&1 || fail 'Missing required command: sha256sum'
[[ -r /dev/tty && -w /dev/tty ]] || fail 'Run this command in an interactive terminal.'

printf 'Default Runner install directory: %s\n' "$install_root" >/dev/tty
while true; do
  read -r -p 'Press Enter to use the default, or enter another existing root directory: ' selection </dev/tty
  case "$selection" in
    ''|y|Y|yes|YES|Yes) candidate="$install_root" ;;
    /*)
      if [[ ! -d "$selection" ]]; then
        printf 'The selected root directory does not exist: %s\n' "$selection" >/dev/tty
        continue
      fi
      if [[ "$selection" == / ]]; then candidate=/aster-team
      else candidate="${selection%/}/aster-team"
      fi
      ;;
    *) printf 'Enter Y or an existing root directory using an absolute path.\n' >/dev/tty; continue ;;
  esac
  repairable=0
  if [[ -e "$candidate" ]]; then
    if [[ -d "$candidate" && -f "$candidate/install.json" && ! -e "$candidate/current" && ! -L "$candidate/current" && ! -e "$candidate/config/control/install-role" && ! -e "$candidate/config/runner/install-role" ]]; then
      repairable=1
    fi
    if [[ ! -d "$candidate" || ( -n "$(find "$candidate" -mindepth 1 -maxdepth 1 -print -quit)" && $repairable -eq 0 ) ]]; then
      printf 'The Runner install directory is already occupied: %s\n' "$candidate" >/dev/tty
      continue
    fi
    [[ $repairable -eq 0 ]] || printf 'Continuing the incomplete Runner installation in: %s\n' "$candidate" >/dev/tty
  fi
  install_root="$candidate"
  printf 'Runner install directory: %s\n' "$install_root" >/dev/tty
  break
done
[[ ! -e "$install_root/current" && ! -L "$install_root/current" ]] || fail 'An Aster Team installation already exists on this host.'
if [[ -f "$install_root/install.json" && ! -e "$install_root/config/control/install-role" && ! -e "$install_root/config/runner/install-role" ]]; then
  for relative in \
    data/plugins/incoming data/plugins/versions data/plugins \
    state/upgrades/queued state/upgrades/running state/upgrades/completed state/upgrades \
    config/control config/caddy config/keys config/license config/tls \
    data/database data/caddy data/settlements state/migrations state/slots; do
    rmdir -- "$install_root/$relative" 2>/dev/null || true
  done
fi

mkdir -p -- "$install_root/logs"
install_log="$install_root/logs/install-runner.log"
exec > >(tee -a "$install_log") 2>&1
trap 'status=$?; printf "Runner installation failed. Install log: %s\n" "$install_log" >&2; exit "$status"' ERR
printf 'Install log: %s\n' "$install_log"

temporary_directory="$(mktemp -d -t aster-team-runner.XXXXXXXX)" || fail 'Could not create a temporary directory.'
token_file="$temporary_directory/enrollment-token"
cleanup() { rm -rf -- "$temporary_directory"; }
trap cleanup EXIT

package_url="${control_url%/}/api/runner/install-package/${platform}/${architecture}"
checksum_file="$temporary_directory/package.sha256"
curl_protocol='=https'
if [[ $allow_insecure_http -eq 1 ]]; then curl_protocol='=http,https'; fi
curl --fail --silent --show-error --proto "$curl_protocol" \
  --connect-timeout 15 --max-time 60 --max-filesize 1024 --retry 3 \
  --header "Authorization: Bearer $enrollment_token" \
  --output "$checksum_file" "${package_url}/checksum" || fail 'Could not read the Runner package checksum from Control.'
[[ "$(wc -c < "$checksum_file")" -le 1024 ]] || fail 'The Runner package checksum is too large.'
checksum_line="$(tr -d '\r\n' < "$checksum_file")"
expected_sha256="${checksum_line%%  *}"
archive_name="${checksum_line#*  }"
[[ "$expected_sha256" =~ ^[a-f0-9]{64}$ && "$archive_name" =~ ^aster-team-runner-([0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?)-${platform}-${architecture}\.tar\.gz$ ]] \
  || fail 'Control returned an invalid Runner package checksum.'
version="${BASH_REMATCH[1]}"

archive="$temporary_directory/$archive_name"
printf 'Downloading Aster Team Runner %s for %s %s...\n' "$version" "$platform" "$architecture"
curl --fail --silent --show-error --proto "$curl_protocol" \
  --connect-timeout 15 --max-time 900 --max-filesize 1073741824 --retry 3 \
  --header "Authorization: Bearer $enrollment_token" \
  --output "$archive" "$package_url"
actual_size="$(wc -c < "$archive")"
[[ "$actual_size" =~ ^[1-9][0-9]*$ && ${#actual_size} -le 10 ]] || fail 'The downloaded package size is invalid.'
((actual_size <= 1073741824)) || fail 'The downloaded package is too large.'
actual_sha256="$(sha256sum "$archive")"
[[ "${actual_sha256%% *}" == "$expected_sha256" ]] || fail 'Downloaded package SHA-256 does not match Control.'

tar -xzf "$archive" -C "$temporary_directory"
release_directory="$temporary_directory/aster-team-runner-${version}-${platform}-${architecture}"
[[ -f "$release_directory/$initializer" && -x "$release_directory/bin/aster-team-cli" ]] || fail 'The release package is incomplete.'
"$release_directory/$initializer" --install-root "$install_root" --runner-only
"$install_root/bin/aster-team-cli" runner install
printf '%s' "$enrollment_token" > "$token_file"
chmod 600 "$token_file"
enrollment_arguments=(runner enroll --control-url "$control_url" --token-file "$token_file")
if [[ $allow_insecure_http -eq 1 ]]; then enrollment_arguments+=(--allow-insecure-http); fi
"$install_root/bin/aster-team-cli" "${enrollment_arguments[@]}"
printf 'Aster Team Runner is installed and connected.\n'
