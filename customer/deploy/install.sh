#!/usr/bin/env bash
set -Eeuo pipefail
umask 027

usage() {
  cat <<'EOF'
Internal Aster Team installation engine.

This file is not a public Customer command. Use init.sh once, then use
aster-team-cli install. Control upgrades are handled by the maintenance queue;
dedicated Runner upgrades continue to use the runner subcommand.

Options:
  --release-manifest-sha256 HEX  Required on first install; obtain separately.
  --install-root DIR             Immutable Aster Team installation root.
  --service-registration-root DIR
                                 Platform service registration directory.
  --owner-email EMAIL            Initial owner email on first install.
  --owner-password-file FILE     Initial owner password, never passed in argv.
  --database-config FILE         Non-secret installation database JSON (Linux amd64).
  --database-password-file FILE  MariaDB password input, root-owned mode 0600.
  --database-ca-certificate FILE Trusted MariaDB CA PEM when custom_ca is enabled.
  --license FILE                 Optional machine-bound license to import.
  --skip-owner                   Deliberately initialize without an owner.
  --install-local-runner         Register and start a Runner on the Control host.
  --access-protocol http|https   Public protocol; defaults to HTTP.
  --access-host HOST             Public IP or base domain.
  --bind-address IPv4            LAN address used by Aster or managed Caddy.
  --certificate-source SOURCE    caddy or provided; required for HTTPS.
  --tls-certificate FILE         PEM chain when SOURCE is provided.
  --tls-private-key FILE         Unencrypted PEM key when SOURCE is provided.
  --recover-preserved            Restore a complete preserved Control installation.
  --runner-only                  Install or upgrade a dedicated Runner host.
  -h, --help                     Show this help.
EOF
}

manifest_sha256=''
install_root=''
service_registration_root=''
owner_email=''
owner_password_file=''
license_source=''
database_config_source=''
database_password_source=''
database_ca_source=''
database_driver='sqlcipher'
skip_owner=0
runner_only=0
install_local_runner=0
access_protocol='http'
access_host=''
bind_address=''
certificate_source=''
tls_certificate=''
tls_private_key=''
recover_preserved=0
while (($#)); do
  case "$1" in
    --release-manifest-sha256) manifest_sha256="${2:-}"; shift 2 ;;
    --install-root) install_root="${2:-}"; shift 2 ;;
    --service-registration-root) service_registration_root="${2:-}"; shift 2 ;;
    --owner-email) owner_email="${2:-}"; shift 2 ;;
    --owner-password-file) owner_password_file="${2:-}"; shift 2 ;;
    --database-config) database_config_source="${2:-}"; shift 2 ;;
    --database-password-file) database_password_source="${2:-}"; shift 2 ;;
    --database-ca-certificate) database_ca_source="${2:-}"; shift 2 ;;
    --license) license_source="${2:-}"; shift 2 ;;
    --skip-owner) skip_owner=1; shift ;;
    --install-local-runner) install_local_runner=1; shift ;;
    --access-protocol) access_protocol="${2:-}"; shift 2 ;;
    --access-host) access_host="${2:-}"; shift 2 ;;
    --bind-address) bind_address="${2:-}"; shift 2 ;;
    --certificate-source) certificate_source="${2:-}"; shift 2 ;;
    --tls-certificate) tls_certificate="${2:-}"; shift 2 ;;
    --tls-private-key) tls_private_key="${2:-}"; shift 2 ;;
    --recover-preserved) recover_preserved=1; shift ;;
    --runner-only) runner_only=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ ${EUID} -eq 0 ]] || { echo 'Run the installer as root (sudo).' >&2; exit 1; }
[[ "$install_root" =~ ^/[A-Za-z0-9._/+:-]+$ && "$install_root" != '/' ]] || { echo '--install-root must be a non-root absolute path without whitespace.' >&2; exit 2; }
[[ "$service_registration_root" =~ ^/[A-Za-z0-9._/+:-]+$ && "$service_registration_root" != '/' ]] || { echo '--service-registration-root must be a non-root absolute path without whitespace.' >&2; exit 2; }
install_root="${install_root%/}"
service_registration_root="${service_registration_root%/}"
for command in awk chmod chown cp curl date dirname env find flock getent groupadd head id install ln mkdir mktemp mv nologin readlink rm rmdir runuser sed sha256sum sleep stat systemctl tar tr useradd; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required system command: $command" >&2; exit 1; }
done
install -d -o root -g root -m 0700 "$install_root/staging"
exec 9>"$install_root/staging/maintenance.lock"
flock -n 9 || { echo 'Another Aster install, upgrade, or restore operation is already running.' >&2; exit 1; }

bundle_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/.." && pwd -P)"
bundled_free_license="$bundle_root/licenses/free-license.json"
required_release_paths=(RELEASE.json VERSION init.sh bin/aster-team-cli libexec/install.sh libexec/restore-backup.sh libexec/service-health.sh bin/aster-runner systemd/aster-runner.service)
if [[ $runner_only -eq 0 ]]; then
  required_release_paths+=(bin/aster-control bin/caddy admin/index.html member/index.html systemd/aster-control@.service systemd/aster-runner@.service systemd/local-slot-preparation.json systemd/aster-caddy.service systemd/aster-upgrade.path systemd/aster-upgrade.service)
fi
for path in "${required_release_paths[@]}"; do
  [[ -f "$bundle_root/$path" ]] || { echo "Release package is incomplete: $path" >&2; exit 1; }
done
if [[ -e "$bundled_free_license" || -L "$bundled_free_license" ]]; then
  [[ -f "$bundled_free_license" && ! -L "$bundled_free_license" ]] || {
    echo 'Bundled free license must be an ordinary file.' >&2
    exit 1
  }
fi
version="$(tr -d '\r\n' < "$bundle_root/VERSION")"
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { echo 'VERSION is invalid.' >&2; exit 1; }
candidate_manifest_sha256="$(sha256sum "$bundle_root/RELEASE.json" | awk '{print $1}')"

config_parent="$install_root/config"
config_root="$config_parent/control"
cli_private_root="$install_root/config/cli"
runner_config_root="$install_root/config/runner"
caddy_config_root="$install_root/config/caddy"
key_root="$install_root/config/keys"
tls_root="$install_root/config/tls"
license_root="$install_root/config/license"
data_root="$install_root/data"
database_root="$data_root/database"
caddy_state_root="$data_root/caddy"
runtime_root="$data_root/runtime"
settlement_root="$data_root/settlements"
state_root="$install_root/state"
backup_root="$install_root/backups"
staging_root="$install_root/staging"
logs_root="$install_root/logs"
release_root="$install_root/releases"
release_dir="$release_root/$version"
current_link="$install_root/current"
first_install=1
requested_role='control'
[[ $runner_only -eq 0 ]] || requested_role='runner'
if [[ $runner_only -eq 1 ]]; then
  [[ -L "$current_link" && -x "$current_link/bin/aster-runner" ]] && first_install=0
else
  [[ -L "$current_link" && -x "$current_link/bin/aster-control" ]] && first_install=0
fi
control_role_file="$config_root/install-role"
runner_role_file="$runner_config_root/install-role"
if [[ $first_install -eq 0 ]]; then
  installed_role=''
  [[ ! -f "$control_role_file" ]] || installed_role="$(tr -d '\r\n' < "$control_role_file")"
  if [[ -f "$runner_role_file" ]]; then
    [[ -z "$installed_role" ]] || { echo 'Conflicting installation role markers.' >&2; exit 1; }
    installed_role="$(tr -d '\r\n' < "$runner_role_file")"
  fi
  [[ "$installed_role" == 'control' || "$installed_role" == 'runner' ]] || { echo 'Installation role marker is missing or invalid.' >&2; exit 1; }
  [[ "$installed_role" == "$requested_role" ]] || { echo "This host is installed as $installed_role; rerun with the matching install role." >&2; exit 2; }
fi

if [[ $first_install -eq 1 ]]; then
  [[ "$manifest_sha256" =~ ^[0-9a-f]{64}$ ]] || { echo '--release-manifest-sha256 is required on first install.' >&2; exit 2; }
  [[ "$candidate_manifest_sha256" == "$manifest_sha256" ]] || { echo 'RELEASE.json does not match the separately supplied SHA-256.' >&2; exit 1; }
  verifier_binary='bin/aster-control'
  [[ $runner_only -eq 0 ]] || verifier_binary='bin/aster-team-cli'
  expected_verifier_sha256="$(awk -v selected_path="$verifier_binary" '
    index($0, "\"path\": \"" selected_path "\"") { selected=1; next }
    selected && /"sha256":/ { value=$0; sub(/^.*"sha256": "/,"",value); sub(/".*$/,"",value); print value; exit }
  ' "$bundle_root/RELEASE.json")"
  [[ "$expected_verifier_sha256" =~ ^[0-9a-f]{64}$ ]] || { echo 'Trusted release manifest does not contain the installer verifier digest.' >&2; exit 1; }
  actual_verifier_sha256="$(sha256sum "$bundle_root/$verifier_binary" | awk '{print $1}')"
  [[ "$actual_verifier_sha256" == "$expected_verifier_sha256" ]] || { echo 'Installer verifier does not match the trusted release manifest.' >&2; exit 1; }
  verifier="$bundle_root/$verifier_binary"
else
  if [[ $runner_only -eq 1 ]]; then verifier="$current_link/bin/aster-team-cli"
  else verifier="$current_link/bin/aster-control"
  fi
fi
verify_arguments=(verify-release --root "$bundle_root")
[[ $runner_only -eq 0 ]] || verify_arguments+=(--runner-only)
"$verifier" "${verify_arguments[@]}"
if [[ -n "$database_config_source" || -n "$database_password_source" || -n "$database_ca_source" ]]; then
  [[ $first_install -eq 1 && $runner_only -eq 0 && $recover_preserved -eq 0 && -n "$database_config_source" ]] || { echo 'Database inputs apply only to a new Control installation.' >&2; exit 2; }
  [[ -f "$database_config_source" && ! -L "$database_config_source" ]] || { echo 'Database configuration must be a regular file.' >&2; exit 2; }
  database_driver="$("$verifier" inspect-database-configuration --source "$database_config_source")"
  if [[ "$database_driver" == 'mariadb' ]]; then
    [[ -f "$database_password_source" && ! -L "$database_password_source" && "$(stat -c '%U:%a' "$database_password_source")" == 'root:600' ]] || { echo 'MariaDB password input must be a root-owned regular file with mode 0600.' >&2; exit 2; }
    [[ -z "$database_ca_source" || ( -f "$database_ca_source" && ! -L "$database_ca_source" ) ]] || { echo 'MariaDB CA input must be a regular file.' >&2; exit 2; }
  else
    [[ -z "$database_password_source" && -z "$database_ca_source" ]] || { echo 'SQLCipher does not accept MariaDB inputs.' >&2; exit 2; }
  fi
elif [[ -e "$config_root/database.json" || -L "$config_root/database.json" ]]; then
  [[ -f "$config_root/database.json" && ! -L "$config_root/database.json" ]] || { echo 'Installed database configuration is unsafe.' >&2; exit 2; }
  database_driver="$("$verifier" inspect-database-configuration --source "$config_root/database.json")"
fi

preserved_control_required=(
  "$control_role_file"
  "$config_root/access.json"
  "$config_root/control.env"
  "$config_root/control-blue.env"
  "$config_root/control-green.env"
  "$key_root/installation.key"
  "$key_root/runner-task.key"
  "$config_root/runner-task-keys.json"
  "$license_root/installation.json"
)
if [[ "$database_driver" == 'mariadb' ]]; then
  preserved_control_required+=("$config_root/database.json" "$key_root/mariadb.password")
else
  preserved_control_required+=("$key_root/database.key" "$database_root/aster-team.db")
fi
preserved_control_sentinels=(
  "$key_root/database.key"
  "$database_root/aster-team.db"
  "${preserved_control_required[@]}"
  "$config_root/database.json"
  "$key_root/mariadb.password"
  "$config_root/database-ca.pem"
  "$config_root/initial-owner-credentials"
  "$license_root/license.json"
  "$state_root/license.json"
  "$state_root/license.json.pending"
  "$state_root/license.json.lock"
  "$state_root/license.json.mutation"
  "$state_root/initialization-complete"
  "$license_root/request.json"
  "$runner_config_root/identity.json"
  "$runner_config_root/runner.env"
  "$runner_config_root/task-keys.json"
  "$caddy_config_root/Caddyfile"
  "$caddy_config_root/upstreams.caddy"
)
is_plain_file() {
  [[ -f "$1" && ! -L "$1" ]]
}

if [[ $recover_preserved -eq 1 ]]; then
  [[ $first_install -eq 1 && $runner_only -eq 0 ]] || { echo '--recover-preserved is only valid when restoring an uninstalled Control host.' >&2; exit 2; }
  missing_preserved=()
  for path in "${preserved_control_required[@]}"; do
    is_plain_file "$path" || missing_preserved+=("$path")
  done
  [[ ${#missing_preserved[@]} -eq 0 ]] || {
    printf 'Preserved Control data is incomplete; missing: %s\n' "${missing_preserved[*]}" >&2
    echo 'Restore a complete backup or run aster-team-cli uninstall --purge before a new installation.' >&2
    exit 1
  }
  if ! is_plain_file "$state_root/initialization-complete" && ! is_plain_file "$license_root/request.json"; then
    echo 'Preserved Control data has no completed-initialization marker; automatic recovery cannot prove that owner initialization finished.' >&2
    exit 1
  fi
  [[ "$(tr -d '\r\n' < "$control_role_file")" == 'control' ]] || { echo 'Preserved Control role marker is invalid.' >&2; exit 1; }
  [[ ! -e "$runner_role_file" ]] || { echo 'A conflicting dedicated Runner role marker is present.' >&2; exit 1; }
elif [[ $first_install -eq 1 && $runner_only -eq 0 ]]; then
  for path in "${preserved_control_sentinels[@]}"; do
    if [[ -e "$path" || -L "$path" ]]; then
      echo 'Preserved or interrupted Control data was detected. Run aster-team-cli install to recover it, or run aster-team-cli uninstall --purge before a new installation.' >&2
      exit 1
    fi
  done
fi

health_module="$bundle_root/libexec/service-health.sh"
[[ ! -L "$health_module" ]] || { echo 'Release health module must not be a symlink.' >&2; exit 1; }
export ASTER_INSTALL_ROOT="$install_root"
source "$health_module"

if [[ $first_install -eq 1 ]]; then
  if [[ $runner_only -eq 1 ]]; then
    [[ -z "$owner_email" && -z "$owner_password_file" && -z "$license_source" && $skip_owner -eq 0 && $install_local_runner -eq 0 && "$access_protocol" == 'http' && -z "$access_host" && -z "$bind_address" && -z "$certificate_source" && -z "$tls_certificate" && -z "$tls_private_key" ]] || { echo 'Control initialization options cannot be used with --runner-only.' >&2; exit 2; }
  elif [[ $recover_preserved -eq 1 ]]; then
    [[ -z "$owner_email" && -z "$owner_password_file" && -z "$license_source" && $skip_owner -eq 0 && $install_local_runner -eq 0 ]] || { echo 'Owner, license, and local Runner initialization options cannot be used with --recover-preserved.' >&2; exit 2; }
  elif [[ $skip_owner -eq 0 ]]; then
    [[ "$owner_email" =~ ^[^[:space:]@]+@[^[:space:]@]+\.[^[:space:]@]+$ ]] || { echo '--owner-email is required on first install.' >&2; exit 2; }
    [[ -f "$owner_password_file" ]] || { echo '--owner-password-file is required on first install.' >&2; exit 2; }
  fi
else
  [[ -z "$owner_email" && -z "$owner_password_file" && -z "$license_source" && $skip_owner -eq 0 && $install_local_runner -eq 0 && "$access_protocol" == 'http' && -z "$access_host" && -z "$bind_address" && -z "$certificate_source" && -z "$tls_certificate" && -z "$tls_private_key" ]] || { echo 'Initialization options apply only to a Control first install.' >&2; exit 2; }
fi
[[ -z "$license_source" || -f "$license_source" ]] || { echo 'License source file is unavailable.' >&2; exit 2; }
[[ $install_local_runner -eq 0 || $skip_owner -eq 0 ]] || { echo '--install-local-runner requires owner initialization.' >&2; exit 2; }

bundled_free_selected=0
if [[ $first_install -eq 1 && $runner_only -eq 0 && $recover_preserved -eq 0 && -z "$license_source" && -f "$bundled_free_license" ]]; then
  "$verifier" verify-bundled-free-license \
    --source "$bundled_free_license" \
    --minimum-valid-for-seconds 900
  bundled_free_selected=1
fi

is_ipv4() {
  local value="$1" octet
  local -a octets=()
  IFS=. read -r -a octets <<< "$value"
  [[ ${#octets[@]} -eq 4 ]] || return 1
  for octet in "${octets[@]}"; do
    [[ "$octet" =~ ^[0-9]{1,3}$ ]] || return 1
    (( 10#$octet <= 255 )) || return 1
  done
}

is_domain() {
  local value="$1" label
  [[ ${#value} -ge 1 && ${#value} -le 253 && "$value" != *'..'* ]] || return 1
  IFS=. read -r -a labels <<< "$value"
  for label in "${labels[@]}"; do
    [[ ${#label} -ge 1 && ${#label} -le 63 && "$label" =~ ^[A-Za-z0-9]([A-Za-z0-9-]*[A-Za-z0-9])?$ ]] || return 1
  done
}

if [[ $runner_only -eq 0 && $first_install -eq 1 ]]; then
  [[ "$access_protocol" == 'http' || "$access_protocol" == 'https' ]] || { echo '--access-protocol must be http or https.' >&2; exit 2; }
  is_ipv4 "$bind_address" || { echo '--bind-address must be a concrete IPv4 address.' >&2; exit 2; }
  if is_ipv4 "$access_host"; then
    address_kind='ip'
    [[ "$access_host" == "$bind_address" ]] || { echo 'IP access requires --access-host and --bind-address to match.' >&2; exit 2; }
  elif is_domain "$access_host"; then
    address_kind='domain'
    access_host="${access_host,,}"
  else
    echo '--access-host must be an IPv4 address or base domain without a scheme or port.' >&2
    exit 2
  fi
  if [[ "$access_protocol" == 'http' ]]; then
    [[ -z "$certificate_source" && -z "$tls_certificate" && -z "$tls_private_key" ]] || { echo 'HTTP access does not accept certificate options.' >&2; exit 2; }
    certificate_source='none'
  else
    [[ "$certificate_source" == 'caddy' || "$certificate_source" == 'provided' ]] || { echo 'HTTPS requires --certificate-source=caddy or provided.' >&2; exit 2; }
    if [[ "$certificate_source" == 'provided' ]]; then
      if [[ $recover_preserved -eq 1 ]]; then
        is_plain_file "$tls_root/server.crt" && is_plain_file "$tls_root/server.key" || { echo 'Preserved provided HTTPS certificate files are incomplete.' >&2; exit 1; }
        [[ -z "$tls_certificate" && -z "$tls_private_key" ]] || { echo 'Recovery reuses the installed TLS certificate and does not accept replacement TLS files.' >&2; exit 2; }
      else
        [[ -f "$tls_certificate" && -f "$tls_private_key" ]] || { echo 'Provided HTTPS requires both TLS files.' >&2; exit 2; }
      fi
    else
      [[ -z "$tls_certificate" && -z "$tls_private_key" ]] || { echo 'Caddy-generated HTTPS does not accept TLS files.' >&2; exit 2; }
    fi
  fi
fi

caddy_enabled=1
member_host=''
admin_host=''
api_host=''
member_url=''
admin_url=''
api_url=''
runner_public_wss=''
if [[ $runner_only -eq 0 && $first_install -eq 1 ]]; then
  if [[ "$address_kind" == 'domain' ]]; then
    member_host="app.$access_host"
    admin_host="admin.$access_host"
    api_host="api.$access_host"
    member_url="$access_protocol://$member_host"
    admin_url="$access_protocol://$admin_host"
    api_url="$access_protocol://$api_host"
  else
    member_host="$access_host"
    admin_host="$access_host"
    api_host="$access_host"
    member_url="$access_protocol://$access_host:11081"
    admin_url="$access_protocol://$access_host:11082"
    api_url="$access_protocol://$access_host:11080"
  fi
  runner_public_wss="${api_url/http:/ws:}/api/runner/channel"
  runner_public_wss="${runner_public_wss/https:/wss:}"
fi

restore_local_runner=0
if [[ $recover_preserved -eq 1 ]]; then
  runner_identity_present=0
  runner_environment_present=0
  is_plain_file "$runner_config_root/identity.json" && runner_identity_present=1
  is_plain_file "$runner_config_root/runner.env" && runner_environment_present=1
  [[ $runner_identity_present -eq $runner_environment_present ]] || {
    echo 'Preserved local Runner configuration is incomplete; identity.json and runner.env must be present together.' >&2
    exit 1
  }
  [[ $runner_identity_present -eq 0 ]] || restore_local_runner=1
  if [[ $caddy_enabled -eq 1 ]]; then
    is_plain_file "$caddy_config_root/Caddyfile" || { echo 'Preserved Caddyfile is missing or unsafe.' >&2; exit 1; }
    if [[ "$certificate_source" == 'caddy' ]]; then
      caddy_ca_paths=(
        "$tls_root/caddy-root.crt"
        "$caddy_state_root/caddy/pki/authorities/local/root.crt"
        "$caddy_state_root/caddy/pki/authorities/local/root.key"
      )
      caddy_ca_present=0
      for path in "${caddy_ca_paths[@]}"; do
        is_plain_file "$path" && caddy_ca_present=$((caddy_ca_present + 1))
      done
      if [[ $caddy_ca_present -ne 0 && $caddy_ca_present -ne ${#caddy_ca_paths[@]} ]]; then
        echo 'Preserved Caddy CA state is incomplete; restore all CA files or purge the installation.' >&2
        exit 1
      fi
    fi
  fi
fi

reuse_release=0
if [[ -e "$release_dir" ]]; then
  [[ -d "$release_dir" && ! -L "$release_dir" && -f "$release_dir/RELEASE.json" ]] || { echo "Existing release path $release_dir is unsafe." >&2; exit 1; }
  existing_manifest_sha256="$(sha256sum "$release_dir/RELEASE.json" | awk '{print $1}')"
  [[ "$existing_manifest_sha256" == "$candidate_manifest_sha256" ]] || { echo "Release version $version is already staged with different contents." >&2; exit 1; }
  verify_arguments=(verify-release --root "$release_dir")
  [[ $runner_only -eq 0 ]] || verify_arguments+=(--runner-only)
  "$verifier" "${verify_arguments[@]}"
  reuse_release=1
fi

nologin_shell="$(command -v nologin)"
getent group aster-runner >/dev/null || groupadd --system aster-runner
id aster-runner >/dev/null 2>&1 || useradd --system --gid aster-runner --home-dir /nonexistent --shell "$nologin_shell" aster-runner
if [[ $runner_only -eq 0 ]]; then
  getent group aster-team >/dev/null || groupadd --system aster-team
  id aster-team >/dev/null 2>&1 || useradd --system --gid aster-team --home-dir "$state_root" --shell "$nologin_shell" aster-team
fi
if [[ $runner_only -eq 0 && $caddy_enabled -eq 1 ]]; then
  getent group aster-caddy >/dev/null || groupadd --system aster-caddy
  id aster-caddy >/dev/null 2>&1 || useradd --system --gid aster-caddy --home-dir "$caddy_state_root" --shell "$nologin_shell" aster-caddy
fi
install -d -o root -g root -m 0755 "$install_root" "$release_root" "$config_parent"
install -d -o root -g root -m 0700 "$cli_private_root"
install -d -o root -g aster-runner -m 0750 "$runner_config_root"
if [[ $runner_only -eq 0 ]]; then
  install -d -o root -g root -m 0755 "$data_root"
  install -d -o root -g aster-team -m 0750 "$config_root" "$key_root"
  install -d -o aster-team -g aster-team -m 0750 "$database_root" "$runtime_root" "$settlement_root" "$state_root" "$staging_root" "$staging_root/upgrades" "$logs_root"
  install -d -o aster-team -g aster-team -m 0750 "$data_root/plugins" "$data_root/plugins/incoming" "$data_root/plugins/versions" "$state_root/plugins"
  install -d -o aster-team -g aster-team -m 0750 "$state_root/slots" "$state_root/upgrades" "$state_root/upgrades/queued" "$state_root/upgrades/running" "$state_root/upgrades/completed" "$state_root/locks"
  install -d -o root -g root -m 0700 "$backup_root" "$backup_root/upgrades"
  install -d -o aster-team -g aster-team -m 2750 "$license_root"
fi

if [[ $runner_only -eq 0 && $caddy_enabled -eq 1 ]]; then
  install -d -o root -g aster-caddy -m 0750 "$caddy_config_root" "$tls_root"
  install -d -o aster-caddy -g aster-caddy -m 0750 "$caddy_state_root"
elif [[ $runner_only -eq 0 ]]; then
  install -d -o root -g aster-team -m 0750 "$tls_root"
fi
installer_temp_root="$staging_root/installer"
install -d -o root -g root -m 0700 "$installer_temp_root"

restore_license_state_ownership() {
  local path
  for path in \
    "$state_root/license.json" \
    "$state_root/license.json.pending" \
    "$state_root/license.json.lock" \
    "$state_root/license.json.staged" \
    "$state_root/license.json.activation" \
    "$state_root/license.json.mutation"; do
    [[ ! -e "$path" ]] || {
      [[ -f "$path" && ! -L "$path" ]] || {
        echo "Unsafe license state path: $path" >&2
        return 1
      }
      chown aster-team:aster-team "$path"
      chmod 0640 "$path"
    }
  done
}

license_import_root=''
staged_explicit_license=''
if [[ $runner_only -eq 0 && $first_install -eq 1 && $recover_preserved -eq 0 && ( $bundled_free_selected -eq 1 || -n "$license_source" ) ]]; then
  license_import_root="$config_root/.license-import"
  install -d -o root -g aster-team -m 0750 "$license_import_root"
  if [[ -n "$license_source" ]]; then
    staged_explicit_license="$license_import_root/explicit-license.json"
    install -o root -g aster-team -m 0640 "$license_source" "$staged_explicit_license"
  fi
fi

if [[ $bundled_free_selected -eq 1 ]]; then
  license_bootstrap_root="$license_import_root/bootstrap"
  install -d -o root -g aster-team -m 0750 "$license_bootstrap_root"
  install -o root -g aster-team -m 0750 "$verifier" "$license_bootstrap_root/aster-control"
  install -o root -g aster-team -m 0640 "$bundled_free_license" "$license_bootstrap_root/free-license.json"
  if ! "$license_bootstrap_root/aster-control" initialize-installation; then
    rm -f -- "$license_bootstrap_root/aster-control" "$license_bootstrap_root/free-license.json"
    rmdir -- "$license_bootstrap_root"
    exit 1
  fi
  chown root:aster-team "$key_root/installation.key"
  chmod 0640 "$key_root/installation.key"
  chown aster-team:aster-team "$license_root/installation.json"
  chmod 0640 "$license_root/installation.json"
  if ! "$license_bootstrap_root/aster-control" install-license --source "$license_bootstrap_root/free-license.json"; then
    rm -f -- \
      "$license_root/installation.json" \
      "$key_root/installation.key" \
      "$license_root/license.json" \
      "$state_root/license.json" \
      "$state_root/license.json.pending" \
      "$state_root/license.json.lock" \
      "$state_root/license.json.mutation" \
      "$license_bootstrap_root/aster-control" \
      "$license_bootstrap_root/free-license.json"
    rmdir -- "$license_bootstrap_root"
    echo 'Bundled free license bootstrap failed before account or database initialization; the installation can be retried.' >&2
    exit 1
  fi
  restore_license_state_ownership
  rm -f -- "$license_bootstrap_root/aster-control" "$license_bootstrap_root/free-license.json"
  rmdir -- "$license_bootstrap_root"
  chown -R aster-team:aster-team "$license_root"
  chmod 2750 "$license_root"
fi

if [[ $runner_only -eq 0 && $first_install -eq 1 && $recover_preserved -eq 0 ]]; then
  if [[ -n "$database_config_source" ]]; then
    install -o root -g aster-team -m 0640 "$database_config_source" "$config_root/database.json"
    if [[ "$database_driver" == 'mariadb' ]]; then
      install -o root -g aster-team -m 0640 "$database_password_source" "$key_root/mariadb.password"
      [[ -z "$database_ca_source" ]] || install -o root -g aster-team -m 0640 "$database_ca_source" "$config_root/database-ca.pem"
    fi
  else
    printf '{"driver":"sqlcipher"}\n' > "$config_root/database.json"
    chown root:aster-team "$config_root/database.json"
    chmod 0640 "$config_root/database.json"
  fi
  control_env_temp="$(mktemp "$installer_temp_root/control-env.XXXXXX")"
  printf '%s\n' \
    'ASTER_CONTROL_ALLOW_INSECURE_HTTP=false' \
    "ASTER_CONTROL_SECURE_COOKIES=$([[ "$access_protocol" == 'https' ]] && echo true || echo false)" > "$control_env_temp"
  install -o root -g aster-team -m 0640 "$control_env_temp" "$config_root/control.env"
  rm -f -- "$control_env_temp"

  blue_env_temp="$(mktemp "$installer_temp_root/control-blue.XXXXXX")"
  printf '%s\n' \
    'ASTER_CONTROL_LISTEN=127.0.0.1:11380' \
    'ASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:11381' \
    'ASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:11382' > "$blue_env_temp"
  install -o root -g aster-team -m 0640 "$blue_env_temp" "$config_root/control-blue.env"
  rm -f -- "$blue_env_temp"
  green_env_temp="$(mktemp "$installer_temp_root/control-green.XXXXXX")"
  printf '%s\n' \
    'ASTER_CONTROL_LISTEN=127.0.0.1:11480' \
    'ASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:11481' \
    'ASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:11482' > "$green_env_temp"
  install -o root -g aster-team -m 0640 "$green_env_temp" "$config_root/control-green.env"
  rm -f -- "$green_env_temp"

  access_temp="$(mktemp "$installer_temp_root/access.XXXXXX")"
  cat > "$access_temp" <<EOF
{
  "schema": "aster.team.access/v1",
  "protocol": "$access_protocol",
  "address_kind": "$address_kind",
  "host": "$access_host",
  "bind_address": "$bind_address",
  "certificate_source": "$certificate_source",
  "caddy_enabled": $([[ $caddy_enabled -eq 1 ]] && echo true || echo false),
  "member_url": "$member_url",
  "admin_url": "$admin_url",
  "api_url": "$api_url",
  "runner_websocket_url": "$runner_public_wss"
}
EOF
  install -o root -g aster-team -m 0640 "$access_temp" "$config_root/access.json"
  rm -f -- "$access_temp"

  if [[ $caddy_enabled -eq 1 ]]; then
    tls_directive=''
    if [[ "$access_protocol" == 'https' && "$certificate_source" == 'caddy' ]]; then
      tls_directive='tls internal'
    elif [[ "$access_protocol" == 'https' ]]; then
      install -o root -g aster-caddy -m 0640 "$tls_certificate" "$tls_root/server.crt"
      install -o root -g aster-caddy -m 0640 "$tls_private_key" "$tls_root/server.key"
      tls_directive="tls $tls_root/server.crt $tls_root/server.key"
    fi
    caddy_temp="$(mktemp "$installer_temp_root/Caddyfile.XXXXXX")"
    upstreams_temp="$(mktemp "$installer_temp_root/upstreams.XXXXXX")"
    cat > "$upstreams_temp" <<'EOF'
(aster_api_upstream) {
	reverse_proxy 127.0.0.1:11380 {
		stream_close_delay 15m
	}
}

(aster_member_upstream) {
	reverse_proxy 127.0.0.1:11381 {
		stream_close_delay 15m
	}
}

(aster_admin_upstream) {
	reverse_proxy 127.0.0.1:11382 {
		stream_close_delay 15m
	}
}
EOF
    install -o root -g aster-caddy -m 0640 "$upstreams_temp" "$caddy_config_root/upstreams.caddy"
    rm -f -- "$upstreams_temp"
    {
      printf '{\n\tadmin 127.0.0.1:2019\n\tpersist_config off\n\tauto_https disable_redirects\n}\n\n'
      printf 'import %s/upstreams.caddy\n\n' "$caddy_config_root"
      if [[ "$address_kind" == 'domain' ]]; then
        printf '%s://%s {\n\tbind %s\n' "$access_protocol" "$api_host" "$bind_address"
        [[ -z "$tls_directive" ]] || printf '\t%s\n' "$tls_directive"
        printf '\timport aster_api_upstream\n}\n\n'
        printf '%s://%s {\n\tbind %s\n' "$access_protocol" "$member_host" "$bind_address"
        [[ -z "$tls_directive" ]] || printf '\t%s\n' "$tls_directive"
        printf '\timport aster_member_upstream\n}\n\n'
        printf '%s://%s {\n\tbind %s\n' "$access_protocol" "$admin_host" "$bind_address"
        [[ -z "$tls_directive" ]] || printf '\t%s\n' "$tls_directive"
        printf '\timport aster_admin_upstream\n}\n'
      else
        for mapping in '11080:aster_api_upstream' '11081:aster_member_upstream' '11082:aster_admin_upstream'; do
          public_port="${mapping%%:*}"
          upstream="${mapping##*:}"
          printf '%s://%s:%s {\n\tbind %s\n' "$access_protocol" "$access_host" "$public_port" "$bind_address"
          [[ -z "$tls_directive" ]] || printf '\t%s\n' "$tls_directive"
          printf '\timport %s\n}\n' "$upstream"
          [[ "$upstream" == 'aster_admin_upstream' ]] || printf '\n'
        done
      fi
    } > "$caddy_temp"
    install -o root -g aster-caddy -m 0640 "$caddy_temp" "$caddy_config_root/Caddyfile"
    rm -f -- "$caddy_temp"
  fi
fi

role_file="$control_role_file"
role_group='aster-team'
other_role_file="$runner_role_file"
if [[ $runner_only -eq 1 ]]; then
  role_file="$runner_role_file"
  role_group='aster-runner'
  other_role_file="$control_role_file"
fi
[[ ! -e "$other_role_file" ]] || { echo 'A different installation role is already present.' >&2; exit 1; }
if [[ -f "$role_file" ]]; then
  [[ "$(tr -d '\r\n' < "$role_file")" == "$requested_role" ]] || { echo 'Installation role marker is invalid.' >&2; exit 1; }
else
  role_temp="$(mktemp "$installer_temp_root/install-role.XXXXXX")"
  printf '%s\n' "$requested_role" > "$role_temp"
  install -o root -g "$role_group" -m 0640 "$role_temp" "$role_file"
  rm -f -- "$role_temp"
fi

if [[ $reuse_release -eq 0 ]]; then
  stage="$(mktemp -d "$release_root/.${version}.XXXXXX")"
  cleanup() {
    if [[ -n "${stage:-}" && "$stage" == "$release_root/.${version}."* && -d "$stage" ]]; then
      rm -rf -- "$stage"
    fi
  }
  trap cleanup EXIT
  if [[ $runner_only -eq 1 ]]; then
    for relative in "${required_release_paths[@]}"; do
      mkdir -p -- "$stage/$(dirname -- "$relative")"
      cp -a -- "$bundle_root/$relative" "$stage/$relative"
    done
  else
    cp -a -- "$bundle_root/." "$stage/"
  fi
  chown -R root:root "$stage"
  find "$stage" -type d -exec chmod 0755 {} +
  find "$stage" -type f -exec chmod 0644 {} +
  executable_paths=("$stage/init.sh" "$stage/bin/aster-team-cli" "$stage/bin/aster-runner" "$stage/libexec/install.sh" "$stage/libexec/restore-backup.sh")
  if [[ $runner_only -eq 0 ]]; then executable_paths+=("$stage/bin/aster-control" "$stage/bin/caddy"); fi
  chmod 0755 "${executable_paths[@]}"
  verify_arguments=(verify-release --root "$stage")
  [[ $runner_only -eq 0 ]] || verify_arguments+=(--runner-only)
  "$verifier" "${verify_arguments[@]}"
  mv -- "$stage" "$release_dir"
  stage=''
fi
if [[ $runner_only -eq 0 && $recover_preserved -eq 0 ]]; then
  for provider in openai deepseek glm; do
    if [[ ! -e "$data_root/plugins/incoming/$provider.asterlua" ]]; then
      install -o aster-team -g aster-team -m 0640 \
        "$release_dir/plugins/$provider.asterlua" "$data_root/plugins/incoming/$provider.asterlua"
    fi
  done
fi

if [[ $first_install -eq 1 && $runner_only -eq 0 && $caddy_enabled -eq 1 ]]; then
  if [[ $recover_preserved -eq 0 ]]; then
    "$release_dir/bin/caddy" fmt --overwrite "$caddy_config_root/Caddyfile"
    "$release_dir/bin/caddy" fmt --overwrite "$caddy_config_root/upstreams.caddy"
  fi
  runuser -u aster-caddy -- env \
    HOME="$caddy_state_root" \
    XDG_DATA_HOME="$caddy_state_root" \
    XDG_CONFIG_HOME="$caddy_config_root" \
    "$release_dir/bin/caddy" validate --config "$caddy_config_root/Caddyfile" --adapter caddyfile
fi

install_units_from() {
  local source_release="$1"
  local unit temporary
  install_unit() {
    unit="$1"
    temporary="$(mktemp "$installer_temp_root/systemd-unit.XXXXXX")"
    sed "s|@ASTER_ROOT@|$install_root|g" "$source_release/systemd/$unit" > "$temporary"
    install -o root -g root -m 0644 "$temporary" "$service_registration_root/$unit"
    rm -f -- "$temporary"
  }
  if [[ $runner_only -eq 1 ]]; then
    install_unit aster-runner.service
  else
    install_unit 'aster-control@.service'
    # Old releases do not contain the slot Runner template. Restore their
    # original service inventory without enabling the new template.
    if [[ -f "$source_release/systemd/aster-runner@.service" && ! -L "$source_release/systemd/aster-runner@.service" ]]; then
      install_unit 'aster-runner@.service'
    elif [[ ! -e "$source_release/systemd/aster-runner@.service" && ! -L "$source_release/systemd/aster-runner@.service" ]]; then
      if [[ -e "$service_registration_root/aster-runner@.service" || -L "$service_registration_root/aster-runner@.service" ]]; then
        [[ -f "$service_registration_root/aster-runner@.service" && ! -L "$service_registration_root/aster-runner@.service" ]] || { echo 'Unsafe slot Runner service path.' >&2; return 1; }
        rm -f -- "$service_registration_root/aster-runner@.service"
      fi
    else
      echo 'Unsafe slot Runner service template.' >&2; return 1
    fi
    install_unit aster-runner.service
    install_unit aster-caddy.service
    install_unit aster-upgrade.path
    install_unit aster-upgrade.service
  fi
  systemctl daemon-reload
}

control_preflight() {
  local binary="$1"
  local admin_assets="$2"
  local member_assets="$3"
  local control_env="$config_root/control.env"
  local listen=''
  local admin_listen=''
  local member_listen=''
  local allow_insecure_http=''
  local secure_cookies=''
  local certificate=''
  local private_key=''
  local args=("$binary" preflight
    --admin-assets "$admin_assets" --member-assets "$member_assets")
  if [[ -f "$control_env" ]]; then
    [[ "$(stat -c '%U:%G:%a' "$control_env")" == 'root:aster-team:640' ]] || {
      echo "$control_env must be root:aster-team with mode 0640." >&2
      return 1
    }
    awk '
      { sub(/\r$/, "") }
      /^[[:space:]]*($|#)/ { next }
      !/^[A-Z0-9_]+=[^[:space:]]+$/ { exit 1 }
      {
        key=$0; sub(/=.*/, "", key)
        if (key != "ASTER_CONTROL_LISTEN" &&
            key != "ASTER_CONTROL_ADMIN_LISTEN" &&
            key != "ASTER_CONTROL_MEMBER_LISTEN" &&
            key != "ASTER_CONTROL_ALLOW_INSECURE_HTTP" &&
            key != "ASTER_CONTROL_SECURE_COOKIES" &&
            key != "ASTER_CONTROL_API_TLS_CERTIFICATE" &&
            key != "ASTER_CONTROL_API_TLS_PRIVATE_KEY") exit 1
        if (++seen[key] > 1) exit 1
      }
    ' "$control_env" || { echo "$control_env contains an unsupported, duplicate, or malformed setting." >&2; return 1; }
    listen="$(awk -F= '$1=="ASTER_CONTROL_LISTEN" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    admin_listen="$(awk -F= '$1=="ASTER_CONTROL_ADMIN_LISTEN" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    member_listen="$(awk -F= '$1=="ASTER_CONTROL_MEMBER_LISTEN" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    allow_insecure_http="$(awk -F= '$1=="ASTER_CONTROL_ALLOW_INSECURE_HTTP" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    secure_cookies="$(awk -F= '$1=="ASTER_CONTROL_SECURE_COOKIES" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    certificate="$(awk -F= '$1=="ASTER_CONTROL_API_TLS_CERTIFICATE" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    private_key="$(awk -F= '$1=="ASTER_CONTROL_API_TLS_PRIVATE_KEY" { sub(/\r$/, "", $2); print $2 }' "$control_env")"
    [[ -z "$listen" ]] || args+=(--listen "$listen")
    [[ -z "$admin_listen" ]] || args+=(--admin-listen "$admin_listen")
    [[ -z "$member_listen" ]] || args+=(--member-listen "$member_listen")
    [[ -z "$allow_insecure_http" ]] || args+=(--allow-insecure-http "$allow_insecure_http")
    [[ -z "$secure_cookies" ]] || args+=(--secure-cookies "$secure_cookies")
    if [[ -n "$certificate" || -n "$private_key" ]]; then
      [[ "$certificate" == "$tls_root/server.crt" && "$private_key" == "$tls_root/server.key" ]] || {
        echo "Control TLS files must be $tls_root/server.crt and $tls_root/server.key so they are included in backup and restore." >&2
        return 1
      }
      args+=(--api-tls-certificate "$certificate" --api-tls-private-key "$private_key")
    fi
  fi
  runuser -u aster-team -- "${args[@]}"
}

write_initialization_complete() {
  local marker_temp
  marker_temp="$(mktemp "$installer_temp_root/initialization.XXXXXX")"
  printf 'aster.team.initialization-complete/v1\n' > "$marker_temp"
  install -o aster-team -g aster-team -m 0640 "$marker_temp" "$state_root/initialization-complete"
  rm -f -- "$marker_temp"
}

recovery_committed=0
rollback_preserved_recovery() {
  if [[ $recover_preserved -eq 1 && $recovery_committed -eq 0 ]]; then
    systemctl disable --now aster-runner.service aster-caddy.service aster-control@blue.service aster-control@green.service aster-upgrade.path >/dev/null 2>&1 || true
    rm -f -- \
      "$service_registration_root/aster-runner.service" \
      "$service_registration_root/aster-caddy.service" \
      "$service_registration_root/aster-control@.service" \
      "$service_registration_root/aster-upgrade.path" \
      "$service_registration_root/aster-upgrade.service"
    [[ ! -L "$current_link" ]] || rm -f -- "$current_link"
    systemctl daemon-reload >/dev/null 2>&1 || true
    echo 'Preserved installation recovery was rolled back; accounts, settings, and data were not changed.' >&2
  fi
}
if [[ $recover_preserved -eq 1 ]]; then
  trap rollback_preserved_recovery EXIT
fi

if [[ $first_install -eq 1 ]]; then
  if [[ $runner_only -eq 1 ]]; then
    install_units_from "$release_dir"
    temporary_link="$install_root/.current.$$"
    ln -s "$release_dir" "$temporary_link"
    mv -T "$temporary_link" "$current_link"
    systemctl enable aster-runner.service
    echo "Aster Team Runner $version is installed but not started."
    echo "Configure $runner_config_root/task-keys.json, identity.json and runner.env, then start aster-runner.service."
    exit 0
  fi
  if [[ $recover_preserved -eq 1 ]]; then
    "$release_dir/bin/aster-control" verify-machine-identity
    control_preflight "$release_dir/bin/aster-control" "$release_dir/admin" "$release_dir/member"
    install -o root -g aster-runner -m 0640 "$config_root/runner-task-keys.json" "$runner_config_root/task-keys.json"
  else
    "$release_dir/bin/aster-control" initialize-installation
    "$release_dir/bin/aster-control" initialize-runner-task-key
    "$release_dir/bin/aster-control" export-runner-task-keys
    if [[ "$database_driver" == 'sqlcipher' && ! -e "$key_root/database.key" ]]; then
      install -o aster-team -g aster-team -m 0600 /dev/null "$key_root/database.key"
      head -c 32 /dev/urandom > "$key_root/database.key"
    fi
    chown root:aster-team "$key_root/installation.key" "$key_root/runner-task.key" "$config_root/runner-task-keys.json"
    chmod 0640 "$key_root/installation.key" "$key_root/runner-task.key" "$config_root/runner-task-keys.json"
    install -o root -g aster-runner -m 0640 "$config_root/runner-task-keys.json" "$runner_config_root/task-keys.json"
    chown aster-team:aster-team "$license_root/installation.json"
    chmod 0640 "$license_root/installation.json"
    runuser -u aster-team -- "$release_dir/bin/aster-control" initialize-database
    runuser -u aster-team -- "$release_dir/bin/aster-control" initialize-runtime-configuration \
       --public-api-base-url "$api_url"
    if [[ $skip_owner -eq 0 ]]; then
      password_copy="$(mktemp "$runtime_root/.owner-password.XXXXXX")"
      install -o aster-team -g aster-team -m 0600 "$owner_password_file" "$password_copy"
      if ! runuser -u aster-team -- "$release_dir/bin/aster-control" initialize-owner \
         --email "$owner_email" --password-file "$password_copy"; then
        rm -f -- "$password_copy"
        exit 1
      fi
      rm -f -- "$password_copy"
      credentials_temp="$(mktemp "$installer_temp_root/owner-credentials.XXXXXX")"
      {
        printf 'ASTER_OWNER_EMAIL=%s\n' "$owner_email"
        printf 'ASTER_OWNER_TEMPORARY_PASSWORD='
        tr -d '\r\n' < "$owner_password_file"
        printf '\n'
      } > "$credentials_temp"
      install -o root -g root -m 0600 "$credentials_temp" "$config_root/initial-owner-credentials"
      rm -f -- "$credentials_temp"
    fi
    initial_license_source="$staged_explicit_license"
    if [[ -z "$initial_license_source" && -f "$release_dir/licenses/free-license.json" ]]; then
      initial_license_source="$release_dir/licenses/free-license.json"
    fi
    if [[ -n "$initial_license_source" ]]; then
      "$release_dir/bin/aster-control" install-license --source "$initial_license_source"
      restore_license_state_ownership
      if [[ -n "$staged_explicit_license" ]]; then
        rm -f -- "$staged_explicit_license"
      fi
      chown -R aster-team:aster-team "$license_root"
      chmod 2750 "$license_root"
    fi
    if [[ $install_local_runner -eq 1 ]]; then
      local_runner_identity="$runtime_root/.local-runner-identity.json"
      runuser -u aster-team -- "$release_dir/bin/aster-control" initialize-local-runner \
         --owner-email "$owner_email" --name local-runner \
        --identity-output "$local_runner_identity"
      install -o root -g aster-runner -m 0640 "$local_runner_identity" "$runner_config_root/identity.json"
      rm -f -- "$local_runner_identity"
      runner_env_temp="$(mktemp "$installer_temp_root/runner-env.XXXXXX")"
      printf 'ASTER_RUNNER_CONTROL_WSS=%s\n' "$runner_public_wss" > "$runner_env_temp"
      if [[ "$access_protocol" == 'http' ]]; then
        printf 'ASTER_RUNNER_ALLOW_INSECURE_HTTP=true\n' >> "$runner_env_temp"
      elif [[ "$certificate_source" == 'caddy' ]]; then
        printf 'ASTER_RUNNER_CONTROL_CA_CERTIFICATE=%s/control-ca.pem\n' "$runner_config_root" >> "$runner_env_temp"
      fi
      install -o root -g aster-runner -m 0640 "$runner_env_temp" "$runner_config_root/runner.env"
      rm -f -- "$runner_env_temp"
    fi
    if [[ -n "$license_import_root" ]]; then
      rmdir -- "$license_import_root"
    fi
    write_initialization_complete
  fi
  install_units_from "$release_dir"
  temporary_link="$install_root/.current.$$"
  ln -s "$release_dir" "$temporary_link"
  mv -T "$temporary_link" "$current_link"
  blue_link="$state_root/slots/.blue-release.$$"
  ln -s "$release_dir" "$blue_link"
  mv -T "$blue_link" "$state_root/slots/blue-release"
  active_slot_temp="$(mktemp "$installer_temp_root/active-slot.XXXXXX")"
  cat > "$active_slot_temp" <<EOF
{
  "schema": "aster.active-release-slot.v1",
  "slot": "blue",
  "version": "$version"
}
EOF
  install -o aster-team -g aster-team -m 0640 "$active_slot_temp" "$state_root/slots/active.json"
  rm -f -- "$active_slot_temp"
  service_ready_deadline="$(aster_health_new_deadline)"
  systemctl enable --now aster-control@blue.service
  if [[ $caddy_enabled -eq 1 ]]; then
    runuser -u aster-caddy -- env \
      HOME="$caddy_state_root" \
      XDG_DATA_HOME="$caddy_state_root" \
      XDG_CONFIG_HOME="$caddy_config_root" \
      "$current_link/bin/caddy" validate --config "$caddy_config_root/Caddyfile" --adapter caddyfile
    systemctl enable --now aster-caddy.service aster-upgrade.path
    if [[ "$certificate_source" == 'caddy' ]]; then
      caddy_root="$caddy_state_root/caddy/pki/authorities/local/root.crt"
      if ! aster_wait_for_regular_file "$service_ready_deadline" "$caddy_root" 'Caddy internal root CA'; then
        aster_report_service_diagnostics aster-control@blue.service aster-caddy.service
        exit 1
      fi
      install -o root -g root -m 0644 "$caddy_root" "$tls_root/caddy-root.crt"
      if [[ $install_local_runner -eq 1 || $restore_local_runner -eq 1 ]]; then
        install -o root -g aster-runner -m 0640 "$caddy_root" "$runner_config_root/control-ca.pem"
      fi
    fi
  fi
else
  if [[ $runner_only -eq 1 ]]; then
    runner_env="$runner_config_root/runner.env"
    runner_identity="$runner_config_root/identity.json"
    runner_task_keys="$runner_config_root/task-keys.json"
    configured_files=0
    for runner_file in "$runner_env" "$runner_identity" "$runner_task_keys"; do
      [[ ! -f "$runner_file" ]] || configured_files=$((configured_files + 1))
    done
    [[ $configured_files -eq 0 || $configured_files -eq 3 ]] || { echo 'Runner configuration is incomplete; expected runner.env, identity.json and task-keys.json together.' >&2; exit 1; }
    if [[ $configured_files -eq 3 ]]; then
      runner_control_wss="$(awk -v key='ASTER_RUNNER_CONTROL_WSS' '
        index($0,key "=")==1 { count++; value=substr($0,length(key)+2) }
        END { if(count!=1) exit 1; print value }
      ' "$runner_env" | tr -d '\r')" || { echo 'runner.env must contain exactly one ASTER_RUNNER_CONTROL_WSS value.' >&2; exit 1; }
      runner_allow_insecure="$(awk -v key='ASTER_RUNNER_ALLOW_INSECURE_HTTP' '
        index($0,key "=")==1 { count++; value=substr($0,length(key)+2) }
        END { if(count>1) exit 1; if(count==1) print value }
      ' "$runner_env" | tr -d '\r')" || { echo 'runner.env contains duplicate ASTER_RUNNER_ALLOW_INSECURE_HTTP values.' >&2; exit 1; }
      [[ -z "$runner_allow_insecure" || "$runner_allow_insecure" == 'true' ]] || { echo 'ASTER_RUNNER_ALLOW_INSECURE_HTTP must be true when present.' >&2; exit 1; }
      if [[ "$runner_allow_insecure" == 'true' ]]; then
        [[ "$runner_control_wss" =~ ^ws://[^[:space:]]+/api/runner/channel$ ]] || { echo 'Insecure Runner Control must use WS.' >&2; exit 1; }
      else
        [[ "$runner_control_wss" =~ ^wss://[^[:space:]]+/api/runner/channel$ || "$runner_control_wss" == 'ws://127.0.0.1:11080/api/runner/channel' ]] || { echo 'ASTER_RUNNER_CONTROL_WSS is invalid.' >&2; exit 1; }
      fi
      runner_control_ca="$(awk -v key='ASTER_RUNNER_CONTROL_CA_CERTIFICATE' '
        index($0,key "=")==1 { count++; value=substr($0,length(key)+2) }
        END { if(count>1) exit 1; if(count==1) print value }
      ' "$runner_env" | tr -d '\r')" || { echo 'runner.env contains duplicate ASTER_RUNNER_CONTROL_CA_CERTIFICATE values.' >&2; exit 1; }
      preflight=(runuser -u aster-runner -- env)
      if [[ -n "$runner_control_ca" ]]; then
        [[ "$runner_control_ca" == "$runner_config_root/control-ca.pem" ]] || { echo "ASTER_RUNNER_CONTROL_CA_CERTIFICATE must be $runner_config_root/control-ca.pem so it is included in backups." >&2; exit 1; }
        preflight+=("ASTER_RUNNER_CONTROL_CA_CERTIFICATE=$runner_control_ca")
      fi
      if [[ "$runner_allow_insecure" == 'true' ]]; then
        preflight+=('ASTER_RUNNER_ALLOW_INSECURE_HTTP=true')
      fi
      preflight+=("$release_dir/bin/aster-runner" preflight
        --control-wss "$runner_control_wss"
        --allowed-upstream-host api.openai.com
        --allowed-upstream-host auth.openai.com
        --allowed-upstream-host api.anthropic.com
        --allowed-upstream-host chatgpt.com
        --allowed-upstream-host api.deepseek.com
        --allowed-upstream-host open.bigmodel.cn
        --allowed-upstream-host api.z.ai)
      "${preflight[@]}"
    fi

    install -d -o root -g root -m 0700 "$backup_root"
    backup="$(mktemp "$backup_root/pre-runner-${version}-$(date -u +%Y%m%dT%H%M%SZ).XXXXXX.tar.gz")"
    old_release="$(readlink -f "$current_link")"
    runner_was_active=0
    if systemctl is-active --quiet aster-runner.service; then
      runner_was_active=1
      systemctl stop aster-runner.service || { echo 'Could not stop Runner; upgrade was not started.' >&2; exit 1; }
    fi
    if ! tar -C "$install_root" -czf "$backup" config/runner || ! tar -tzf "$backup" >/dev/null; then
      rm -f -- "$backup"
      [[ $runner_was_active -eq 0 ]] || systemctl start aster-runner.service || true
      echo 'Could not back up Runner configuration; upgrade was not applied.' >&2
      exit 1
    fi
    chmod 0600 "$backup"
    rollback_runner_upgrade() {
      systemctl stop aster-runner.service || true
      temporary_link="$install_root/.current.$$"
      ln -s "$old_release" "$temporary_link"
      mv -Tf "$temporary_link" "$current_link"
      install_units_from "$old_release" || true
      [[ $runner_was_active -eq 0 ]] || systemctl start aster-runner.service || true
    }
    if ! install_units_from "$release_dir"; then
      rollback_runner_upgrade
      echo 'Could not install the candidate Runner unit; restored the previous runtime.' >&2
      exit 1
    fi
    temporary_link="$install_root/.current.$$"
    ln -s "$release_dir" "$temporary_link"
    mv -Tf "$temporary_link" "$current_link"
    if [[ $runner_was_active -eq 1 ]]; then
      if ! systemctl start aster-runner.service; then
        rollback_runner_upgrade
        echo 'Runner upgrade start failed; restored the previous release and service unit.' >&2
        exit 1
      fi
      sleep 2
      if ! systemctl is-active --quiet aster-runner.service; then
        rollback_runner_upgrade
        echo 'Runner upgrade health check failed; restored the previous release and service unit.' >&2
        exit 1
      fi
    fi
    echo "Aster Team Runner $version is installed."
    exit 0
  fi
  echo 'Control upgrades must be submitted through aster-team-cli upgrade or the Admin maintenance page.' >&2
  exit 2
fi

healthy=1
if ! aster_wait_for_control_health "$service_ready_deadline"; then
  healthy=0
fi
if [[ $healthy -eq 1 ]] && ! control_preflight "$current_link/bin/aster-control" "$current_link/admin" "$current_link/member"; then
  healthy=0
fi

if [[ $healthy -eq 1 ]] &&
   ! aster_wait_for_access_health "$service_ready_deadline" "$config_root/access.json"; then
  healthy=0
fi

if [[ $healthy -ne 1 ]]; then
  aster_report_service_diagnostics aster-control@blue.service aster-control@green.service aster-caddy.service
  echo 'Initial service health check failed. Inspect journalctl -u aster-control@blue.service.' >&2
  exit 1
fi

if [[ $first_install -eq 1 && ( $install_local_runner -eq 1 || $restore_local_runner -eq 1 ) ]]; then
  systemctl enable --now aster-runner.service
  sleep 2
  systemctl is-active --quiet aster-runner.service || {
    echo 'Local Runner failed to stay active. Inspect journalctl -u aster-runner.service.' >&2
    exit 1
  }
fi

if [[ $recover_preserved -eq 1 ]]; then
  write_initialization_complete
  recovery_committed=1
fi

echo "Aster Team $version is active."
if [[ $first_install -eq 1 ]]; then
  if [[ $recover_preserved -eq 1 ]]; then
    echo 'Accounts/passwords: preserved (no owner initialization was performed)'
  else
    echo "Initial credentials: $config_root/initial-owner-credentials"
  fi
  echo "Admin UI:        $admin_url"
  echo "Member UI:       $member_url"
  echo "Model API:       $api_url"
  echo "Runner channel:  $runner_public_wss"
  if [[ "$address_kind" == 'domain' ]]; then
    echo "Firewall:        allow inbound TCP $([[ "$access_protocol" == 'https' ]] && echo 443 || echo 80) on $bind_address"
  else
    echo "Firewall:        allow inbound TCP 11080-11082 on $bind_address"
  fi
  if [[ "$address_kind" == 'domain' ]]; then
    echo
    echo 'If DNS is not configured, add this line to each client hosts file:'
    echo "  $bind_address $member_host $admin_host $api_host"
  fi
  if [[ "$certificate_source" == 'caddy' ]]; then
    echo
    echo "Caddy root CA:   $tls_root/caddy-root.crt"
    echo "Root SHA-256:    $(sha256sum "$tls_root/caddy-root.crt" | awk '{print $1}')"
    echo 'Install this public root certificate into each client trust store before opening the HTTPS URLs.'
    echo "Never copy the Caddy CA private key from $caddy_state_root."
  elif [[ "$access_protocol" == 'http' ]]; then
    echo
    echo 'Warning: trusted-LAN HTTP is enabled. Do not expose ports 11080-11082 to the public Internet.'
  fi
  if [[ $install_local_runner -eq 1 || $restore_local_runner -eq 1 ]]; then
    echo 'Local Runner:    installed and active'
  fi
fi
