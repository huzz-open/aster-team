#!/bin/sh
set -eu
umask 077

fail() { echo "$*" >&2; exit 1; }
backup=''
install_root=''
service_registration_root=''
confirmed=0
runner_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --backup) backup=${2-}; shift 2 ;;
    --install-root) install_root=${2-}; shift 2 ;;
    --service-registration-root) service_registration_root=${2-}; shift 2 ;;
    --confirm-restore) confirmed=1; shift ;;
    --runner-only) runner_only=1; shift ;;
    *) fail "unknown internal restore option: $1" ;;
  esac
done
[ "$confirmed" -eq 1 ] || fail '--confirm-restore is required.'
[ "$(id -u)" -eq 0 ] || fail 'Run restore with sudo.'
[ "$(uname -s)" = Darwin ] || fail 'This restore engine requires macOS.'
case "$install_root" in /*) ;; *) fail '--install-root must be absolute.' ;; esac
[ "$install_root" != / ] || fail 'filesystem root is not an installation root.'
case "$service_registration_root" in /*) ;; *) fail '--service-registration-root must be absolute.' ;; esac
[ "$service_registration_root" != / ] || fail 'filesystem root is not a service registration directory.'
case "$backup" in /*) ;; *) fail '--backup must be absolute.' ;; esac
[ -f "$backup" ] && [ ! -L "$backup" ] || fail 'backup must be a regular file.'

parent=$(dirname -- "$install_root")
root_name=$(basename -- "$install_root")
restore_id="$$-$(date +%s)"
workspace="$install_root/staging/restores/$restore_id"
prepared_root="$workspace/prepared"
previous="$workspace/previous"
failed="$workspace/failed"
safety="$install_root/backups/aster-team-before-restore-$restore_id.tar.gz"
lock="$install_root/staging/restore.lock"
mkdir -p "$install_root/staging/restores" "$install_root/backups"
mkdir "$lock" 2>/dev/null || fail 'another restore is already running.'
preserve_workspace=0
cleanup() {
  rmdir "$lock" >/dev/null 2>&1 || true
  [ "$preserve_workspace" -eq 1 ] || rm -rf -- "$workspace"
}
trap cleanup EXIT HUP INT TERM
mkdir -p "$prepared_root" "$previous" "$failed"

tar -tzf "$backup" | while IFS= read -r entry; do
  case "$entry" in
    "$root_name"|"$root_name/"|"$root_name/"*) ;;
    *) fail "backup contains an unsafe path: $entry" ;;
  esac
  case "/$entry/" in */../*) fail "backup contains traversal: $entry" ;; esac
done
tar -xzf "$backup" -C "$prepared_root"
prepared="$prepared_root/$root_name"
marker="$prepared/install.json"
[ -f "$marker" ] || fail 'backup installation marker is missing.'
marker_root=$(plutil -extract root raw -o - "$marker")
marker_platform=$(plutil -extract platform raw -o - "$marker")
[ "$marker_root" = "$install_root" ] && [ "$marker_platform" = macos ] || fail 'backup belongs to another installation root or platform.'
target=$(readlink "$prepared/current")
case "$target" in "$install_root/releases/"*) ;; *) fail 'backup current release leaves the installation root.' ;; esac
version=$(tr -d '\r\n' < "$prepared/${target#"$install_root/"}/VERSION")
case "$target" in "$install_root/releases/$version") ;; *) fail 'backup active release metadata is inconsistent.' ;; esac
"$prepared/bin/aster-team-cli" verify-release --root "$prepared/releases/$version"
active=$(plutil -extract slot raw -o - "$prepared/state/slots/active.json" 2>/dev/null || true)
if [ "$runner_only" -eq 0 ]; then
  [ "$active" = blue ] || [ "$active" = green ] || fail 'backup active slot metadata is invalid.'
fi
tar -C "$parent" -czf "$safety" --exclude="$root_name/backups/*" --exclude="$root_name/staging/*" "$root_name"

labels='com.aster-team.control.blue com.aster-team.control.green com.aster-team.runner com.aster-team.caddy com.aster-team.maintenance'
xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}
sed_replacement_escape() {
  printf '%s' "$1" | sed -e 's/[\&|]/\\&/g'
}
stop_services() {
  for label in $labels; do launchctl bootout "system/$label" >/dev/null 2>&1 || true; done
}
install_services() {
  root=$1
  active=$(plutil -extract slot raw -o - "$root/state/slots/active.json" 2>/dev/null || true)
  if [ "$runner_only" -eq 0 ]; then
  [ "$active" = blue ] || [ "$active" = green ] || return 1
  fi
  plist_root=$(sed_replacement_escape "$(xml_escape "$root")")
  for label in $labels; do
    sed "s|@ASTER_ROOT@|$plist_root|g" "$root/current/launchd/$label.plist" > "$service_registration_root/$label.plist.tmp.$$"
    chmod 0644 "$service_registration_root/$label.plist.tmp.$$"
    chown root:wheel "$service_registration_root/$label.plist.tmp.$$"
    mv -f "$service_registration_root/$label.plist.tmp.$$" "$service_registration_root/$label.plist"
    plutil -lint "$service_registration_root/$label.plist" >/dev/null
    launchctl bootstrap system "$service_registration_root/$label.plist"
    launchctl disable "system/$label"
  done
  if [ "$runner_only" -eq 1 ]; then
    launchctl enable system/com.aster-team.runner
    [ -f "$root/config/runner/identity.json" ] && launchctl kickstart -k system/com.aster-team.runner
    return 0
  fi
  launchctl enable "system/com.aster-team.control.$active"
  launchctl enable system/com.aster-team.caddy
  launchctl enable system/com.aster-team.maintenance
  [ -f "$root/config/runner/identity.json" ] && launchctl enable system/com.aster-team.runner
  launchctl kickstart -k "system/com.aster-team.control.$active"
  launchctl kickstart -k system/com.aster-team.caddy
  [ -f "$root/config/runner/identity.json" ] && launchctl kickstart -k system/com.aster-team.runner
  if [ "$active" = blue ]; then health_port=11382; else health_port=11482; fi
  deadline=$(( $(date +%s) + 90 ))
  until curl -fsS "http://127.0.0.1:$health_port/healthz" >/dev/null 2>&1; do
    [ "$(date +%s)" -lt "$deadline" ] || return 1
    sleep 1
  done
}

managed_entries='bin releases config data state logs current install.json'
move_entries() {
  source_root=$1
  destination_root=$2
  for entry in $managed_entries; do
    source="$source_root/$entry"
    destination="$destination_root/$entry"
    if [ -e "$source" ] || [ -L "$source" ]; then
      mv "$source" "$destination"
    fi
  done
}

stop_services
if ! move_entries "$install_root" "$previous"; then
  if ! move_entries "$previous" "$install_root" || ! install_services "$install_root"; then
    preserve_workspace=1
    fail "could not stage the live installation and automatic rollback is incomplete; recovery files remain inside the installation root at $workspace"
  fi
  fail 'could not stage the live installation; the original installation was returned.'
fi
if ! move_entries "$prepared" "$install_root"; then
  move_entries "$install_root" "$failed" || true
  move_entries "$previous" "$install_root" || true
  fail 'could not activate restored installation.'
fi
if ! install_services "$install_root"; then
  stop_services
  if ! move_entries "$install_root" "$failed" || ! move_entries "$previous" "$install_root"; then
    preserve_workspace=1
    fail "restored services failed and automatic rollback is incomplete; recovery files remain inside the installation root at $workspace"
  fi
  if install_services "$install_root"; then
    rm -rf -- "$failed"
    fail 'restored services failed health checks; the original installation was restored.'
  fi
  preserve_workspace=1
  fail "restored services and the original services both failed; recovery files remain inside the installation root at $workspace"
fi
rm -rf -- "$previous"
echo "Backup restored to $install_root."
