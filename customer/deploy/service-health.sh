#!/usr/bin/env bash

# Shared readiness policy for the private Customer lifecycle engines. This file
# is sourced by install.sh and restore-backup.sh, and can also be executed by
# the signed-package smoke test through its private wait-access entry point.

ASTER_HEALTH_TIMEOUT_SECONDS=60
ASTER_HEALTH_INITIAL_DELAY_SECONDS=1
ASTER_HEALTH_MAX_DELAY_SECONDS=5
ASTER_HEALTH_REQUEST_TIMEOUT_SECONDS=4
ASTER_INSTALL_ROOT="${ASTER_INSTALL_ROOT:-}"

aster_health_new_deadline() {
  printf '%d\n' "$((SECONDS + ASTER_HEALTH_TIMEOUT_SECONDS))"
}

aster_health_print_last_error() {
  local error_file="$1"
  [[ ! -s "$error_file" ]] || awk '{ print "  " $0 }' "$error_file" >&2
}

aster_wait_until_ready() {
  local deadline="$1"
  local description="$2"
  local probe="$3"
  shift 3
  local delay=$ASTER_HEALTH_INITIAL_DELAY_SECONDS
  local attempt=1
  local remaining=0
  local wait_seconds=0
  local probe_status=0
  local error_file=''

  [[ -n "$ASTER_INSTALL_ROOT" && "$ASTER_INSTALL_ROOT" == /* ]] || {
    echo 'ASTER_INSTALL_ROOT is required for readiness checks.' >&2
    return 1
  }

  [[ "$deadline" =~ ^[0-9]+$ ]] || {
    echo "Invalid readiness deadline for $description." >&2
    return 1
  }
  mkdir -p "$ASTER_INSTALL_ROOT/data/runtime"
  error_file="$(mktemp "$ASTER_INSTALL_ROOT/data/runtime/health.XXXXXX")"
  while true; do
    remaining=$((deadline - SECONDS))
    if (( remaining <= 0 )); then
      break
    fi
    : > "$error_file"
    if "$probe" "$remaining" "$error_file" "$@"; then
      rm -f -- "$error_file"
      return 0
    else
      probe_status=$?
    fi
    if [[ $probe_status -eq 2 ]]; then
      echo "$description readiness check failed with a non-retryable error." >&2
      aster_health_print_last_error "$error_file"
      rm -f -- "$error_file"
      return 1
    fi
    remaining=$((deadline - SECONDS))
    if (( remaining <= 0 )); then
      break
    fi
    wait_seconds=$delay
    (( wait_seconds <= remaining )) || wait_seconds=$remaining
    echo "$description is not ready (attempt $attempt); retrying in ${wait_seconds}s." >&2
    sleep "$wait_seconds"
    attempt=$((attempt + 1))
    if (( delay < ASTER_HEALTH_MAX_DELAY_SECONDS )); then
      delay=$((delay * 2))
      (( delay <= ASTER_HEALTH_MAX_DELAY_SECONDS )) || delay=$ASTER_HEALTH_MAX_DELAY_SECONDS
    fi
  done

  echo "$description did not become ready within ${ASTER_HEALTH_TIMEOUT_SECONDS}s." >&2
  aster_health_print_last_error "$error_file"
  rm -f -- "$error_file"
  return 1
}

aster_probe_regular_file() {
  local _remaining="$1"
  local error_file="$2"
  local path="$3"
  local description="$4"
  if [[ -f "$path" && ! -L "$path" ]]; then
    return 0
  fi
  if [[ -e "$path" || -L "$path" ]]; then
    printf '%s exists but is not a regular, non-symlinked file: %s\n' "$description" "$path" > "$error_file"
    return 2
  fi
  printf '%s is not present yet: %s\n' "$description" "$path" > "$error_file"
  return 1
}

aster_wait_for_regular_file() {
  local deadline="$1"
  local path="$2"
  local description="$3"
  aster_wait_until_ready "$deadline" "$description" aster_probe_regular_file "$path" "$description"
}

aster_health_request_timeout() {
  local remaining="$1"
  local timeout=$ASTER_HEALTH_REQUEST_TIMEOUT_SECONDS
  (( timeout <= remaining )) || timeout=$remaining
  (( timeout >= 1 )) || timeout=1
  printf '%d\n' "$timeout"
}

aster_probe_control_health() {
  local remaining="$1"
  local error_file="$2"
  local timeout=''
  local http_code=''
  local curl_status=0
  local active_slot=''
  local admin_port=''
  active_slot="$(awk -F'"' '$2 == "slot" { print $4; exit }' "$ASTER_INSTALL_ROOT/state/slots/active.json" 2>/dev/null || true)"
  case "$active_slot" in
    blue) admin_port=11382 ;;
    green) admin_port=11482 ;;
    *) printf 'Active Control slot is unavailable.\n' > "$error_file"; return 1 ;;
  esac
  timeout="$(aster_health_request_timeout "$remaining")"
  http_code="$(curl --silent --show-error --noproxy '*' \
    --connect-timeout "$timeout" --max-time "$timeout" \
    --output /dev/null --write-out '%{http_code}' \
    "http://127.0.0.1:$admin_port/healthz" 2>"$error_file")" || curl_status=$?
  if [[ $curl_status -eq 0 && "$http_code" =~ ^2[0-9][0-9]$ ]]; then
    return 0
  fi
  printf 'Local Control health check failed (curl exit %s, HTTP %s).\n' \
    "$curl_status" "${http_code:-000}" >> "$error_file"
  return 1
}

aster_wait_for_control_health() {
  local deadline="$1"
  aster_wait_until_ready "$deadline" 'Local Control' aster_probe_control_health
}

aster_active_control_unit() {
  local slot=''
  slot="$(awk -F'"' '$2 == "slot" { print $4; exit }' "$ASTER_INSTALL_ROOT/state/slots/active.json" 2>/dev/null || true)"
  case "$slot" in
    blue|green) printf 'aster-control@%s.service\n' "$slot" ;;
    *) return 1 ;;
  esac
}

aster_health_json_string() {
  local path="$1"
  local key="$2"
  awk -F'"' -v key="$key" '
    $2 == key { count++; value=$4 }
    END { if (count != 1 || value == "") exit 1; print value }
  ' "$path"
}

aster_health_json_boolean() {
  local path="$1"
  local key="$2"
  awk -v expected="\"$key\":" '
    $1 == expected { count++; value=$2; sub(/,$/, "", value) }
    END {
      if (count != 1 || (value != "true" && value != "false")) exit 1
      print value
    }
  ' "$path"
}

aster_probe_access_health() {
  local remaining="$1"
  local error_file="$2"
  local access_config="$3"
  local protocol=''
  local address_kind=''
  local access_host=''
  local bind_address=''
  local certificate_source=''
  local caddy_enabled=''
  local api_url=''
  local member_url=''
  local admin_url=''
  local public_port=''
  local ca_certificate=''
  local timeout=''
  local component=''
  local endpoint_url=''
  local endpoint_host=''
  local http_code=''
  local curl_status=0
  local index=0
  local probe_started=$SECONDS
  local endpoint_remaining=0
  local -a components=()
  local -a endpoint_urls=()
  local -a endpoint_hosts=()
  local -a curl_args=()

  if [[ ! -f "$access_config" || -L "$access_config" ]]; then
    printf 'Access configuration is missing or unsafe: %s\n' "$access_config" > "$error_file"
    return 2
  fi
  if ! protocol="$(aster_health_json_string "$access_config" protocol)" ||
     ! address_kind="$(aster_health_json_string "$access_config" address_kind)" ||
     ! access_host="$(aster_health_json_string "$access_config" host)" ||
     ! bind_address="$(aster_health_json_string "$access_config" bind_address)" ||
     ! certificate_source="$(aster_health_json_string "$access_config" certificate_source)" ||
     ! caddy_enabled="$(aster_health_json_boolean "$access_config" caddy_enabled)" ||
     ! api_url="$(aster_health_json_string "$access_config" api_url)" ||
     ! member_url="$(aster_health_json_string "$access_config" member_url)" ||
     ! admin_url="$(aster_health_json_string "$access_config" admin_url)"; then
    printf 'Access configuration cannot be parsed for public health checks: %s\n' "$access_config" > "$error_file"
    return 2
  fi
  if [[ "$protocol" != http && "$protocol" != https ]]; then
    printf 'Access configuration has an unsupported protocol: %s\n' "$protocol" > "$error_file"
    return 2
  fi

  components=(api member admin)
  endpoint_urls=("$api_url" "$member_url" "$admin_url")
  if [[ "$address_kind" == domain ]]; then
    endpoint_hosts=("api.$access_host" "app.$access_host" "admin.$access_host")
    public_port=80
    [[ "$protocol" != https ]] || public_port=443
    if [[ "$caddy_enabled" != true ||
          "$api_url" != "$protocol://api.$access_host" ||
          "$member_url" != "$protocol://app.$access_host" ||
          "$admin_url" != "$protocol://admin.$access_host" ]]; then
      printf 'Domain access URLs do not match the canonical access configuration.\n' > "$error_file"
      return 2
    fi
  elif [[ "$address_kind" == ip ]]; then
    endpoint_hosts=("$access_host" "$access_host" "$access_host")
    if [[ "$api_url" != "$protocol://$access_host:11080" ||
          "$member_url" != "$protocol://$access_host:11081" ||
          "$admin_url" != "$protocol://$access_host:11082" ]]; then
      printf 'IP access URLs do not match the canonical access configuration.\n' > "$error_file"
      return 2
    fi
    if [[ "$protocol" == https && "$caddy_enabled" != true ]] ||
       [[ "$protocol" == http && "$caddy_enabled" != true ]]; then
      printf 'IP access Caddy state does not match the configured protocol.\n' > "$error_file"
      return 2
    fi
  else
    printf 'Access configuration has an unsupported address kind: %s\n' "$address_kind" > "$error_file"
    return 2
  fi

  if [[ "$protocol" == https ]]; then
    if [[ "$certificate_source" == caddy ]]; then
      ca_certificate="$ASTER_INSTALL_ROOT/config/tls/caddy-root.crt"
    elif [[ "$certificate_source" == provided ]]; then
      ca_certificate="$ASTER_INSTALL_ROOT/config/tls/server.crt"
    else
      printf 'HTTPS access has an unsupported certificate source: %s\n' "$certificate_source" > "$error_file"
      return 2
    fi
    if [[ ! -f "$ca_certificate" || -L "$ca_certificate" ]]; then
      printf 'Public health-check CA certificate is missing or unsafe: %s\n' "$ca_certificate" > "$error_file"
      return 2
    fi
  elif [[ "$certificate_source" != none ]]; then
    printf 'HTTP access must use certificate_source=none.\n' > "$error_file"
    return 2
  fi

  if [[ "$caddy_enabled" == true ]] && ! systemctl is-active --quiet aster-caddy.service; then
    printf 'Managed Caddy is not active yet.\n' > "$error_file"
    return 1
  fi

  for index in "${!components[@]}"; do
    endpoint_remaining=$((remaining - (SECONDS - probe_started)))
    if (( endpoint_remaining <= 0 )); then
      printf 'The public health-check deadline expired before all endpoints were ready.\n' > "$error_file"
      return 1
    fi
    timeout="$(aster_health_request_timeout "$endpoint_remaining")"
    component="${components[$index]}"
    endpoint_url="${endpoint_urls[$index]}"
    endpoint_host="${endpoint_hosts[$index]}"
    curl_args=(--silent --show-error --noproxy '*'
      --connect-timeout "$timeout" --max-time "$timeout"
      --output /dev/null --write-out '%{http_code}')
    [[ -z "$ca_certificate" ]] || curl_args+=(--cacert "$ca_certificate")
    [[ "$address_kind" != domain ]] || curl_args+=(--resolve "$endpoint_host:$public_port:$bind_address")
    : > "$error_file"
    curl_status=0
    http_code="$(curl "${curl_args[@]}" "$endpoint_url/healthz" 2>"$error_file")" || curl_status=$?
    if [[ $curl_status -eq 0 && "$http_code" =~ ^2[0-9][0-9]$ ]]; then
      continue
    fi
    printf 'Public %s health check failed: %s/healthz (curl exit %s, HTTP %s).\n' \
      "$component" "$endpoint_url" "$curl_status" "${http_code:-000}" >> "$error_file"
    case "$curl_status" in
      3|4|51|58|59|60|77) return 2 ;;
    esac
    if [[ $curl_status -eq 0 && ! "$http_code" =~ ^5[0-9][0-9]$ ]]; then
      return 2
    fi
    return 1
  done
  return 0
}

aster_wait_for_access_health() {
  local deadline="$1"
  local access_config="$2"
  aster_wait_until_ready "$deadline" 'Public Aster endpoints' aster_probe_access_health "$access_config"
}

aster_report_service_diagnostics() {
  local unit=''
  for unit in "$@"; do
    echo "===== $unit status =====" >&2
    systemctl --no-pager --full status "$unit" >&2 || true
    if command -v journalctl >/dev/null 2>&1; then
      echo "===== $unit recent journal =====" >&2
      journalctl --no-pager -u "$unit" -n 80 >&2 || true
    fi
  done
}

aster_health_usage() {
  echo 'Internal usage: service-health.sh wait-access ACCESS_JSON' >&2
}

aster_health_main() {
  [[ ${EUID} -eq 0 ]] || { echo 'Run the private health helper as root.' >&2; return 1; }
  [[ $# -eq 2 && "$1" == wait-access ]] || { aster_health_usage; return 2; }
  if [[ -z "$ASTER_INSTALL_ROOT" ]]; then
    ASTER_INSTALL_ROOT="$(cd -- "$(dirname -- "$2")/../.." && pwd -P)"
  fi
  for command in awk curl dirname mkdir mktemp rm sleep systemctl; do
    command -v "$command" >/dev/null 2>&1 || { echo "Missing required system command: $command" >&2; return 1; }
  done
  local deadline=''
  deadline="$(aster_health_new_deadline)"
  if ! aster_wait_for_access_health "$deadline" "$2"; then
    control_unit="$(aster_active_control_unit || echo 'aster-control@blue.service')"
    aster_report_service_diagnostics "$control_unit" aster-caddy.service
    return 1
  fi
}

if [[ "${BASH_SOURCE[0]}" == "$0" ]]; then
  set -Eeuo pipefail
  umask 077
  aster_health_main "$@"
fi
