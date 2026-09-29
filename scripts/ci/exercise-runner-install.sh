#!/usr/bin/env bash
set -Eeuo pipefail

usage() {
  echo 'Usage: exercise-runner-install.sh --package-dir PATH --version VERSION' >&2
}

package_dir=''
version=''
while [[ $# -gt 0 ]]; do
  case "$1" in
    --package-dir) package_dir=${2-}; shift 2 ;;
    --version) version=${2-}; shift 2 ;;
    -h|--help) usage; exit 0 ;;
    *) echo "Unknown argument: $1" >&2; usage; exit 2 ;;
  esac
done

[[ -n "$package_dir" && -d "$package_dir" ]] || { echo 'A package directory is required.' >&2; exit 2; }
[[ "$version" =~ ^[0-9]+\.[0-9]+\.[0-9]+([-+][0-9A-Za-z.-]+)?$ ]] || {
  echo "Invalid release version: $version" >&2
  exit 2
}

package_dir="$(cd -- "$package_dir" && pwd -P)"
archive="$package_dir/aster-team-${version}-linux-amd64.tar.gz"
checksum="$archive.sha256"
bundle="$package_dir/aster-team-${version}-linux-amd64"
[[ -f "$archive" && -f "$checksum" ]] || { echo 'Runner package or checksum is missing.' >&2; exit 2; }

(
  cd -- "$package_dir"
  sha256sum --check "$(basename -- "$checksum")"
)
rm -rf -- "$bundle"
tar -xzf "$archive" -C "$package_dir"
[[ -d "$bundle" ]] || { echo 'Extracted Runner bundle is missing.' >&2; exit 1; }
install_root='/srv/aster-team-runner'
cli_path='/usr/local/bin/aster-team-cli'
if getent group aster-team >/dev/null; then
  echo 'Runner-only smoke must begin without the Control service group.' >&2
  exit 1
fi
sudo "$bundle/init.sh" --install-root "$install_root"
sudo "$cli_path" runner install
test "$(sudo cat "$install_root/config/runner/install-role")" = 'runner'
test "$(sudo stat -c '%U:%G:%a' "$install_root/config/runner/install-role")" = 'root:aster-runner:640'
sudo systemctl is-enabled --quiet aster-runner.service
if sudo systemctl is-active --quiet aster-runner.service; then
  echo 'Unconfigured Runner service must not be started.' >&2
  exit 1
fi
test ! -e '/etc/systemd/system/aster-control@.service'
test ! -e "$install_root/data/database/aster-team.db"

backup="/root/aster-team-runner-${version}-backup.tar.gz"
sudo rm -f -- "$backup"
sudo "$cli_path" runner backup create --output "$backup"
sudo "$bundle/init.sh" --install-root "$install_root"
if sudo "$cli_path" runner upgrade; then
  echo 'A same-version Runner candidate unexpectedly upgraded.' >&2
  exit 1
fi
sudo "$cli_path" runner backup restore --source "$backup" --confirm
test "$(sudo cat "$install_root/config/runner/install-role")" = 'runner'
if sudo systemctl is-active --quiet aster-runner.service; then
  echo 'Restored unconfigured Runner service must not be started.' >&2
  exit 1
fi

echo "Runner-only install smoke test passed for Aster Team $version."
