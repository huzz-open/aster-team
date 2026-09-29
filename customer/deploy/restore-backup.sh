#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

usage() {
  cat <<'EOF'
Internal Aster Team restore engine.

Use `aster-team-cli backup restore --source FILE --confirm`. The archive must
have been created for the same immutable installation root. Restored contents
are validated before live services are stopped and swapped inside that root.

Options:
  --backup FILE       Backup archive to restore.
  --install-root DIR  Immutable Aster Team installation root.
  --service-registration-root DIR
                      Platform service registration directory.
  --runner-only       Restore a dedicated Runner host.
  --confirm-restore   Required destructive-operation confirmation.
EOF
}

backup=''
install_root=''
service_registration_root=''
runner_only=0
confirmed=0
while (($#)); do
  case "$1" in
    --backup) backup="${2:-}"; shift 2 ;;
    --install-root) install_root="${2:-}"; shift 2 ;;
    --service-registration-root) service_registration_root="${2:-}"; shift 2 ;;
    --runner-only) runner_only=1; shift ;;
    --confirm-restore) confirmed=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown option: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ ${EUID} -eq 0 ]] || { echo 'Run the restore tool as root (sudo).' >&2; exit 1; }
[[ $confirmed -eq 1 ]] || { echo '--confirm-restore is required.' >&2; exit 2; }
[[ "$install_root" =~ ^/[A-Za-z0-9._/+:-]+$ && "$install_root" != '/' ]] || { echo '--install-root must be a non-root absolute path without whitespace.' >&2; exit 2; }
[[ "$service_registration_root" =~ ^/[A-Za-z0-9._/+:-]+$ && "$service_registration_root" != '/' ]] || { echo '--service-registration-root must be a non-root absolute path without whitespace.' >&2; exit 2; }
install_root="${install_root%/}"
service_registration_root="${service_registration_root%/}"
for command in awk basename chmod chown cp date dirname find flock getent id install mktemp mv readlink rm runuser stat systemctl tar tr; do
  command -v "$command" >/dev/null 2>&1 || { echo "Missing required system command: $command" >&2; exit 1; }
done
[[ -f "$backup" && ! -L "$backup" ]] || { echo 'Backup must be a regular, non-symlinked file.' >&2; exit 2; }
backup="$(readlink -f -- "$backup")"
[[ "$(stat -c '%u' -- "$backup")" == 0 ]] || { echo 'Backup must be owned by root.' >&2; exit 1; }
backup_mode="$(stat -c '%a' -- "$backup")"
(( (8#$backup_mode & 077) == 0 )) || { echo 'Backup must not be accessible by group or other users.' >&2; exit 1; }

marker="$install_root/install.json"
current_link="$install_root/current"
[[ -f "$marker" && ! -L "$marker" && -L "$current_link" ]] || { echo 'A complete Aster Team installation root is required.' >&2; exit 1; }
parent="$(dirname -- "$install_root")"
root_name="$(basename -- "$install_root")"
install -d -o root -g root -m 0700 "$install_root/staging/restores" "$install_root/backups"
workspace="$(mktemp -d "$install_root/staging/restores/restore.XXXXXX")"
stage="$workspace/prepared"
mkdir -m 0700 "$stage"
list_file="$workspace/archive.list"
: > "$list_file"
prepared="$stage/$root_name"
previous="$workspace/previous"
failed="$workspace/failed"
safety="$install_root/backups/aster-team-before-restore-$(date -u +%Y%m%dT%H%M%SZ).$$.tar.gz"
unit_backup="$workspace/systemd-before-restore"
unit_names=(aster-control@.service aster-runner.service aster-runner@.service aster-caddy.service aster-upgrade.path aster-upgrade.service)
preserve_workspace=0

safe_remove() {
  local path="$1"
  case "$path" in
    "$install_root"/staging/restores/restore.*)
      [[ ! -e "$path" && ! -L "$path" ]] || rm -rf -- "$path"
      ;;
    *) echo "Refusing unsafe temporary cleanup: $path" >&2; return 1 ;;
  esac
}
cleanup() {
  if [[ $preserve_workspace -eq 0 ]]; then
    safe_remove "$workspace" || true
  fi
}
trap cleanup EXIT

mkdir -m 0700 "$unit_backup"
for unit in "${unit_names[@]}"; do
  unit_path="$service_registration_root/$unit"
  if [[ -f "$unit_path" && ! -L "$unit_path" ]]; then
    cp -a -- "$unit_path" "$unit_backup/$unit"
  elif [[ -e "$unit_path" || -L "$unit_path" ]]; then
    echo "Service unit path is unsafe: $unit_path" >&2
    exit 1
  else
    : > "$unit_backup/$unit.missing"
  fi
done

install_release_units() {
  local release="$1" unit source temporary
  for unit in "${unit_names[@]}"; do
    source="$release/systemd/$unit"
    if [[ "$unit" == 'aster-runner@.service' && ! -e "$source" && ! -L "$source" ]]; then
      if [[ -e "$service_registration_root/$unit" || -L "$service_registration_root/$unit" ]]; then
        [[ -f "$service_registration_root/$unit" && ! -L "$service_registration_root/$unit" ]] || { echo 'Unsafe slot Runner service path.' >&2; return 1; }
      fi
      rm -f -- "$service_registration_root/$unit"
      continue
    fi
    [[ -f "$source" && ! -L "$source" ]] || { echo "Release service unit is unavailable: $source" >&2; return 1; }
    temporary="$(mktemp "$workspace/unit.XXXXXX")"
    sed "s|@ASTER_ROOT@|$install_root|g" "$source" > "$temporary" || { rm -f -- "$temporary"; return 1; }
    install -o root -g root -m 0644 "$temporary" "$service_registration_root/$unit" || { rm -f -- "$temporary"; return 1; }
    rm -f -- "$temporary"
  done
  systemctl daemon-reload
}

restore_previous_units() {
  local unit
  for unit in "${unit_names[@]}"; do
    if [[ -f "$unit_backup/$unit" ]]; then
      install -o root -g root -m 0644 "$unit_backup/$unit" "$service_registration_root/$unit"
    else
      rm -f -- "$service_registration_root/$unit"
    fi
  done
  systemctl daemon-reload
}

tar -tzf "$backup" > "$list_file"
[[ -s "$list_file" ]] || { echo 'Backup archive is empty.' >&2; exit 1; }
while IFS= read -r raw_entry; do
  entry="${raw_entry#./}"
  [[ -n "$entry" && "$entry" != /* && "$entry" != '../'* && "$entry" != *'/../'* && "$entry" != *'/..' && "$entry" != *'//'* ]] || {
    echo 'Backup contains an unsafe path.' >&2
    exit 1
  }
  [[ "$entry" == "$root_name" || "$entry" == "$root_name/" || "$entry" == "$root_name/"* ]] || {
    echo 'Backup does not contain exactly the configured installation root.' >&2
    exit 1
  }
done < "$list_file"

tar --extract --gzip --file="$backup" --directory="$stage" \
  --no-same-owner --no-same-permissions --delay-directory-restore --restrict
[[ -d "$prepared" && ! -L "$prepared" && -f "$prepared/install.json" ]] || { echo 'Backup installation root is incomplete.' >&2; exit 1; }
unexpected_type="$(find "$prepared" -xdev ! -type d ! -type f ! -type l -print -quit)"
[[ -z "$unexpected_type" ]] || { echo "Backup contains an unsupported file type: $unexpected_type" >&2; exit 1; }
hard_link="$(find "$prepared" -xdev -type f -links +1 -print -quit)"
[[ -z "$hard_link" ]] || { echo "Backup contains a hard-linked file: $hard_link" >&2; exit 1; }
while IFS= read -r link; do
  relative="${link#"$prepared"/}"
  case "$relative" in
    current|state/slots/blue-release|state/slots/green-release) ;;
    *) echo "Backup contains an unexpected symbolic link: $relative" >&2; exit 1 ;;
  esac
  target="$(readlink -- "$link")"
  release_name="${target#"$install_root/releases/"}"
  [[ "$target" == "$install_root/releases/$release_name" && "$release_name" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
    echo "Backup link does not reference one direct release: $relative" >&2
    exit 1
  }
done < <(find "$prepared" -xdev -type l -print)

marker_root="$(awk -F'"' '$2 == "root" { print $4; exit }' "$prepared/install.json")"
marker_platform="$(awk -F'"' '$2 == "platform" { print $4; exit }' "$prepared/install.json")"
[[ "$marker_root" == "$install_root" && "$marker_platform" == linux ]] || { echo 'Backup belongs to a different installation root or platform.' >&2; exit 1; }
requested_role=control
role_file="$prepared/config/control/install-role"
[[ $runner_only -eq 0 ]] || { requested_role=runner; role_file="$prepared/config/runner/install-role"; }
[[ -f "$role_file" && "$(tr -d '\r\n' < "$role_file")" == "$requested_role" ]] || { echo 'Backup role does not match this host.' >&2; exit 1; }
[[ -x "$prepared/bin/aster-team-cli" && -L "$prepared/current" ]] || { echo 'Backup does not contain the stable CLI and current release link.' >&2; exit 1; }
active_slot=''
if [[ $runner_only -eq 0 ]]; then
  active_slot="$(awk -F'"' '$2 == "slot" { print $4; exit }' "$prepared/state/slots/active.json" 2>/dev/null || true)"
  [[ "$active_slot" == blue || "$active_slot" == green ]] || { echo 'Backup active release slot is invalid.' >&2; exit 1; }
  verified_release="$(readlink -- "$prepared/state/slots/$active_slot-release")"
  verified_version="$(awk -F'"' '$2 == "version" { print $4; exit }' "$prepared/state/slots/active.json")"
  [[ "$verified_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ && "$verified_release" == "$install_root/releases/$verified_version" ]] || { echo 'Backup active release metadata is inconsistent.' >&2; exit 1; }
else
  verified_release="$(readlink -- "$prepared/current")"
  [[ "$verified_release" == "$install_root/releases/"* ]] || { echo 'Backup current Runner release leaves the installation root.' >&2; exit 1; }
  verified_version="$(tr -d '\r\n' < "$prepared/${verified_release#"$install_root"/}/VERSION")"
  [[ "$verified_version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ && "$verified_release" == "$install_root/releases/$verified_version" ]] || { echo 'Backup current Runner release metadata is inconsistent.' >&2; exit 1; }
fi
[[ -f "$prepared/releases/$verified_version/RELEASE.json" ]] || { echo 'Backup active release is incomplete.' >&2; exit 1; }
"$install_root/bin/aster-team-cli" verify-release --root "$prepared/releases/$verified_version"

getent group aster-runner >/dev/null && id aster-runner >/dev/null 2>&1 || { echo 'Runner service account is unavailable.' >&2; exit 1; }
chown -R root:root "$prepared"
find "$prepared" -xdev -type d -exec chmod 0755 {} +
find "$prepared" -xdev -type f -exec chmod 0644 {} +
chown -R root:root "$prepared/config/cli"
find "$prepared/config/cli" -xdev -type d -exec chmod 0700 {} +
find "$prepared/config/cli" -xdev -type f -exec chmod 0600 {} +
chmod 0755 \
  "$prepared/bin/aster-team-cli" \
  "$prepared/releases/$verified_version/bin/aster-control" \
  "$prepared/releases/$verified_version/bin/aster-runner" \
  "$prepared/releases/$verified_version/bin/caddy"
chown -R root:aster-runner "$prepared/config/runner"
find "$prepared/config/runner" -xdev -type d -exec chmod 0750 {} +
find "$prepared/config/runner" -xdev -type f -exec chmod 0640 {} +
if [[ $runner_only -eq 0 ]]; then
  getent group aster-team >/dev/null && id aster-team >/dev/null 2>&1 || { echo 'Control service account is unavailable.' >&2; exit 1; }
  getent group aster-caddy >/dev/null && id aster-caddy >/dev/null 2>&1 || { echo 'Caddy service account is unavailable.' >&2; exit 1; }
  chown -R root:aster-team "$prepared/config/control" "$prepared/config/keys"
  chown -R root:aster-caddy "$prepared/config/tls"
  chown -R aster-team:aster-team "$prepared/config/license"
  chown -R aster-team:aster-team "$prepared/data/database" "$prepared/data/runtime" "$prepared/state" "$prepared/logs" "$prepared/staging"
  chown -R root:aster-caddy "$prepared/config/caddy"
  chown -R aster-caddy:aster-caddy "$prepared/data/caddy"
  find "$prepared/config/control" "$prepared/config/keys" "$prepared/config/tls" -xdev -type d -exec chmod 0750 {} +
  find "$prepared/config/control" "$prepared/config/keys" "$prepared/config/tls" -xdev -type f -exec chmod 0640 {} +
  if [[ -f "$prepared/config/tls/caddy-root.crt" ]]; then
    chown root:root "$prepared/config/tls/caddy-root.crt"
    chmod 0644 "$prepared/config/tls/caddy-root.crt"
  fi
  find "$prepared/config/license" "$prepared/data/database" "$prepared/data/runtime" "$prepared/state" "$prepared/logs" "$prepared/staging" -xdev -type d -exec chmod 0750 {} +
  find "$prepared/config/license" "$prepared/data/database" "$prepared/data/runtime" "$prepared/state" "$prepared/logs" "$prepared/staging" -xdev -type f -exec chmod 0640 {} +
  install -d -o aster-team -g aster-team -m 0700 "$prepared/data/settlements"
  chown -R aster-team:aster-team "$prepared/data/settlements"
  find "$prepared/data/settlements" -xdev -type d -exec chmod 0700 {} +
  find "$prepared/data/settlements" -xdev -type f -exec chmod 0600 {} +
  chmod 2750 "$prepared/config/license"
  find "$prepared/config/caddy" -xdev -type d -exec chmod 0750 {} +
  find "$prepared/config/caddy" -xdev -type f -exec chmod 0640 {} +
  chown aster-team:aster-team "$prepared/config/keys/database.key"
  chmod 0600 "$prepared/config/keys/database.key"
fi

control_unit=''
[[ $runner_only -eq 1 ]] || control_unit="aster-control@$active_slot.service"
original_control_unit=''
if [[ $runner_only -eq 0 ]]; then
  original_active_slot="$(awk -F'"' '$2 == "slot" { print $4; exit }' "$install_root/state/slots/active.json" 2>/dev/null || true)"
  [[ "$original_active_slot" == blue || "$original_active_slot" == green ]] || { echo 'Live active release slot is invalid.' >&2; exit 1; }
  original_control_unit="aster-control@$original_active_slot.service"
fi
control_was_active=0
runner_was_active=0
caddy_was_active=0
upgrade_path_was_active=0
[[ $runner_only -eq 1 ]] || { systemctl is-active --quiet "$original_control_unit" && control_was_active=1 || true; }
systemctl is-active --quiet aster-runner.service && runner_was_active=1 || true
[[ $runner_only -eq 1 ]] || { systemctl is-active --quiet aster-caddy.service && caddy_was_active=1 || true; }
[[ $runner_only -eq 1 ]] || { systemctl is-active --quiet aster-upgrade.path && upgrade_path_was_active=1 || true; }

tar -C "$parent" -czf "$safety" \
  --exclude="$root_name/backups/*" \
  --exclude="$root_name/staging/*" \
  "$root_name"
tar -tzf "$safety" >/dev/null
chmod 0600 "$safety"

managed_entries=(bin releases config data state logs current install.json)
mkdir -m 0700 "$previous" "$failed"
move_entries() {
  local source_root="$1" destination_root="$2" entry source destination
  for entry in "${managed_entries[@]}"; do
    source="$source_root/$entry"
    destination="$destination_root/$entry"
    if [[ -e "$source" || -L "$source" ]]; then
      mv -- "$source" "$destination" || return 1
    fi
  done
}

restart_original_services() {
  local restored=1
  restore_previous_units || restored=0
  if [[ $runner_only -eq 0 ]]; then
    systemctl disable --now "aster-control@$([[ "$original_active_slot" == blue ]] && echo green || echo blue).service" >/dev/null 2>&1 || true
    systemctl enable "$original_control_unit" aster-caddy.service aster-upgrade.path >/dev/null 2>&1 || restored=0
  fi
  [[ $runner_was_active -eq 0 ]] || systemctl enable aster-runner.service >/dev/null 2>&1 || restored=0
  [[ $control_was_active -eq 0 ]] || systemctl start "$original_control_unit" || restored=0
  [[ $caddy_was_active -eq 0 ]] || systemctl start aster-caddy.service || restored=0
  [[ $runner_was_active -eq 0 ]] || systemctl start aster-runner.service || restored=0
  [[ $upgrade_path_was_active -eq 0 ]] || systemctl start aster-upgrade.path || restored=0
  [[ $restored -eq 1 ]]
}

systemctl stop aster-upgrade.path aster-runner@blue.service aster-runner@green.service aster-runner.service aster-caddy.service aster-control@blue.service aster-control@green.service >/dev/null 2>&1 || true
if ! move_entries "$install_root" "$previous"; then
  if ! move_entries "$previous" "$install_root" || ! restart_original_services; then
    preserve_workspace=1
    echo "Could not stage the live installation and automatic rollback is incomplete; recovery files remain inside the installation root at $workspace." >&2
  else
    echo 'Could not stage the live installation; the original installation was returned.' >&2
  fi
  exit 1
fi
if ! move_entries "$prepared" "$install_root"; then
  move_entries "$install_root" "$failed" || true
  if ! move_entries "$previous" "$install_root" || ! restart_original_services; then
    preserve_workspace=1
    echo "Could not activate the restored installation and automatic rollback is incomplete; recovery files remain inside the installation root at $workspace." >&2
  else
    rm -rf -- "$failed"
    echo 'Could not place the restored installation contents; the original installation was returned.' >&2
  fi
  exit 1
fi

restart_restored=1
if ! install_release_units "$install_root/releases/$verified_version"; then
  restart_restored=0
fi
if [[ $runner_only -eq 0 ]]; then
  systemctl disable --now "aster-control@$([[ "$active_slot" == blue ]] && echo green || echo blue).service" >/dev/null 2>&1 || true
  systemctl enable "$control_unit" aster-caddy.service aster-upgrade.path >/dev/null 2>&1 || restart_restored=0
  if [[ -f "$install_root/config/runner/identity.json" && -f "$install_root/config/runner/runner.env" && -f "$install_root/config/runner/task-keys.json" ]]; then
    systemctl enable aster-runner.service >/dev/null 2>&1 || restart_restored=0
  else
    systemctl disable aster-runner.service >/dev/null 2>&1 || true
  fi
elif ! systemctl enable aster-runner.service >/dev/null 2>&1; then
  restart_restored=0
fi
[[ $control_was_active -eq 0 ]] || systemctl start "$control_unit" || restart_restored=0
[[ $caddy_was_active -eq 0 ]] || systemctl start aster-caddy.service || restart_restored=0
[[ $runner_was_active -eq 0 ]] || systemctl start aster-runner.service || restart_restored=0
[[ $upgrade_path_was_active -eq 0 ]] || systemctl start aster-upgrade.path || restart_restored=0
if [[ $restart_restored -eq 1 && $control_was_active -eq 1 ]]; then
  export ASTER_INSTALL_ROOT="$install_root"
  source "$install_root/current/libexec/service-health.sh"
  deadline="$(aster_health_new_deadline)"
  aster_wait_for_control_health "$deadline" || restart_restored=0
  [[ $restart_restored -eq 0 ]] || aster_wait_for_access_health "$deadline" "$install_root/config/control/access.json" || restart_restored=0
fi

if [[ $restart_restored -ne 1 ]]; then
  systemctl stop aster-upgrade.path aster-runner@blue.service aster-runner@green.service aster-runner.service aster-caddy.service aster-control@blue.service aster-control@green.service >/dev/null 2>&1 || true
  if ! move_entries "$install_root" "$failed" || ! move_entries "$previous" "$install_root" || ! restart_original_services; then
    preserve_workspace=1
    echo "Restored data failed health checks and automatic rollback is incomplete; recovery files remain inside the installation root at $workspace." >&2
  else
    rm -rf -- "$failed"
    echo 'Restored data failed health checks; the original installation was restored.' >&2
  fi
  exit 1
fi

rm -rf -- "$previous" "$failed"
trap - EXIT
cleanup
echo "Aster Team $requested_role backup restored successfully."
echo "Pre-restore safety backup retained at $safety"
