#!/usr/bin/env bash
set -Eeuo pipefail

output=${1:-linux-diagnostics}
mkdir -p -- "$output"
for unit in aster-control@blue.service aster-control@green.service aster-caddy.service aster-runner.service aster-runner@blue.service aster-runner@green.service aster-upgrade.path aster-upgrade.service; do
  sudo systemctl status "$unit" --no-pager > "$output/${unit}.status.txt" 2>&1 || true
  sudo journalctl -u "$unit" --no-pager -n 500 > "$output/${unit}.journal.txt" 2>&1 || true
done
install_root=${ASTER_INSTALL_ROOT:-}
if [[ -z "$install_root" ]] && command -v aster-team-cli >/dev/null 2>&1; then
  cli_target="$(readlink -f -- "$(command -v aster-team-cli)")"
  install_root="$(dirname -- "$(dirname -- "$cli_target")")"
fi
if [[ "$install_root" == /* && "$install_root" != / ]]; then
  sudo find "$install_root" \
    -maxdepth 3 -printf '%M %u:%g %p\n' > "$output/filesystem.txt" 2>&1 || true
else
  printf 'Aster Team installation root could not be discovered.\n' > "$output/filesystem.txt"
fi
echo "Linux diagnostics collected in $output."
