ARG NODE_IMAGE=node:22.19.0-bookworm-slim@sha256:4a4884e8a44826194dff92ba316264f392056cbe243dcc9fd3551e71cea02b90
ARG GO_IMAGE=golang:1.25.14-bookworm@sha256:3b4a11519ad929d1e1d261a12cff056f0c85b735253d7d861346b9c6f8b36437
ARG RUST_IMAGE=rust:1.95.0-slim-bookworm@sha256:d7482085ff5b415f84dba5647ae71606650bdef00db7aeb69f4b3d170c3e4082

FROM ${NODE_IMAGE} AS node_runtime
FROM ${GO_IMAGE} AS go_runtime
FROM ${RUST_IMAGE}

ARG DEBIAN_MIRROR=
ARG DEBIAN_SECURITY_MIRROR=
ARG RUSTUP_DIST_SERVER=https://static.rust-lang.org

COPY --from=node_runtime /usr/local/bin/node /usr/local/bin/node
COPY --from=node_runtime /usr/local/lib/node_modules /usr/local/lib/node_modules
COPY --from=go_runtime /usr/local/go /usr/local/go

ENV PATH="/usr/local/go/bin:/usr/local/cargo/bin:${PATH}"
ARG DEBIAN_FRONTEND=noninteractive
RUN if [ -n "$DEBIAN_SECURITY_MIRROR" ]; then \
      sed -i "s|http://deb.debian.org/debian-security|$DEBIAN_SECURITY_MIRROR|g" /etc/apt/sources.list.d/debian.sources; \
    fi \
    && if [ -n "$DEBIAN_MIRROR" ]; then \
      sed -i "s|http://deb.debian.org/debian|$DEBIAN_MIRROR|g" /etc/apt/sources.list.d/debian.sources; \
    fi \
    && ln -s /usr/local/lib/node_modules/npm/bin/npm-cli.js /usr/local/bin/npm \
    && ln -s /usr/local/lib/node_modules/npm/bin/npx-cli.js /usr/local/bin/npx \
    && apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 update \
    && apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 install --yes --no-install-recommends \
      build-essential \
      binutils \
      ca-certificates \
      curl \
      git \
      musl-tools \
      openssl \
      perl \
      pkg-config \
      python3 \
      tar \
    && rm -rf /var/lib/apt/lists/* \
    && RUSTUP_DIST_SERVER="$RUSTUP_DIST_SERVER" rustup component add clippy rustfmt \
    && RUSTUP_DIST_SERVER="$RUSTUP_DIST_SERVER" rustup target add x86_64-unknown-linux-musl

WORKDIR /workspace
CMD ["bash"]
