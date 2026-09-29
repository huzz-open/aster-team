#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  cat <<'EOF'
Usage: exercise-supported-linux-install.sh --target ID --mode MODE --role ROLE --package-dir PATH --version VERSION [options]

Options for container mode:
  --image IMAGE
  --package-family apt|dnf
  --expected-bundled-license-id ID
  --exercise-paid-license-switch
EOF
}

target=''
mode=''
role=''
package_dir=''
version=''
image=''
package_family=''
expected_bundled_license_id=''
exercise_paid_license_switch=0
while [[ $# -gt 0 ]]; do
  case "$1" in
    --target) target=${2-}; shift 2 ;;
    --mode) mode=${2-}; shift 2 ;;
    --role) role=${2-}; shift 2 ;;
    --package-dir) package_dir=${2-}; shift 2 ;;
    --version) version=${2-}; shift 2 ;;
    --image) image=${2-}; shift 2 ;;
    --package-family) package_family=${2-}; shift 2 ;;
    --expected-bundled-license-id) expected_bundled_license_id=${2-}; shift 2 ;;
    --exercise-paid-license-switch) exercise_paid_license_switch=1; shift ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage >&2; exit 2 ;;
  esac
done

[[ "$target" =~ ^[a-z0-9][a-z0-9.-]*$ ]] || { echo "Invalid smoke target: $target" >&2; exit 2; }
[[ "$mode" == 'host' || "$mode" == 'container' ]] || { echo "Invalid smoke mode: $mode" >&2; exit 2; }
[[ "$role" == 'customer' || "$role" == 'runner' ]] || { echo "Invalid install role: $role" >&2; exit 2; }
[[ -d "$package_dir" ]] || { echo "Package directory is missing: $package_dir" >&2; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || { echo "Invalid version: $version" >&2; exit 2; }

repo_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")/../.." && pwd -P)"
package_dir="$(cd -- "$package_dir" && pwd -P)"
smoke_script="exercise-${role}-install.sh"
archive="$package_dir/aster-team-${version}-linux-amd64.tar.gz"
node "$repo_root/scripts/ci/verify-static-archive.mjs" "$archive"
customer_access_arguments=()
if [[ "$role" == customer ]]; then
  case "$target" in
    ubuntu-20.04) customer_access_arguments=(--access-mode ip-http) ;;
    ubuntu-22.04) customer_access_arguments=(--access-mode ip-https-caddy) ;;
    ubuntu-24.04) customer_access_arguments=(--access-mode ip-https-provided) ;;
    debian-12) customer_access_arguments=(--access-mode domain-http) ;;
    debian-13) customer_access_arguments=(--access-mode domain-https-caddy) ;;
    rocky-linux-9) customer_access_arguments=(--access-mode domain-https-provided) ;;
    *) customer_access_arguments=(--access-mode ip-http) ;;
  esac
  if [[ -n "$expected_bundled_license_id" ]]; then
    customer_access_arguments+=(--expected-bundled-license-id "$expected_bundled_license_id")
  fi
elif [[ -n "$expected_bundled_license_id" ]]; then
  echo '--expected-bundled-license-id is valid only for the Customer role.' >&2
  exit 2
fi
if [[ $exercise_paid_license_switch -eq 1 && "$role" != customer ]]; then
  echo '--exercise-paid-license-switch is valid only for the Customer role.' >&2
  exit 2
fi

if [[ "$mode" == 'host' ]]; then
  [[ -z "$image" && -z "$package_family" ]] || { echo 'Host mode does not accept container settings.' >&2; exit 2; }
  if [[ $exercise_paid_license_switch -eq 1 ]]; then
    [[ -x "$package_dir/lablicensesigner" && -f "$package_dir/paid-license-private.pkcs8.base64url" ]] || {
      echo 'Paid License switch fixtures are missing from the package test directory.' >&2
      exit 2
    }
    customer_access_arguments+=(
      --lab-paid-license-signer "$package_dir/lablicensesigner"
      --lab-paid-license-signing-key "$package_dir/paid-license-private.pkcs8.base64url"
    )
  fi
  exec bash "$repo_root/scripts/ci/$smoke_script" --package-dir "$package_dir" --version "$version" "${customer_access_arguments[@]}"
fi

command -v docker >/dev/null 2>&1 || { echo 'Docker is required for container smoke targets.' >&2; exit 2; }
host_system="$(uname -s)"
windows_docker_host=0
case "$host_system" in
  Linux) ;;
  MINGW*|MSYS*|CYGWIN*) windows_docker_host=1 ;;
  *) echo "Unsupported Docker smoke host: $host_system" >&2; exit 2 ;;
esac
if [[ $windows_docker_host -eq 1 ]]; then
  command -v cygpath >/dev/null 2>&1 || {
    echo 'Windows smoke tests must run from Git Bash with cygpath available.' >&2
    exit 2
  }
fi
docker_command=(docker)
docker_cli() {
  if [[ $windows_docker_host -eq 1 ]]; then
    MSYS_NO_PATHCONV=1 "${docker_command[@]}" "$@"
  else
    "${docker_command[@]}" "$@"
  fi
}
docker_host_path() {
  if [[ $windows_docker_host -eq 1 ]]; then
    cygpath --absolute --mixed "$1"
  else
    printf '%s\n' "$1"
  fi
}
if ! docker_cli info >/dev/null 2>&1; then
  if command -v sudo >/dev/null 2>&1 && sudo -n docker info >/dev/null 2>&1; then
    docker_command=(sudo -n docker)
  else
    echo 'Docker is installed, but this user cannot access its daemon directly or through passwordless sudo.' >&2
    exit 2
  fi
fi
[[ -n "$image" ]] || { echo 'Container image is required.' >&2; exit 2; }
[[ "$package_family" == 'apt' || "$package_family" == 'dnf' ]] || { echo "Invalid package family: $package_family" >&2; exit 2; }

safe_run_id="${GITHUB_RUN_ID:-local}-$(printf '%s' "${GITHUB_JOB:-job}" | tr -c 'a-zA-Z0-9_.-' '-')"
container_name="aster-${target}-${role}-${safe_run_id}"
test_image="$(node "$repo_root/scripts/ci/ci-images.mjs" "$target" "$image" "$package_family")"
smoke_machine_id="$(printf '%s' "$container_name" | sha256sum | awk '{print substr($1, 1, 32)}')"
smoke_dmi_product_uuid="${smoke_machine_id:0:8}-${smoke_machine_id:8:4}-${smoke_machine_id:12:4}-${smoke_machine_id:16:4}-${smoke_machine_id:20:12}"
diagnostic_root="${SMOKE_DIAGNOSTIC_ROOT:-$repo_root/linux-diagnostics}"
diagnostic_dir="$diagnostic_root/${target}-${role}"
container_started=0
if [[ -n "${SMOKE_APT_MIRROR:-}" && -z "${SMOKE_APT_SECURITY_MIRROR:-}" ]] ||
   [[ -z "${SMOKE_APT_MIRROR:-}" && -n "${SMOKE_APT_SECURITY_MIRROR:-}" ]]; then
  echo 'SMOKE_APT_MIRROR and SMOKE_APT_SECURITY_MIRROR must be set together.' >&2
  exit 2
fi
if [[ -n "${SMOKE_APT_MIRROR:-}${SMOKE_APT_SECURITY_MIRROR:-}" ]]; then
  echo 'Runtime tests cannot modify package sources; update the pinned base with the local maintenance script.' >&2
  exit 2
fi
docker_package_dir="$(docker_host_path "$package_dir")"
docker_ci_dir="$(docker_host_path "$repo_root/scripts/ci")"

collect_diagnostics() {
  local exit_code=$?
  if [[ $exit_code -ne 0 && $container_started -eq 1 ]]; then
    mkdir -p -- "$diagnostic_dir"
    docker_cli logs "$container_name" >"$diagnostic_dir/container.log" 2>&1 || true
    docker_cli inspect "$container_name" >"$diagnostic_dir/container-inspect.json" 2>&1 || true
    docker_cli exec "$container_name" journalctl --no-pager -b >"$diagnostic_dir/journal.log" 2>&1 || true
    docker_cli exec "$container_name" systemctl --no-pager --full status aster-control@blue.service aster-control@green.service aster-caddy.service aster-runner.service aster-upgrade.path \
      >"$diagnostic_dir/aster-services.log" 2>&1 || true
    docker_cli exec "$container_name" sh -c 'cat /etc/os-release; uname -a' \
      >"$diagnostic_dir/platform.log" 2>&1 || true
  fi
  if [[ $container_started -eq 1 ]]; then
    docker_cli rm --force "$container_name" >/dev/null 2>&1 || true
  fi
  exit "$exit_code"
}
trap collect_diagnostics EXIT

if ! docker_cli image inspect "$test_image" >/dev/null 2>&1; then
  docker_cli pull --platform linux/amd64 "$test_image"
fi

docker_cli run --detach \
  --name "$container_name" \
  --hostname "$target" \
  --privileged \
  --cgroupns=host \
  --tmpfs /run \
  --tmpfs /run/lock \
  --tmpfs /sys:rw \
  --volume /sys/fs/cgroup:/sys/fs/cgroup:rw \
  --volume "$docker_package_dir:/workspace/package-input:ro" \
  --volume "$docker_ci_dir:/workspace/scripts/ci:ro" \
  "$test_image" >/dev/null
container_started=1

docker_cli exec "$container_name" bash -Eeuo pipefail -c '
  for _ in $(seq 1 60); do
    state=$(systemctl is-system-running 2>/dev/null || true)
    case "$state" in
      running|degraded) exit 0 ;;
      initializing|starting) sleep 1 ;;
      *) sleep 1 ;;
    esac
  done
  systemctl --no-pager --full status || true
  exit 1
'

docker_cli exec "$container_name" bash -Eeuo pipefail -c '
  if [[ ! -s /etc/machine-id ]]; then
    printf "%s\n" "$1" > /etc/machine-id
  fi
  mkdir -p /sys/class/dmi/id
  printf "%s\n" "$2" > /sys/class/dmi/id/product_uuid
  chmod 0400 /sys/class/dmi/id/product_uuid
  test -s /etc/machine-id
  test -s /sys/class/dmi/id/product_uuid
  [[ "$(stat -c %a /sys/class/dmi/id/product_uuid)" == 400 ]]
' bash "$smoke_machine_id" "$smoke_dmi_product_uuid"

docker_cli exec "$container_name" bash -Eeuo pipefail -c '
  mkdir -p /workspace/package
  cp -a /workspace/package-input/. /workspace/package/
'

if [[ $exercise_paid_license_switch -eq 1 ]]; then
  docker_cli exec "$container_name" test -x /workspace/package/lablicensesigner
  docker_cli exec "$container_name" test -f /workspace/package/paid-license-private.pkcs8.base64url
  customer_access_arguments+=(
    --lab-paid-license-signer /workspace/package/lablicensesigner
    --lab-paid-license-signing-key /workspace/package/paid-license-private.pkcs8.base64url
  )
fi

runtime_os="${target%%-*}"
runtime_version="${target##*-}"
docker_cli exec "$container_name" sh /workspace/scripts/ci/check-runtime-baseline.sh "$runtime_os" "$runtime_version"

docker_cli exec "$container_name" bash "/workspace/scripts/ci/$smoke_script" \
  --package-dir /workspace/package \
  --version "$version" \
  "${customer_access_arguments[@]}"

docker_cli exec "$container_name" sh /workspace/scripts/ci/check-runtime-baseline.sh "$runtime_os" "$runtime_version"

docker_cli exec "$container_name" sh -c 'cat /etc/os-release; uname -a; systemctl --version | head -n 1'
echo "$role install smoke test passed on $target."
