#!/bin/sh
set -eu
umask 077

service=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --service) service=${2-}; shift 2 ;;
    *) echo "unknown service launcher argument: $1" >&2; exit 2 ;;
  esac
done

script_dir=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
install_root=$(CDPATH= cd -- "$script_dir/../.." && pwd -P)
marker="$install_root/install.json"
[ -f "$marker" ] || { echo 'installation marker is missing' >&2; exit 1; }
marker_root=$(/usr/bin/plutil -extract root raw -o - "$marker")
marker_platform=$(/usr/bin/plutil -extract platform raw -o - "$marker")
[ "$marker_root" = "$install_root" ] && [ "$marker_platform" = macos ] || {
  echo 'installation marker does not match this service launcher' >&2
  exit 1
}

load_environment() {
  file=$1
  allowed=$2
  [ -f "$file" ] || return 0
  while IFS='=' read -r key value || [ -n "$key$value" ]; do
    [ -z "$key$value" ] && continue
    case "$key" in \#*) continue ;; esac
    case " $allowed " in
      *" $key "*) ;;
      *) echo "unsupported environment key in $file" >&2; exit 1 ;;
    esac
    [ -n "$value" ] || { echo "empty environment value in $file" >&2; exit 1; }
    export "$key=$value"
  done < "$file"
}

case "$service" in
  control-blue|control-green)
    slot=${service#control-}
    release="$install_root/state/slots/$slot-release"
    load_environment "$install_root/config/control/control.env" 'ASTER_CONTROL_ALLOW_INSECURE_HTTP ASTER_CONTROL_SECURE_COOKIES'
    load_environment "$install_root/config/control/control-$slot.env" 'ASTER_CONTROL_LISTEN ASTER_CONTROL_MEMBER_LISTEN ASTER_CONTROL_ADMIN_LISTEN'
    "$release/bin/aster-control" verify-machine-identity
    exec "$release/bin/aster-control" serve \
      --database-driver sqlcipher \
      --admin-assets "$release/admin" \
      --member-assets "$release/member"
    ;;
  runner)
    load_environment "$install_root/config/runner/runner.env" 'ASTER_RUNNER_CONTROL_WSS ASTER_RUNNER_ALLOW_INSECURE_HTTP ASTER_RUNNER_CONTROL_CA_CERTIFICATE ASTER_RUNNER_UPSTREAM_CA_CERTIFICATE'
    exec "$install_root/current/bin/aster-runner" serve \
      --control-wss "$ASTER_RUNNER_CONTROL_WSS" \
      --allowed-upstream-host api.openai.com \
      --allowed-upstream-host auth.openai.com \
      --allowed-upstream-host api.anthropic.com \
      --allowed-upstream-host chatgpt.com \
      --allowed-upstream-host api.deepseek.com \
      --allowed-upstream-host open.bigmodel.cn \
      --allowed-upstream-host api.z.ai
    ;;
  caddy)
    export HOME="$install_root/data/caddy"
    export XDG_DATA_HOME="$install_root/data/caddy"
    export XDG_CONFIG_HOME="$install_root/config/caddy"
    exec "$install_root/bin/caddy" run --environ --config "$install_root/config/caddy/Caddyfile" --adapter caddyfile
    ;;
  maintenance)
    exec "$install_root/bin/aster-team-cli" maintenance run-next
    ;;
  *) echo "unknown Aster Team service: $service" >&2; exit 2 ;;
esac
