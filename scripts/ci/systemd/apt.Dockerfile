ARG BASE_IMAGE=ubuntu:24.04
FROM ${BASE_IMAGE}

ARG DEBIAN_FRONTEND=noninteractive
ARG APT_MIRROR=
ARG APT_SECURITY_MIRROR=
ENV container=docker

RUN if [ -n "$APT_SECURITY_MIRROR" ]; then \
      find /etc/apt -type f \( -name '*.list' -o -name '*.sources' \) -exec sed -i \
        -e "s|http://security.ubuntu.com/ubuntu|$APT_SECURITY_MIRROR|g" \
        -e "s|http://deb.debian.org/debian-security|$APT_SECURITY_MIRROR|g" {} +; \
    fi \
    && if [ -n "$APT_MIRROR" ]; then \
      find /etc/apt -type f \( -name '*.list' -o -name '*.sources' \) -exec sed -i \
        -e "s|http://archive.ubuntu.com/ubuntu|$APT_MIRROR|g" \
        -e "s|http://deb.debian.org/debian|$APT_MIRROR|g" {} +; \
    fi \
    && apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 update \
    && apt-get -o Acquire::Retries=2 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30 install --yes --no-install-recommends \
      ca-certificates \
      coreutils \
      curl \
      findutils \
      gawk \
      grep \
      gzip \
      iproute2 \
      openssl \
      passwd \
      procps \
      sed \
      sudo \
      systemd \
      systemd-sysv \
      tar \
      util-linux \
    && apt-get clean \
    && rm -rf /var/lib/apt/lists/* \
    && systemctl mask dev-hugepages.mount sys-fs-fuse-connections.mount
RUN truncate -s 0 /etc/machine-id && rm -f /var/lib/dbus/machine-id
RUN mkdir -p /usr/share/aster-ci \
    && dpkg-query -W -f='${Package}\t${Version}\n' | sort > /usr/share/aster-ci/packages.tsv

STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
