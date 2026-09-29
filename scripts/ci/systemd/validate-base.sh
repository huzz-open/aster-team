set -eu
. /etc/os-release
test "$ID" = "$1"
case "$VERSION_ID" in "$2"|"$2".*) ;; *) echo 'Unexpected OS version' >&2; exit 1;; esac
test -x /sbin/init
test ! -s /etc/machine-id
for tool in bash systemctl systemd-run curl openssl tar gzip awk grep sed find sudo ip ps mount sha256sum; do
  command -v "$tool" >/dev/null
done
test ! -e /opt/aster
test ! -e /data/aster-team
test ! -e /root/.docker/config.json
test ! -e /root/.ssh/id_rsa
systemctl --version
for tool in cc gcc clang make cargo rustc node npm go pkg-config; do
  if command -v "$tool" >/dev/null 2>&1; then echo "Compiler/toolchain in runtime image: $tool" >&2; exit 1; fi
done
if command -v dpkg-query >/dev/null; then
  dpkg-query -W -f='${Package}\t${Version}\n' | sort > /tmp/actual-packages
else
  rpm -qa --qf '%{NAME}\t%{VERSION}-%{RELEASE}\n' | sort > /tmp/actual-packages
fi
expected=$(sha256sum /usr/share/aster-ci/packages.tsv)
actual=$(sha256sum /tmp/actual-packages)
test "${expected%% *}" = "${actual%% *}"
