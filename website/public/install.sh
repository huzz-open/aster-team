#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

fail() {
  printf 'Aster Team install: %s\n' "$*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
Usage: bash install.sh [options]
       curl -fsSL <script-url> | bash -s -- [options]

Options:
  --version latest|vX.Y.Z  Select the published Linux release (default: latest)
  --email ADDRESS          Initial admin email
  --protocol http|https    Public access protocol (default: http)
  --host DOMAIN_OR_IP      Public access address (default: detected LAN IP)

Fresh Linux x86-64 Control installation on a systemd host. The installer
downloads the published release, checks its SHA-256, then runs
the signed package's interactive initialization and installation commands.
EOF
}

selected_version=latest
owner_email=''
access_protocol=http
access_host=''
custom_config=false
while (($#)); do
  case "$1" in
    -h|--help) usage; exit 0 ;;
    --version|--email|--protocol|--host)
      (($# >= 2)) || fail "Missing value for $1"
      case "$1" in
        --version) selected_version="$2" ;;
        --email) owner_email="$2"; custom_config=true ;;
        --protocol) access_protocol="$2"; custom_config=true ;;
        --host) access_host="$2"; custom_config=true ;;
      esac
      shift 2 ;;
    *) fail "Unknown argument: $1" ;;
  esac
done
[[ "$selected_version" == latest || "$selected_version" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || fail 'Version must be latest or vX.Y.Z.'
[[ "$access_protocol" == http || "$access_protocol" == https ]] || fail 'Protocol must be http or https.'
[[ -z "$owner_email" || "$owner_email" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] || fail 'Admin email is invalid.'
[[ -z "$access_host" || "$access_host" =~ ^[A-Za-z0-9][A-Za-z0-9.:-]*$ ]] || fail 'Access address must be a domain or IP address.'

[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail 'Only Linux x86-64 is supported.'
[[ -d /run/systemd/system ]] || fail 'A running systemd host is required.'
[[ -r /dev/tty && -w /dev/tty ]] || fail 'Run this command in an interactive terminal.'
for command in curl mktemp sha256sum sudo tar wc; do
  command -v "$command" >/dev/null 2>&1 || fail "Missing required command: $command"
done
[[ ! -e /opt/aster-team/current && ! -L /opt/aster-team/current ]] || fail 'An Aster Team installation already exists. Use the documented upgrade flow.'

temp_dir="$(mktemp -d -t aster-team-install.XXXXXXXX)" || fail 'Could not create a temporary directory.'
trap 'rm -rf -- "$temp_dir"' EXIT
if [[ "$selected_version" == latest ]]; then
  release_page="$(curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
    --connect-timeout 15 --max-time 60 --retry 3 --output /dev/null --write-out '%{url_effective}' \
    'https://github.com/huzz-open/aster-team/releases/latest')" || fail 'Could not resolve the latest GitHub release.'
  [[ "$release_page" =~ ^https://github\.com/huzz-open/aster-team/releases/tag/(v[0-9]+\.[0-9]+\.[0-9]+)$ ]] \
    || fail 'The latest GitHub release URL has an unexpected format.'
  tag="${BASH_REMATCH[1]}"
else
  tag="$selected_version"
fi
version="${tag#v}"
name="aster-team-${version}-linux-amd64.tar.gz"
url="https://github.com/huzz-open/aster-team/releases/download/${tag}/${name}"
checksum_file="$temp_dir/$name.sha256"
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
  --connect-timeout 15 --max-time 60 --max-filesize 1024 --retry 3 \
  --output "$checksum_file" "${url}.sha256" || fail 'Could not download the release SHA-256 file.'
[[ "$(wc -c < "$checksum_file")" -le 1024 ]] || fail 'The release SHA-256 file is too large.'
mapfile -t checksum_lines < "$checksum_file"
[[ ${#checksum_lines[@]} -eq 1 ]] || fail 'The release SHA-256 file must contain exactly one entry.'
checksum_line="${checksum_lines[0]%$'\r'}"
expected_sha256="${checksum_line%%  *}"
[[ "$expected_sha256" =~ ^[a-f0-9]{64}$ && "$checksum_line" == "$expected_sha256  $name" ]] \
  || fail 'The release SHA-256 entry does not match the selected package.'

archive="$temp_dir/$name"
printf 'Downloading Aster Team %s for Linux amd64...\n' "$version"
curl --fail --silent --show-error --location --proto '=https' --proto-redir '=https' \
  --connect-timeout 15 --max-time 900 --max-filesize 1073741824 --retry 3 \
  --output "$archive" "$url"
actual_size="$(wc -c < "$archive")"
[[ "$actual_size" =~ ^[1-9][0-9]*$ && ${#actual_size} -le 10 ]] || fail 'The downloaded package size is invalid.'
((actual_size <= 1073741824)) || fail 'The downloaded package is too large.'
actual_sha256="$(sha256sum "$archive")"
[[ "${actual_sha256%% *}" == "$expected_sha256" ]] || fail 'Downloaded package SHA-256 does not match the published release.'

tar -xzf "$archive" -C "$temp_dir"
release_dir="$temp_dir/aster-team-${version}-linux-amd64"
[[ -f "$release_dir/init.sh" && -x "$release_dir/bin/aster-team-cli" ]] || fail 'The release package is incomplete.'
cd "$release_dir"
printf 'Package verified. Initializing Aster Team...\n'
sudo ./init.sh </dev/tty
if [[ "$custom_config" == false ]]; then
  printf 'Starting interactive installation...\n'
  sudo /usr/local/bin/aster-team-cli install </dev/tty
  exit 0
fi

# The CLI accepts configured access and email flags with --unattended.
# Keep the password in a root-owned 0600 file, then show it once on success.
if [[ -z "$owner_email" ]]; then
  read -r -p 'Initial admin email: ' owner_email </dev/tty
  [[ "$owner_email" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}$ ]] || fail 'Admin email is invalid.'
fi
for command in od tr; do command -v "$command" >/dev/null 2>&1 || fail "Missing required command: $command"; done
password="$(od -An -N24 -tx1 /dev/urandom | tr -d ' \n')-Aa1!"
password_file="$(sudo mktemp /root/aster-team-owner.XXXXXXXX)" || fail 'Could not create a root-owned password file.'
cleanup_password() { sudo rm -f -- "$password_file"; }
trap 'cleanup_password; rm -rf -- "$temp_dir"' EXIT
printf '%s\n' "$password" | sudo tee "$password_file" >/dev/null
sudo chmod 600 "$password_file"
install_args=(install --unattended --owner-email "$owner_email" --owner-password-file "$password_file" --access-protocol "$access_protocol" --install-local-runner)
if [[ -n "$access_host" ]]; then install_args+=(--access-host "$access_host"); fi
printf 'Installing Aster Team with the selected settings...\n'
sudo /usr/local/bin/aster-team-cli "${install_args[@]}" </dev/tty
printf '\nINITIAL ADMIN CREDENTIALS - SAVE THESE NOW\nEmail: %s\nTemporary password: %s\n' "$owner_email" "$password"
