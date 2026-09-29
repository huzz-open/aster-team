#!/usr/bin/env sh
set -eu
. /etc/os-release
test "$ID" = "$1"
case "$VERSION_ID" in "$2"|"$2".*) ;; *) echo 'Unexpected runtime OS version' >&2; exit 1;; esac
for tool in cc gcc clang make cargo rustc node npm go pkg-config; do
  if command -v "$tool" >/dev/null 2>&1; then echo "Compiler/toolchain in runtime environment: $tool" >&2; exit 1; fi
done
actual=$(mktemp)
trap 'rm -f -- "$actual"' EXIT
if command -v dpkg-query >/dev/null; then
  dpkg-query -W -f='${Package}\t${Version}\n' | sort > "$actual"
else
  rpm -qa --qf '%{NAME}\t%{VERSION}-%{RELEASE}\n' | sort > "$actual"
fi
expected_digest=$(sha256sum /usr/share/aster-ci/packages.tsv)
actual_digest=$(sha256sum "$actual")
test "${expected_digest%% *}" = "${actual_digest%% *}" || {
  echo 'Runtime packages changed during test. Fix the delivery, not the test environment.' >&2
  exit 1
}
