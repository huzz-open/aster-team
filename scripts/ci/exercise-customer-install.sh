#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: exercise-customer-install.sh (--bundle PATH | --package-dir PATH) --version VERSION [options]

Options:
  --owner-email EMAIL
  --release-signing-key PATH
  --release-signing-key-id ID
  --release-tool PATH
  --architecture ARCH
  --access-mode MODE
  --upgrade-bundle PATH
  --expected-bundled-license-id ID
  --lab-paid-license-signer PATH
  --lab-paid-license-signing-key PATH

The three release-signing options are optional as a group. When supplied, the
script also proves that a deliberately broken, correctly signed upgrade rolls
back to the previous runtime.
EOF
}

bundle=''
package_dir=''
version=''
owner_email='ci-owner@example.test'
release_signing_key=''
release_signing_key_id=''
release_tool=''
architecture='amd64'
access_mode='ip-http'
install_root='/srv/aster-team'
cli_path='/usr/local/bin/aster-team-cli'
upgrade_bundle=''
expected_bundled_license_id=''
lab_paid_license_signer=''
lab_paid_license_signing_key=''

while [[ $# -gt 0 ]]; do
  case "$1" in
    --bundle) bundle=${2-}; shift 2 ;;
    --package-dir) package_dir=${2-}; shift 2 ;;
    --version) version=${2-}; shift 2 ;;
    --owner-email) owner_email=${2-}; shift 2 ;;
    --release-signing-key) release_signing_key=${2-}; shift 2 ;;
    --release-signing-key-id) release_signing_key_id=${2-}; shift 2 ;;
    --release-tool) release_tool=${2-}; shift 2 ;;
    --architecture) architecture=${2-}; shift 2 ;;
    --access-mode) access_mode=${2-}; shift 2 ;;
    --upgrade-bundle) upgrade_bundle=${2-}; shift 2 ;;
    --expected-bundled-license-id) expected_bundled_license_id=${2-}; shift 2 ;;
    --lab-paid-license-signer) lab_paid_license_signer=${2-}; shift 2 ;;
    --lab-paid-license-signing-key) lab_paid_license_signing_key=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

if [[ -n "$bundle" && -n "$package_dir" ]]; then
  echo 'Use either --bundle or --package-dir, not both.' >&2
  exit 2
fi
if [[ -z "$bundle" && -z "$package_dir" ]]; then
  echo 'A bundle or package directory is required.' >&2
  exit 2
fi
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
  echo "Invalid release version: $version" >&2
  exit 2
}
[[ -n "$owner_email" ]] || { echo 'Owner email is required.' >&2; exit 2; }
case "$access_mode" in
  ip-http|ip-https-caddy|ip-https-provided|domain-http|domain-https-caddy|domain-https-provided) ;;
  *) echo "Invalid access smoke mode: $access_mode" >&2; exit 2 ;;
esac

rollback_inputs=0
for value in "$release_signing_key" "$release_signing_key_id" "$release_tool"; do
  [[ -n "$value" ]] && rollback_inputs=$((rollback_inputs + 1))
done
if [[ $rollback_inputs -ne 0 && $rollback_inputs -ne 3 ]]; then
  echo 'Rollback verification requires the signing key, key ID, and release tool together.' >&2
  exit 2
fi
if [[ $rollback_inputs -eq 3 ]]; then
  [[ -f "$release_signing_key" ]] || { echo 'Release signing key is missing.' >&2; exit 2; }
  [[ -x "$release_tool" ]] || { echo 'Release tool is missing or not executable.' >&2; exit 2; }
fi
paid_switch_inputs=0
[[ -n "$lab_paid_license_signer" ]] && paid_switch_inputs=$((paid_switch_inputs + 1))
[[ -n "$lab_paid_license_signing_key" ]] && paid_switch_inputs=$((paid_switch_inputs + 1))
if [[ $paid_switch_inputs -ne 0 && $paid_switch_inputs -ne 2 ]]; then
  echo 'Paid License switch verification requires both the lab signer and its ephemeral key.' >&2
  exit 2
fi
if [[ $paid_switch_inputs -eq 2 ]]; then
  [[ -x "$lab_paid_license_signer" ]] || { echo 'Lab paid License signer is missing or not executable.' >&2; exit 2; }
  [[ -f "$lab_paid_license_signing_key" ]] || { echo 'Lab paid License signing key is missing.' >&2; exit 2; }
  [[ "$owner_email" =~ ^[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+$ ]] || {
    echo 'Paid License switch verification requires a JSON-safe owner email.' >&2
    exit 2
  }
fi
if [[ -n "$upgrade_bundle" ]]; then
  [[ -d "$upgrade_bundle" ]] || { echo 'Upgrade fixture bundle is missing.' >&2; exit 2; }
  upgrade_bundle="$(cd -- "$upgrade_bundle" && pwd -P)"
  [[ -f "$upgrade_bundle/VERSION" && -f "$upgrade_bundle/RELEASE.json" ]] || {
    echo 'Upgrade fixture bundle is incomplete.' >&2
    exit 2
  }
  [[ "$access_mode" == ip-http ]] || {
    echo 'Continuous successful-upgrade probing currently requires ip-http smoke access.' >&2
    exit 2
  }
fi

if [[ -n "$package_dir" ]]; then
  [[ -d "$package_dir" ]] || { echo 'Package directory is missing.' >&2; exit 2; }
  package_dir="$(cd -- "$package_dir" && pwd -P)"
  archive="$package_dir/aster-team-${version}-linux-amd64.tar.gz"
  checksum="$archive.sha256"
  bundle="$package_dir/aster-team-${version}-linux-amd64"
  [[ -f "$archive" && -f "$checksum" ]] || { echo 'Customer package or checksum is missing.' >&2; exit 2; }
  (
    cd -- "$package_dir"
    sha256sum --check "$(basename -- "$checksum")"
  )
  rm -rf -- "$bundle"
  tar -xzf "$archive" -C "$package_dir"
fi
[[ -d "$bundle" ]] || { echo 'Bundle directory is missing.' >&2; exit 2; }
bundle="$(cd -- "$bundle" && pwd -P)"
manifest="$bundle/RELEASE.json"
[[ -f "$manifest" ]] || { echo "Release manifest is missing: $manifest" >&2; exit 2; }

owner_password_file="$(sudo mktemp /root/aster-owner-password.XXXXXX)"
tls_certificate=''
tls_private_key=''
candidate_root=''
availability_stop=''
availability_log=''
availability_pid=''
install_log="$(mktemp)"
paid_license=''
paid_license_root=''
owner_login_request=''
owner_cookie_jar=''
owner_change_request=''
owner_password_rotated=0
cleanup() {
  if [[ -n "$availability_pid" ]]; then
    touch "$availability_stop"
    wait "$availability_pid" || true
    availability_pid=''
  fi
  sudo rm -f -- "$owner_password_file"
  [[ -z "$tls_certificate" ]] || sudo rm -f -- "$tls_certificate"
  [[ -z "$tls_private_key" ]] || sudo rm -f -- "$tls_private_key"
  if [[ -n "$candidate_root" && -d "$candidate_root" ]]; then
    rm -rf -- "$candidate_root"
  fi
  [[ -z "$availability_stop" ]] || rm -f -- "$availability_stop"
  [[ -z "$availability_log" ]] || rm -f -- "$availability_log"
  [[ -z "$paid_license_root" ]] || sudo rm -rf -- "$paid_license_root"
  [[ -z "$owner_login_request" ]] || sudo rm -f -- "$owner_login_request"
  [[ -z "$owner_cookie_jar" ]] || sudo rm -f -- "$owner_cookie_jar"
  [[ -z "$owner_change_request" ]] || sudo rm -f -- "$owner_change_request"
  rm -f -- "$install_log"
}
trap cleanup EXIT

assert_root_file() {
  local path=$1
  sudo test -f "$path" || {
    echo "Expected preserved file is missing or inaccessible to root: $path" >&2
    exit 1
  }
}

assert_root_path_absent() {
  local path=$1
  sudo test ! -e "$path" || {
    echo "Expected removed path is still present: $path" >&2
    exit 1
  }
}

sudo chmod 0600 "$owner_password_file"
openssl rand -base64 48 | tr -d '\n' | sudo tee "$owner_password_file" >/dev/null

lan_ip="$(ip -4 route get 192.0.2.1 | awk '{ for (i=1; i<=NF; i++) if ($i == "src") { print $(i+1); exit } }')"
[[ "$lan_ip" =~ ^[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo 'Could not detect the CI LAN IPv4 address.' >&2; exit 1; }
access_host="$lan_ip"
[[ "$access_mode" != domain-* ]] || access_host='inner-aster.test'
install_access_args=(--access-host "$access_host" --bind-address "$lan_ip")
runner_access_host="$access_host"
[[ "$access_mode" != domain-* ]] || runner_access_host="api.$access_host"
runner_access_scheme='ws'
if [[ "$access_mode" == *-https-* ]]; then
  runner_access_scheme='wss'
  install_access_args+=(--access-protocol https)
  if [[ "$access_mode" == *-caddy ]]; then
    install_access_args+=(--certificate-source caddy)
  else
    tls_certificate="$(sudo mktemp /root/aster-smoke-certificate.XXXXXX.pem)"
    tls_private_key="$(sudo mktemp /root/aster-smoke-private-key.XXXXXX.pem)"
    sudo chmod 0644 "$tls_certificate"
    sudo chmod 0600 "$tls_private_key"
    if [[ "$access_mode" == ip-* ]]; then
      subject_alt_name="IP:$lan_ip"
    else
      subject_alt_name='DNS:app.inner-aster.test,DNS:admin.inner-aster.test,DNS:api.inner-aster.test'
    fi
    sudo openssl req -x509 -newkey rsa:2048 -sha256 -nodes -days 1 \
      -subj '/CN=Aster Team CI' -addext "subjectAltName=$subject_alt_name" \
      -keyout "$tls_private_key" -out "$tls_certificate" >/dev/null 2>&1
    install_access_args+=(
      --certificate-source provided
      --tls-certificate "$tls_certificate"
      --tls-private-key "$tls_private_key"
    )
  fi
fi

sudo "$bundle/init.sh" --install-root "$install_root"
sudo "$cli_path" install --unattended \
  --owner-email "$owner_email" \
  --owner-password-file "$owner_password_file" \
  --install-local-runner \
  "${install_access_args[@]}" 2>&1 | tee "$install_log"

sudo systemctl is-active --quiet aster-control@blue.service
sudo systemctl is-active --quiet aster-caddy.service
sudo systemctl is-active --quiet aster-runner.service
for legacy_root in /opt/aster-team /etc/aster-team /var/lib/aster-team /var/log/aster-team /var/backups/aster-team; do
  sudo test ! -e "$legacy_root" || {
    echo "Installation escaped the configured root into legacy path: $legacy_root" >&2
    exit 1
  }
done
test "$(readlink -f "$cli_path")" = "$(readlink -f "$install_root/bin/aster-team-cli")"
for unit in aster-control@.service aster-caddy.service aster-runner.service aster-upgrade.path aster-upgrade.service; do
  sudo grep -Fq "$install_root" "/etc/systemd/system/$unit" || {
    echo "systemd registration does not reference the configured root: $unit" >&2
    exit 1
  }
done
probe_access() {
  sudo env ASTER_INSTALL_ROOT="$install_root" \
    bash "$install_root/current/libexec/service-health.sh" \
    wait-access "$install_root/config/control/access.json"
}

start_availability_probe() {
  availability_stop="$(mktemp)"
  availability_log="$(mktemp)"
  rm -f -- "$availability_stop"
  (
    while [[ ! -e "$availability_stop" ]]; do
      for url in "http://$lan_ip:11080/healthz" "http://$lan_ip:11081/" "http://$lan_ip:11082/"; do
        if ! curl --fail --silent --show-error --max-time 3 "$url" >/dev/null; then
          printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$url" >> "$availability_log"
        fi
      done
      if sudo systemctl is-active --quiet aster-control@blue.service &&
         sudo systemctl is-active --quiet aster-control@green.service; then
        echo 'Concurrent Control slots during SQLite maintenance upgrade.' >> "$availability_log"
        exit 1
      fi
      sleep 0.1
    done
  ) &
  availability_pid=$!
}

stop_availability_probe() {
  [[ -n "$availability_pid" ]] || return 0
  touch "$availability_stop"
  if ! wait "$availability_pid"; then
    cat "$availability_log" >&2
    echo 'Maintenance probe failed (including concurrent Control slots).' >&2
    exit 1
  fi
  # SQLite explicitly permits downtime; recovery still must pass public probes.
  if [[ -s "$availability_log" ]]; then
    echo 'Recorded expected maintenance-window outages:'
    cat "$availability_log"
  fi
  probe_access
  availability_pid=''
}
probe_access
bundled_license_sha256=''
expected_active_license_sha256=''
if [[ -f "$bundle/licenses/free-license.json" ]]; then
  grep -Fq 'A license was installed during setup.' "$install_log"
  grep -Fq 'license: active' "$install_log"
  grep -Eq '^plan: .+$' "$install_log"
  grep -Eq '^member seats: [0-9]+$' "$install_log"
  grep -Eq '^runners: ([0-9]+|unlimited)$' "$install_log"
  grep -Eq '^subscriptions/accounts: ([0-9]+|unlimited)$' "$install_log"
  grep -Eq '^API keys per member: ([0-9]+|unlimited)$' "$install_log"
  grep -Eq '^expires: (never|[0-9]{4}-[0-9]{2}-[0-9]{2}T.+Z)$' "$install_log"
  if grep -Fq 'aster-team-license-request-' "$install_log"; then
    echo 'Bundled free license installation unexpectedly generated a machine-bound license request.' >&2
    exit 1
  fi
  sudo test -f "$install_root/config/license/license.json"
  sudo test -f "$install_root/state/license.json"
  sudo test -f "$install_root/state/license.json.lock"
  sudo test -f "$install_root/state/license.json.activation"
  sudo test -f "$install_root/state/license.json.mutation"
  test "$(sudo stat -c '%U:%G:%a' "$install_root/config/license/license.json")" = 'aster-team:aster-team:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json")" = 'aster-team:aster-team:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json.lock")" = 'aster-team:aster-team:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json.activation")" = 'aster-team:aster-team:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json.mutation")" = 'aster-team:aster-team:640'
  bundled_license_sha256="$(sha256sum "$bundle/licenses/free-license.json" | awk '{print $1}')"
  expected_active_license_sha256="$bundled_license_sha256"
  test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$bundled_license_sha256"
  if [[ -n "$expected_bundled_license_id" ]]; then
    [[ "$expected_bundled_license_id" =~ ^[A-Za-z0-9_.:-]{3,128}$ ]] || { echo 'Expected bundled license ID is invalid.' >&2; exit 2; }
    sudo grep -Fq "\"license_id\":\"$expected_bundled_license_id\"" "$install_root/config/license/license.json"
  fi
elif [[ -n "$expected_bundled_license_id" ]]; then
  echo 'The expected bundled license fixture is missing from the package.' >&2
  exit 1
fi
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/keys/installation.key")" = 'root:aster-team:640'
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/keys/database.key")" = 'aster-team:aster-team:600'
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/control/initial-owner-credentials")" = 'root:root:600'
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/runner/identity.json")" = 'root:aster-runner:640'
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/runner/runner.env")" = 'root:aster-runner:640'
runner_access_port=':11080'
[[ "$access_mode" != domain-* ]] || runner_access_port=''
sudo grep -qx "ASTER_RUNNER_CONTROL_WSS=${runner_access_scheme}://${runner_access_host}${runner_access_port}/api/runner/channel" "$install_root/config/runner/runner.env"
if sudo head -c 16 "$install_root/data/database/aster-team.db" | grep -aq '^SQLite format 3'; then
  echo 'SQLCipher database has a plaintext SQLite header.' >&2
  exit 1
fi
sudo "$cli_path" status
sudo "$cli_path" doctor

assert_owner_login() {
  local owner_password new_password login_status login_headers login_response change_required=0
  owner_login_request="$(sudo mktemp /root/aster-owner-login.XXXXXX.json)"
  sudo chmod 0600 "$owner_login_request"
  owner_password="$(sudo tr -d '\r\n' < "$owner_password_file")"
  printf '{"email":"%s","password":"%s"}\n' "$owner_email" "$owner_password" |
    sudo tee "$owner_login_request" >/dev/null
  owner_password=''
  [[ -n "$owner_cookie_jar" ]] || {
    owner_cookie_jar="$(sudo mktemp /root/aster-owner-cookie.XXXXXX.txt)"
    sudo chmod 0600 "$owner_cookie_jar"
  }
  login_headers="$(sudo mktemp /root/aster-owner-login-headers.XXXXXX)"
  login_response="$(sudo mktemp /root/aster-owner-login-response.XXXXXX)"
  sudo chmod 0600 "$login_headers" "$login_response"
  login_status="$(sudo curl --silent --show-error \
    --header 'Content-Type: application/json' \
    --cookie-jar "$owner_cookie_jar" \
    --dump-header "$login_headers" \
    --output "$login_response" \
    --write-out '%{http_code}' \
    --data-binary "@$owner_login_request" \
    "http://$lan_ip:11080/api/admin/auth/login")"
  if [[ "$login_status" != 200 ]]; then
    printf 'Owner login failed: HTTP %s; response SHA-256 %s\n' \
      "$login_status" "$(sudo sha256sum "$login_response" | awk '{print $1}')" >&2
    sudo awk 'BEGIN { IGNORECASE = 1 } tolower($1) == "server:" || tolower($1) == "x-aster-error-number:" || tolower($1) == "content-type:" { gsub("\r", "", $2); print $1, $2 }' "$login_headers" >&2
    return 1
  fi
  if sudo grep -Eq '"password_change_required"[[:space:]]*:[[:space:]]*true' "$login_response"; then
    change_required=1
  fi
  sudo rm -f -- "$login_headers" "$login_response"
  sudo rm -f -- "$owner_login_request"
  owner_login_request=''
  if [[ $change_required -eq 1 ]]; then
    [[ $owner_password_rotated -eq 0 ]] || { echo 'Owner still requires a password change after rotation.' >&2; return 1; }
    owner_password_rotated=1
    owner_password="$(sudo tr -d '\r\n' < "$owner_password_file")"
    new_password="$(openssl rand -base64 48 | tr -d '\n')"
    owner_change_request="$(sudo mktemp /root/aster-owner-change.XXXXXX.json)"
    sudo chmod 0600 "$owner_change_request"
    printf '{"current_password":"%s","new_password":"%s"}\n' "$owner_password" "$new_password" |
      sudo tee "$owner_change_request" >/dev/null
    owner_password=''
    sudo curl --fail --silent --show-error \
      --cookie "$owner_cookie_jar" \
      --header 'Content-Type: application/json' \
      --data-binary "@$owner_change_request" \
      "http://$lan_ip:11080/api/admin/auth/password" >/dev/null
    printf '%s\n' "$new_password" | sudo tee "$owner_password_file" >/dev/null
    new_password=''
    sudo rm -f -- "$owner_change_request"
    owner_change_request=''
    assert_owner_login
  fi
}

if [[ $paid_switch_inputs -eq 2 ]]; then
  [[ -n "$bundled_license_sha256" ]] || {
    echo 'Paid License switch verification requires a bundled free License.' >&2
    exit 1
  }
  assert_owner_login
  paid_license_root="$(sudo mktemp -d /root/aster-lab-paid-license.XXXXXX)"
  paid_license="$paid_license_root/paid-license.json"
  sudo "$lab_paid_license_signer" \
    --installation-profile "$install_root/config/license/installation.json" \
    --private-key "$lab_paid_license_signing_key" \
    --minimum-version "$version" \
    --output "$paid_license"
  sudo test "$(sudo stat -c '%a' "$paid_license")" = 600
  sudo curl --fail --silent --show-error \
    --cookie "$owner_cookie_jar" \
    --header 'Content-Type: application/json' \
    --data-binary "@$paid_license" \
    "http://$lan_ip:11080/api/admin/license/preview" |
    grep -q '"activation":"active"'
  [[ "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" == "$bundled_license_sha256" ]]
  sudo curl --fail --silent --show-error \
    --cookie "$owner_cookie_jar" \
    --header 'Content-Type: application/json' \
    --data-binary "@$paid_license" \
    "http://$lan_ip:11080/api/admin/license" |
    grep -q '"activation":"active"'
  paid_status="$(sudo "$cli_path" license status)"
  grep -Fqx 'license: active' <<<"$paid_status"
  grep -Fqx 'id: lab_paid_switch' <<<"$paid_status"
  grep -Fqx 'plan: lab_paid_20' <<<"$paid_status"
  grep -Fqx 'member seats: 20' <<<"$paid_status"
  grep -Fqx 'runners: unlimited' <<<"$paid_status"
  grep -Fqx 'subscriptions/accounts: unlimited' <<<"$paid_status"
  grep -Fqx 'API keys per member: unlimited' <<<"$paid_status"
  expected_active_license_sha256="$(sudo sha256sum "$paid_license" | awk '{print $1}')"
  test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$expected_active_license_sha256"
  assert_owner_login
  sudo chown root:root "$install_root/state/license.json.lock"
  sudo chown root:root "$install_root/state/license.json.mutation"
  if sudo "$cli_path" license install --source "$bundle/licenses/free-license.json"; then
    echo 'Bundled free License unexpectedly replaced the paid License.' >&2
    exit 1
  fi
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json.lock")" = 'aster-team:aster-team:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/state/license.json.mutation")" = 'aster-team:aster-team:640'
  test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$expected_active_license_sha256"
  # The CLI intentionally stops Control during a privileged License import, even
  # when it rejects a rollback. Wait for the restarted service before logging in.
  probe_access
  assert_owner_login
fi

backup="/root/aster-team-${version}-backup.tar.gz"
sudo rm -f -- "$backup"
sudo "$cli_path" backup create --output "$backup"
sudo test -f "$backup" || { echo 'Backup command did not create the archive.' >&2; exit 1; }
sudo "$cli_path" backup restore --source "$backup" --confirm
sudo systemctl is-active --quiet aster-control@blue.service
probe_access
[[ $paid_switch_inputs -ne 2 ]] || assert_owner_login
if [[ "$access_mode" == *-https-provided ]]; then
  test "$(sudo stat -c '%U:%G:%a' "$install_root/config/tls/server.crt")" = 'root:aster-caddy:640'
  test "$(sudo stat -c '%U:%G:%a' "$install_root/config/tls/server.key")" = 'root:aster-caddy:640'
elif [[ "$access_mode" == *-https-caddy ]]; then
  test "$(sudo stat -c '%U:%G:%a' "$install_root/config/tls/caddy-root.crt")" = 'root:root:644'
fi
sudo find "$install_root/backups" -maxdepth 1 -type f \
  -name 'aster-team-before-restore-*.tar.gz' -print -quit | grep -q .

installation_identity_sha256="$(sudo sha256sum "$install_root/config/license/installation.json" | awk '{print $1}')"
installation_key_sha256="$(sudo sha256sum "$install_root/config/keys/installation.key" | awk '{print $1}')"
database_key_sha256="$(sudo sha256sum "$install_root/config/keys/database.key" | awk '{print $1}')"
access_configuration_sha256="$(sudo sha256sum "$install_root/config/control/access.json" | awk '{print $1}')"
sudo "$cli_path" uninstall
assert_root_path_absent "$install_root/current"
assert_root_file "$install_root/data/database/aster-team.db"
assert_root_file "$install_root/config/keys/database.key"
sudo "$bundle/init.sh" --install-root "$install_root"
sudo "$cli_path" install --recover-preserved
test "$(sudo sha256sum "$install_root/config/license/installation.json" | awk '{print $1}')" = "$installation_identity_sha256"
test "$(sudo sha256sum "$install_root/config/keys/installation.key" | awk '{print $1}')" = "$installation_key_sha256"
test "$(sudo sha256sum "$install_root/config/keys/database.key" | awk '{print $1}')" = "$database_key_sha256"
test "$(sudo sha256sum "$install_root/config/control/access.json" | awk '{print $1}')" = "$access_configuration_sha256"
sudo systemctl is-active --quiet aster-control@blue.service
sudo systemctl is-active --quiet aster-caddy.service
sudo systemctl is-active --quiet aster-runner.service
sudo "$cli_path" doctor
probe_access
if [[ -n "$expected_active_license_sha256" ]]; then
  test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$expected_active_license_sha256"
fi
[[ $paid_switch_inputs -ne 2 ]] || assert_owner_login

current_runtime_version="$version"
current_active_slot="$(sudo awk -F'"' '$2 == "slot" { print $4; exit }' "$install_root/state/slots/active.json")"
[[ "$current_active_slot" == blue || "$current_active_slot" == green ]]
if [[ -n "$upgrade_bundle" ]]; then
  upgrade_version="$(tr -d '\r\n' < "$upgrade_bundle/VERSION")"
  start_availability_probe
  sudo "$upgrade_bundle/init.sh" --install-root "$install_root"
  sudo "$cli_path" upgrade
  test "$(readlink -f "$install_root/current")" = \
    "$(readlink -f "$install_root/releases/$upgrade_version")"
  test "$(sudo awk -F'"' '$2 == "version" { print $4; exit }' "$install_root/state/slots/active.json")" = "$upgrade_version"
  active_slot="$(sudo awk -F'"' '$2 == "slot" { print $4; exit }' "$install_root/state/slots/active.json")"
  [[ "$active_slot" == blue || "$active_slot" == green ]]
  sudo systemctl is-active --quiet "aster-control@${active_slot}.service"
  if [[ "$active_slot" == blue ]]; then
    previous_slot=green
  else
    previous_slot=blue
  fi
  sudo systemctl is-active --quiet "aster-control@${previous_slot}.service" && {
    echo 'Previous Control slot is still active after maintenance upgrade.' >&2
    exit 1
  }
  probe_access
  if [[ -n "$expected_active_license_sha256" ]]; then
    test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$expected_active_license_sha256"
  fi
  [[ $paid_switch_inputs -ne 2 ]] || assert_owner_login
  current_runtime_version="$upgrade_version"
  current_active_slot="$active_slot"
fi

if [[ $rollback_inputs -eq 3 ]]; then
  current_before="$(readlink -f "$install_root/current")"
  [[ "$current_runtime_version" =~ ^([0-9]+)\.([0-9]+)\.([0-9]+) ]] || {
    echo 'Current runtime version is invalid.' >&2
    exit 1
  }
  rollback_version="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((BASH_REMATCH[3] + 1))-ci-fail"
  candidate_root="$(mktemp -d)"
  candidate="$candidate_root/aster-team-$rollback_version"
  candidate_source="$bundle"
  [[ -z "$upgrade_bundle" ]] || candidate_source="$upgrade_bundle"
  cp -a -- "$candidate_source" "$candidate"
  rm -f -- "$candidate/RELEASE.json"
  printf '%s\n' "$rollback_version" > "$candidate/VERSION"
  printf '#!/usr/bin/env bash\nexit 1\n' > "$candidate/bin/aster-control"
  chmod 0755 "$candidate/bin/aster-control"
  "$release_tool" sign \
    --root "$candidate" \
    --private-key "$release_signing_key" \
    --key-id "$release_signing_key_id" \
    --version "$rollback_version" \
    --architecture "$architecture" \
    --runtime musl-static \
    --created-at "$(date -u +%Y-%m-%dT%H:%M:%S.000Z)"
  sudo "$candidate/init.sh" --install-root "$install_root"
  if sudo "$cli_path" upgrade; then
    echo 'Deliberately broken signed candidate unexpectedly installed.' >&2
    exit 1
  fi
  test "$(readlink -f "$install_root/current")" = "$current_before"
  test "$(sudo awk -F'"' '$2 == "slot" { print $4; exit }' "$install_root/state/slots/active.json")" = "$current_active_slot"
  sudo systemctl is-active --quiet "aster-control@${current_active_slot}.service"
  if [[ "$current_active_slot" == blue ]]; then
    failed_candidate_slot=green
  else
    failed_candidate_slot=blue
  fi
  sudo systemctl is-active --quiet "aster-control@${failed_candidate_slot}.service" && {
    echo 'Failed candidate Control slot is still active after rollback.' >&2
    exit 1
  }
  grep -q "^ExecStart=$install_root/state/slots/%i-release/bin/aster-control serve " \
    '/etc/systemd/system/aster-control@.service'
  probe_access
  if [[ -n "$expected_active_license_sha256" ]]; then
    test "$(sudo sha256sum "$install_root/config/license/license.json" | awk '{print $1}')" = "$expected_active_license_sha256"
  fi
  [[ $paid_switch_inputs -ne 2 ]] || assert_owner_login
fi

stop_availability_probe

echo "Customer install smoke test passed for Aster Team $version${upgrade_bundle:+ with maintenance upgrade to $current_runtime_version}."
