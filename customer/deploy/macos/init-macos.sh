#!/bin/sh
set -eu

install_root=''
while [ "$#" -gt 0 ]; do
  case "$1" in
    --install-root) install_root=${2-}; shift 2 ;;
    -h|--help) echo 'Usage: sudo ./init-macos.sh [--install-root ABSOLUTE_PATH]'; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done
[ "$(id -u)" -eq 0 ] || { echo 'Run init-macos.sh with sudo.' >&2; exit 1; }
[ "$(uname -s)" = Darwin ] || { echo 'This package requires macOS.' >&2; exit 1; }
bundle_root=$(CDPATH= cd -- "$(dirname -- "$0")" && pwd -P)
for relative in RELEASE.json VERSION bin/aster-team-cli; do
  [ -f "$bundle_root/$relative" ] || { echo "signed package is incomplete: $relative" >&2; exit 1; }
done
set -- "$bundle_root/bin/aster-team-cli" bootstrap --release-root "$bundle_root"
if [ -n "$install_root" ]; then
  case "$install_root" in /*) ;; *) echo '--install-root must be absolute.' >&2; exit 2 ;; esac
  set -- "$@" --install-root "$install_root"
fi
exec "$@"
