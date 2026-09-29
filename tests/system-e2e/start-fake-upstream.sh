#!/usr/bin/env sh
set -eu

test -s /certs/ca.crt
test -s /certs/server.crt
test -s /certs/server.key

exec node /opt/aster-e2e/fake-upstream.mjs
