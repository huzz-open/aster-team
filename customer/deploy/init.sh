#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

fail() {
  local code="$1"
  local number="$2"
  shift 2
  printf '%s（%s / 错误码：%s）\n' "$*" "$code" "$number" >&2
  exit 1
}

install_root=''
runner_only=0
while (($#)); do
  case "$1" in
    --install-root) install_root="${2:-}"; shift 2 ;;
    --runner-only) runner_only=1; shift ;;
    -h|--help)
      printf '%s\n' '用法: sudo ./init.sh [--install-root 绝对路径] [--runner-only]'
      exit 0
      ;;
    *) fail DELIVERY_INPUT_INVALID 43007 "未知参数：$1。" ;;
  esac
done
[[ -z "$install_root" || "$install_root" == /* ]] || fail DELIVERY_INPUT_INVALID 43007 '--install-root 必须是绝对路径。'
[[ ${EUID} -eq 0 ]] || fail DELIVERY_ROOT_REQUIRED 43001 '请使用 sudo ./init.sh。'
[[ "$(uname -s)" == Linux && "$(uname -m)" == x86_64 ]] || fail DELIVERY_PLATFORM_UNSUPPORTED 43010 '仅支持 Linux x86-64 主机。'
for command in systemctl runuser flock getent groupadd useradd nologin; do
  command -v "$command" >/dev/null 2>&1 || fail DELIVERY_PLATFORM_UNSUPPORTED 43010 "当前主机缺少必需命令：$command。"
done
[[ -d /run/systemd/system ]] || fail DELIVERY_PLATFORM_UNSUPPORTED 43010 '当前主机未运行 systemd，无法安装 Aster Team 服务。'

bundle_root="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd -P)"
for path in RELEASE.json VERSION bin/aster-team-cli; do
  [[ -f "$bundle_root/$path" ]] || fail DELIVERY_RELEASE_INVALID 43003 "发布包缺少 $path。"
done
[[ -x "$bundle_root/bin/aster-team-cli" ]] || fail DELIVERY_RELEASE_INVALID 43003 '包内 aster-team-cli 不可执行。'

bootstrap=("$bundle_root/bin/aster-team-cli" bootstrap --release-root "$bundle_root")
[[ -z "$install_root" ]] || bootstrap+=(--install-root "$install_root")
[[ $runner_only -eq 0 ]] || bootstrap+=(--runner-only)
exec "${bootstrap[@]}"
