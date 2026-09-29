#!/usr/bin/env bash
set -Eeuo pipefail
umask 027

usage() {
  cat <<'EOF'
Usage: sudo ./install.sh --env-file /path/to/operations.env [options]

Installs or upgrades the Aster Team internal Operations Console and API.
The env file must already contain the MariaDB and bootstrap credentials.

Options:
  --install-root PATH  Keep all Aster Operations files below PATH.
                       Default: /opt/aster-operations
  --service-registration-root PATH
                       systemd unit directory. Default: /etc/systemd/system
  --create-database    Create the configured database before migration.
  -h, --help           Show this help.
EOF
}

env_source=''
install_root='/opt/aster-operations'
service_registration_root='/etc/systemd/system'
create_database=0
while (($#)); do
  case "$1" in
    --env-file) env_source="${2:-}"; shift 2 ;;
    --install-root) install_root="${2:-}"; shift 2 ;;
    --service-registration-root) service_registration_root="${2:-}"; shift 2 ;;
    --create-database) create_database=1; shift ;;
    --help|-h) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ ${EUID} -eq 0 ]] || { echo 'Run this installer as root (sudo).' >&2; exit 1; }
[[ -n "$env_source" && -f "$env_source" ]] || { echo '--env-file must reference an existing file.' >&2; exit 1; }
for command in awk install mktemp realpath sed useradd runuser systemctl; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required command: $command" >&2; exit 1; }
done
[[ "$install_root" =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo '--install-root must be an absolute Linux path without spaces or control characters.' >&2; exit 1; }
install_root="$(realpath -m -- "$install_root")"
[[ "$install_root" != / ]] || { echo '--install-root cannot be the filesystem root.' >&2; exit 1; }
[[ "$service_registration_root" =~ ^/[A-Za-z0-9._/-]+$ ]] || { echo '--service-registration-root must be an absolute Linux path without spaces or control characters.' >&2; exit 1; }
service_registration_root="$(realpath -m -- "$service_registration_root")"
[[ "$service_registration_root" != / ]] || { echo '--service-registration-root cannot be the filesystem root.' >&2; exit 1; }

bundle_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
for required in VERSION bin/aster-operations-api bin/aster-operations-backup bin/aster-webhost console/index.html operations.env.example systemd/aster-operations-api.service systemd/aster-operations-web.service; do
  [[ -e "$bundle_root/$required" ]] || { echo "Operations bundle is incomplete: missing $required" >&2; exit 1; }
done
version="$(tr -d '\r\n' < "$bundle_root/VERSION")"
[[ "$version" =~ ^[0-9A-Za-z._-]+$ ]] || { echo 'Bundle version is invalid.' >&2; exit 1; }

required_keys=(ASTER_OPERATIONS_DB_NAME ASTER_OPERATIONS_DB_USER ASTER_OPERATIONS_DB_PASSWORD ASTER_OPERATIONS_BOOTSTRAP_ADMIN_EMAIL ASTER_OPERATIONS_BOOTSTRAP_ADMIN_PASSWORD ASTER_OPERATIONS_ARTIFACT_ROOT ASTER_OPERATIONS_LICENSE_SIGNING_KEY_ID ASTER_OPERATIONS_LICENSE_SIGNING_PRIVATE_KEY_PKCS8 ASTER_OPERATIONS_CUSTOMER_REF_SECRET)
for key in "${required_keys[@]}"; do
  value="$(sed -n "s/^${key}=//p" "$env_source" | tail -n1)"
  [[ -n "$value" ]] || { echo "The env file is missing $key." >&2; exit 1; }
  [[ "$value" != *replace-with* ]] || { echo "The env file still contains a placeholder for $key." >&2; exit 1; }
done
artifact_root="$(sed -n 's/^ASTER_OPERATIONS_ARTIFACT_ROOT=//p' "$env_source" | tail -n1)"
if [[ "$artifact_root" != /* ]]; then
  artifact_root="$install_root/$artifact_root"
fi
artifact_root="$(realpath -m -- "$artifact_root")"
data_root="$install_root/data"
[[ "$artifact_root" == "$data_root"/* ]] || { echo 'ASTER_OPERATIONS_ARTIFACT_ROOT must resolve below <install-root>/data/.' >&2; exit 1; }

if ! id aster-operations >/dev/null 2>&1; then
  useradd --system --home-dir "$data_root" --shell /usr/sbin/nologin aster-operations
fi
release_dir="$install_root/releases/$version"
config_root="$install_root/config"
backup_root="$install_root/backups"
installed_env="$config_root/operations-api.env"
install -d -m 0755 "$install_root" "$install_root/releases" "$release_dir" "$release_dir/bin"
install -d -m 0755 "$service_registration_root"
install -d -o root -g aster-operations -m 0750 "$config_root"
install -d -o aster-operations -g aster-operations -m 0750 "$data_root" "$backup_root"
install -d -o aster-operations -g aster-operations -m 0750 "$artifact_root" "$artifact_root/inbox" "$artifact_root/objects"
env_staging="$(mktemp "$config_root/.operations-api.env.XXXXXX")"
awk -v artifact_root="$artifact_root" '
  /^ASTER_OPERATIONS_ARTIFACT_ROOT=/ { print "ASTER_OPERATIONS_ARTIFACT_ROOT=" artifact_root; next }
  { print }
' "$env_source" > "$env_staging"
chown aster-operations:aster-operations "$env_staging"
chmod 0600 "$env_staging"
mv -f -- "$env_staging" "$installed_env"
install -m 0755 "$bundle_root/bin/aster-operations-api" "$release_dir/bin/aster-operations-api"
install -m 0755 "$bundle_root/bin/aster-operations-backup" "$release_dir/bin/aster-operations-backup"
install -m 0755 "$bundle_root/bin/aster-webhost" "$release_dir/bin/aster-webhost"
cp -a "$bundle_root/console" "$release_dir/console"
install -m 0644 "$bundle_root/VERSION" "$release_dir/VERSION"
chown -R root:root "$release_dir"
ln -sfn "$release_dir" "$install_root/current.new"
mv -Tf "$install_root/current.new" "$install_root/current"

migrate_args=(--env-file="$installed_env" --migrate-only)
if [[ $create_database -eq 1 ]]; then migrate_args+=(--create-database); fi
runuser -u aster-operations -- "$install_root/current/bin/aster-operations-api" "${migrate_args[@]}"
for service in aster-operations-api.service aster-operations-web.service; do
  service_staging="$(mktemp "$service_registration_root/.${service}.XXXXXX")"
  sed "s|@ASTER_OPERATIONS_ROOT@|$install_root|g" "$bundle_root/systemd/$service" > "$service_staging"
  chmod 0644 "$service_staging"
  mv -f -- "$service_staging" "$service_registration_root/$service"
done
systemctl daemon-reload
systemctl enable --now aster-operations-api.service aster-operations-web.service

echo "Aster Operations $version is installed."
echo "Root: $install_root"
echo 'Local console: http://127.0.0.1:12080/'
echo 'Use an authenticated TLS reverse proxy before exposing it beyond this machine.'
