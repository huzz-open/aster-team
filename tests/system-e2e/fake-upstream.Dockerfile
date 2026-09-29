FROM node:22.19.0-bookworm-slim@sha256:4a4884e8a44826194dff92ba316264f392056cbe243dcc9fd3551e71cea02b90

COPY tests/system-e2e/fake-upstream.mjs /opt/aster-e2e/fake-upstream.mjs
COPY tests/system-e2e/start-fake-upstream.sh /opt/aster-e2e/start-fake-upstream.sh
RUN chmod 0755 /opt/aster-e2e/start-fake-upstream.sh

EXPOSE 443
ENTRYPOINT ["/opt/aster-e2e/start-fake-upstream.sh"]
