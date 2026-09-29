#!/bin/sh
set -eu
umask 077

fail() { echo "$*" >&2; exit 1; }
install_root=''
service_registration_root=''
manifest_sha256=''
owner_email=''
owner_password_file=''
access_protocol='http'
access_host=''
bind_address=''
certificate_source=''
tls_certificate=''
tls_private_key=''
skip_owner=0
install_local_runner=0
recover_preserved=0
runner_only=0
while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-root) install_root=${2-}; shift 2 ;;
    --service-registration-root) service_registration_root=${2-}; shift 2 ;;
    --release-manifest-sha256) manifest_sha256=${2-}; shift 2 ;;
    --owner-email) owner_email=${2-}; shift 2 ;;
    --owner-password-file) owner_password_file=${2-}; shift 2 ;;
    --access-protocol) access_protocol=${2-}; shift 2 ;;
    --access-host) access_host=${2-}; shift 2 ;;
    --bind-address) bind_address=${2-}; shift 2 ;;
    --certificate-source) certificate_source=${2-}; shift 2 ;;
    --tls-certificate) tls_certificate=${2-}; shift 2 ;;
    --tls-private-key) tls_private_key=${2-}; shift 2 ;;
    --skip-owner) skip_owner=1; shift ;;
    --install-local-runner) install_local_runner=1; shift ;;
    --recover-preserved) recover_preserved=1; shift ;;
    --runner-only) runner_only=1; shift ;;
    *) fail "unknown internal installer option: $1" ;;
  esac
done

[ "$(id -u)" -eq 0 ] || fail 'Run the Aster Team installer with sudo.'
[ "$(uname -s)" = Darwin ] || fail 'This installer requires macOS.'
[ -n "$install_root" ] && [ "${install_root#/}" != "$install_root" ] && [ "$install_root" != / ] || fail '--install-root must be a non-root absolute path.'
case "$service_registration_root" in /*) ;; *) fail '--service-registration-root must be absolute.' ;; esac
[ "$service_registration_root" != / ] || fail 'filesystem root is not a service registration directory.'

caddy_escape() {
  printf '%s' "$1" | sed -e 's/\\/\\\\/g' -e 's/"/\\"/g'
}

bundle_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd -P)
for relative in VERSION RELEASE.json bin/aster-team-cli bin/aster-control bin/aster-runner bin/caddy macos/service-launch.sh; do
  [ -f "$bundle_root/$relative" ] || fail "signed package is incomplete: $relative"
done
version=$(tr -d '\r\n' < "$bundle_root/VERSION")
case "$version" in ''|*[!0-9A-Za-z.+-]*) fail 'VERSION is invalid.' ;; esac
actual_manifest_sha256=$(shasum -a 256 "$bundle_root/RELEASE.json" | awk '{print $1}')
if [ -e "$install_root/current" ]; then
  [ "$runner_only" -eq 1 ] || fail 'Control upgrades must be submitted through the maintenance upgrade flow.'
  [ -f "$install_root/config/runner/install-role" ] && [ "$(tr -d '\r\n' < "$install_root/config/runner/install-role")" = runner ] || fail 'The existing installation is not a dedicated Runner.'
  verifier_cli="$install_root/current/bin/aster-team-cli"
  [ -x "$verifier_cli" ] || fail 'The trusted installed CLI is unavailable.'
else
  [ "$actual_manifest_sha256" = "$manifest_sha256" ] || fail 'RELEASE.json does not match the separately supplied SHA-256.'
  verifier_cli="$bundle_root/bin/aster-team-cli"
fi
"$verifier_cli" verify-release --root "$bundle_root"

ensure_group() {
  name=$1
  if dscl . -read "/Groups/$name" >/dev/null 2>&1; then return 0; fi
  gid=420
  while dscl . -search /Groups PrimaryGroupID "$gid" | grep -q .; do gid=$((gid + 1)); done
  dscl . -create "/Groups/$name"
  dscl . -create "/Groups/$name" PrimaryGroupID "$gid"
}

ensure_user() {
  name=$1 group=$2 home=$3
  if dscl . -read "/Users/$name" >/dev/null 2>&1; then return 0; fi
  uid=420
  while dscl . -search /Users UniqueID "$uid" | grep -q .; do uid=$((uid + 1)); done
  gid=$(dscl . -read "/Groups/$group" PrimaryGroupID | awk '{print $2}')
  dscl . -create "/Users/$name"
  dscl . -create "/Users/$name" UniqueID "$uid"
  dscl . -create "/Users/$name" PrimaryGroupID "$gid"
  dscl . -create "/Users/$name" UserShell /usr/bin/false
  dscl . -create "/Users/$name" NFSHomeDirectory "$home"
  dscl . -create "/Users/$name" IsHidden 1
  dscl . -create "/Users/$name" RealName "$name service"
}

for relative in bin releases config/cli config/control config/runner config/caddy config/services config/keys config/license config/tls data/database data/runner data/caddy data/runtime data/settlements data/plugins/incoming data/plugins/versions state/plugins state/migrations state/upgrades/queued state/upgrades/running state/upgrades/completed state/slots state/locks staging/upgrades backups/upgrades logs; do
  install -d -m 0750 "$install_root/$relative"
done
ensure_group aster-team
ensure_user aster-team aster-team "$install_root/data"
ensure_group aster-runner
ensure_user aster-runner aster-runner "$install_root/data/runner"

release_directory="$install_root/releases/$version"
if [ -d "$release_directory" ]; then
  [ "$(shasum -a 256 "$release_directory/RELEASE.json" | awk '{print $1}')" = "$actual_manifest_sha256" ] || fail 'same version is already staged with different contents.'
  "$verifier_cli" verify-release --root "$release_directory"
else
  stage="$install_root/releases/.$version.$$"
  [ ! -e "$stage" ] || fail 'release staging directory already exists.'
  mkdir "$stage"
  trap 'rm -rf -- "$stage"' EXIT HUP INT TERM
  ditto "$bundle_root" "$stage"
  "$verifier_cli" verify-release --root "$stage"
  mv "$stage" "$release_directory"
  trap - EXIT HUP INT TERM
fi
install -m 0755 "$release_directory/bin/caddy" "$install_root/bin/caddy"

xml_escape() {
  printf '%s' "$1" | sed -e 's/&/\&amp;/g' -e 's/</\&lt;/g' -e 's/>/\&gt;/g'
}

sed_replacement_escape() {
  printf '%s' "$1" | sed -e 's/[\&|]/\\&/g'
}

install_plist() {
  label=$1
  source="$release_directory/launchd/$label.plist"
  destination="$service_registration_root/$label.plist"
  [ -f "$source" ] || fail "launchd definition is missing: $label"
  plist_root=$(sed_replacement_escape "$(xml_escape "$install_root")")
  sed "s|@ASTER_ROOT@|$plist_root|g" "$source" > "$destination.tmp.$$"
  chmod 0644 "$destination.tmp.$$"
  chown root:wheel "$destination.tmp.$$"
  mv -f "$destination.tmp.$$" "$destination"
  plutil -lint "$destination" >/dev/null
}

if [ "$runner_only" -eq 1 ]; then
  previous_release=''
  runner_upgrade_committed=0
  if [ -L "$install_root/current" ]; then
    previous_release=$(CDPATH= cd -- "$install_root/current" && pwd -P)
  fi
  rollback_runner_upgrade() {
    if [ -n "$previous_release" ] && [ "$runner_upgrade_committed" -eq 0 ]; then
      launchctl bootout system/com.aster-team.runner >/dev/null 2>&1 || true
      rollback_link="$install_root/.current-rollback.$$"
      rm -f "$rollback_link"
      ln -s "$previous_release" "$rollback_link"
      mv -fh "$rollback_link" "$install_root/current"
      install -m 0755 "$previous_release/macos/service-launch.sh" "$install_root/config/services/service-launch.sh"
      launchctl bootout system/com.aster-team.runner >/dev/null 2>&1 || true
      launchctl bootstrap system "$service_registration_root/com.aster-team.runner.plist" >/dev/null 2>&1 || true
      if [ -f "$install_root/config/runner/identity.json" ]; then
        launchctl enable system/com.aster-team.runner >/dev/null 2>&1 || true
        launchctl kickstart -k system/com.aster-team.runner >/dev/null 2>&1 || true
      fi
    fi
  }
  trap rollback_runner_upgrade EXIT HUP INT TERM
  printf '%s\n' runner > "$install_root/config/runner/install-role"
  launchctl bootout system/com.aster-team.runner >/dev/null 2>&1 || true
  install -m 0755 "$release_directory/macos/service-launch.sh" "$install_root/config/services/service-launch.sh"
  current_link="$install_root/.current-runner.$$"
  rm -f "$current_link"
  ln -s "$release_directory" "$current_link"
  mv -fh "$current_link" "$install_root/current"
  install_plist com.aster-team.runner
  chown -R aster-runner:aster-runner "$install_root/config/runner" "$install_root/data/runner"
  launchctl bootstrap system "$service_registration_root/com.aster-team.runner.plist"
  if [ -f "$install_root/config/runner/identity.json" ]; then
    launchctl enable system/com.aster-team.runner
    launchctl kickstart -k system/com.aster-team.runner
    echo "Aster Team Runner upgraded to $version under $install_root."
  else
    launchctl disable system/com.aster-team.runner
    echo "Aster Team Runner $version installed under $install_root; enroll it before starting."
  fi
  runner_upgrade_committed=1
  trap - EXIT HUP INT TERM
  exit 0
fi

[ ! -e "$install_root/current" ] || fail 'Aster Team is already installed; use the upgrade command.'
if [ "$recover_preserved" -eq 0 ]; then
  for provider in openai deepseek glm; do
    install -m 0640 "$release_directory/plugins/$provider.asterlua" \
      "$install_root/data/plugins/incoming/$provider.asterlua"
    chown aster-team:aster-team "$install_root/data/plugins/incoming/$provider.asterlua"
  done
fi
install -m 0755 "$release_directory/macos/service-launch.sh" "$install_root/config/services/service-launch.sh"
if [ "$recover_preserved" -eq 0 ] && [ "$skip_owner" -eq 0 ]; then
  [ -n "$owner_email" ] && [ -f "$owner_password_file" ] || fail 'owner email and password file are required.'
fi
printf '%s\n' control > "$install_root/config/control/install-role"

if [ "$recover_preserved" -eq 0 ]; then
  secure_cookies=false
  [ "$access_protocol" = https ] && secure_cookies=true
  printf 'ASTER_CONTROL_ALLOW_INSECURE_HTTP=false\nASTER_CONTROL_SECURE_COOKIES=%s\n' "$secure_cookies" > "$install_root/config/control/control.env"
  printf 'ASTER_CONTROL_LISTEN=127.0.0.1:11380\nASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:11381\nASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:11382\n' > "$install_root/config/control/control-blue.env"
  printf 'ASTER_CONTROL_LISTEN=127.0.0.1:11480\nASTER_CONTROL_MEMBER_LISTEN=127.0.0.1:11481\nASTER_CONTROL_ADMIN_LISTEN=127.0.0.1:11482\n' > "$install_root/config/control/control-green.env"

  case "$access_host" in *:*) address_kind=ip ;; *.*) if echo "$access_host" | grep -Eq '^[0-9.]+$'; then address_kind=ip; else address_kind=domain; fi ;; *) address_kind=domain ;; esac
  if [ "$address_kind" = domain ]; then
    member_url="$access_protocol://app.$access_host"; admin_url="$access_protocol://admin.$access_host"; api_url="$access_protocol://api.$access_host"
  else
    member_url="$access_protocol://$access_host:11081"; admin_url="$access_protocol://$access_host:11082"; api_url="$access_protocol://$access_host:11080"
  fi
  if [ "$access_protocol" = https ]; then runner_scheme=wss; else runner_scheme=ws; fi
  runner_wss="$runner_scheme://${api_url#*://}/api/runner/channel"
  cat > "$install_root/config/control/access.json" <<EOF
{"schema":"aster.team.access/v1","protocol":"$access_protocol","address_kind":"$address_kind","host":"$access_host","bind_address":"$bind_address","certificate_source":"$certificate_source","caddy_enabled":true,"member_url":"$member_url","admin_url":"$admin_url","api_url":"$api_url","runner_websocket_url":"$runner_wss"}
EOF
  cat > "$install_root/config/caddy/upstreams.caddy" <<'EOF'
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
  tls_directive=''
  if [ "$access_protocol" = https ] && [ "$certificate_source" = caddy ]; then
    tls_directive='tls internal'
  elif [ "$access_protocol" = https ]; then
    install -m 0600 "$tls_certificate" "$install_root/config/tls/server.crt"
    install -m 0600 "$tls_private_key" "$install_root/config/tls/server.key"
    tls_directive="tls \"$(caddy_escape "$install_root/config/tls/server.crt")\" \"$(caddy_escape "$install_root/config/tls/server.key")\""
  fi
  {
    printf '{\n\tadmin 127.0.0.1:2019\n\tpersist_config off\n\tauto_https disable_redirects\n}\n\nimport "%s/config/caddy/upstreams.caddy"\n\n' "$(caddy_escape "$install_root")"
    if [ "$address_kind" = domain ]; then
      for entry in "api.$access_host:aster_api_upstream" "app.$access_host:aster_member_upstream" "admin.$access_host:aster_admin_upstream"; do
        host=${entry%%:*}; upstream=${entry#*:}
        printf '%s://%s {\n\tbind %s\n' "$access_protocol" "$host" "$bind_address"
        [ -z "$tls_directive" ] || printf '\t%s\n' "$tls_directive"
        printf '\timport %s\n}\n' "$upstream"
        [ "$upstream" = aster_admin_upstream ] || printf '\n'
      done
    else
      for entry in '11080:aster_api_upstream' '11081:aster_member_upstream' '11082:aster_admin_upstream'; do
        port=${entry%%:*}; upstream=${entry#*:}
        printf '%s://%s:%s {\n\tbind %s\n' "$access_protocol" "$access_host" "$port" "$bind_address"
        [ -z "$tls_directive" ] || printf '\t%s\n' "$tls_directive"
        printf '\timport %s\n}\n' "$upstream"
        [ "$upstream" = aster_admin_upstream ] || printf '\n'
      done
    fi
  } > "$install_root/config/caddy/Caddyfile"

  chown -R aster-team:aster-team "$install_root/config/control" "$install_root/config/runner" "$install_root/config/keys" "$install_root/config/license" "$install_root/data/database" "$install_root/data/runtime" "$install_root/data/settlements" "$install_root/data/plugins" "$install_root/state"
  control="$release_directory/bin/aster-control"
  sudo -u aster-team "$control" initialize-installation
  sudo -u aster-team "$control" initialize-runner-task-key
  sudo -u aster-team "$control" export-runner-task-keys
  cp "$install_root/config/control/runner-task-keys.json" "$install_root/config/runner/task-keys.json"
  [ -f "$install_root/config/keys/database.key" ] || openssl rand -out "$install_root/config/keys/database.key" 32
  chown aster-team:aster-team "$install_root/config/keys/database.key"
  sudo -u aster-team "$control" initialize-database --database-driver sqlcipher
  sudo -u aster-team "$control" initialize-runtime-configuration --database-driver sqlcipher --public-api-base-url "$api_url"
  if [ "$skip_owner" -eq 0 ]; then
    password_copy="$install_root/config/control/.owner-password.$$"
    install -o aster-team -g aster-team -m 0600 "$owner_password_file" "$password_copy"
    sudo -u aster-team "$control" initialize-owner --database-driver sqlcipher --email "$owner_email" --password-file "$password_copy"
    password=$(tr -d '\r\n' < "$password_copy")
    rm -f "$password_copy"
    printf 'ASTER_OWNER_EMAIL=%s\nASTER_OWNER_TEMPORARY_PASSWORD=%s\n' "$owner_email" "$password" > "$install_root/config/control/initial-owner-credentials"
    chmod 0600 "$install_root/config/control/initial-owner-credentials"
  fi
  if [ "$install_local_runner" -eq 1 ]; then
    sudo -u aster-team "$control" initialize-local-runner --database-driver sqlcipher --owner-email "$owner_email" --name local-runner --identity-output "$install_root/config/runner/identity.json"
    printf 'ASTER_RUNNER_CONTROL_WSS=%s\n' "$runner_wss" > "$install_root/config/runner/runner.env"
    [ "$access_protocol" = http ] && printf 'ASTER_RUNNER_ALLOW_INSECURE_HTTP=true\n' >> "$install_root/config/runner/runner.env"
  fi
  printf '%s\n' 'aster.team.initialization-complete/v1' > "$install_root/state/initialization-complete"
else
  sudo -u aster-team "$release_directory/bin/aster-control" verify-machine-identity
  sudo -u aster-team "$release_directory/bin/aster-control" preflight --database-driver sqlcipher --admin-assets "$release_directory/admin" --member-assets "$release_directory/member"
fi

rm -f "$install_root/current" "$install_root/state/slots/blue-release"
ln -s "$release_directory" "$install_root/current"
ln -s "$release_directory" "$install_root/state/slots/blue-release"
printf '{"schema":"aster.active-release-slot.v1","slot":"blue","version":"%s"}\n' "$version" > "$install_root/state/slots/active.json"
if [ "$recover_preserved" -eq 0 ]; then
  "$install_root/bin/caddy" fmt --overwrite "$install_root/config/caddy/Caddyfile"
  "$install_root/bin/caddy" fmt --overwrite "$install_root/config/caddy/upstreams.caddy"
fi
"$install_root/bin/caddy" validate --config "$install_root/config/caddy/Caddyfile" --adapter caddyfile
for label in com.aster-team.control.blue com.aster-team.control.green com.aster-team.runner com.aster-team.caddy com.aster-team.maintenance; do install_plist "$label"; done
chown -R aster-team:aster-team "$install_root/data/database" "$install_root/data/runtime" "$install_root/data/settlements" "$install_root/data/plugins" "$install_root/state" "$install_root/logs"
chown -R aster-runner:aster-runner "$install_root/config/runner" "$install_root/data/runner"

for label in com.aster-team.control.blue com.aster-team.control.green com.aster-team.runner com.aster-team.caddy com.aster-team.maintenance; do
  launchctl bootout "system/$label" >/dev/null 2>&1 || true
  launchctl bootstrap system "$service_registration_root/$label.plist"
  launchctl disable "system/$label"
done
for label in com.aster-team.control.blue com.aster-team.caddy com.aster-team.maintenance; do launchctl enable "system/$label"; done
[ "$install_local_runner" -eq 1 ] && launchctl enable system/com.aster-team.runner
launchctl kickstart -k system/com.aster-team.control.blue
launchctl kickstart -k system/com.aster-team.caddy
[ "$install_local_runner" -eq 1 ] && launchctl kickstart -k system/com.aster-team.runner

deadline=$(( $(date +%s) + 90 ))
until curl -fsS http://127.0.0.1:11382/healthz >/dev/null 2>&1; do
  [ "$(date +%s)" -lt "$deadline" ] || fail 'Control health check timed out.'
  sleep 1
done
echo "Aster Team $version installed under $install_root."
