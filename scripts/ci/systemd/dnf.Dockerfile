ARG BASE_IMAGE=rockylinux:9
FROM ${BASE_IMAGE}

ENV container=docker

RUN dnf --setopt=timeout=30 --setopt=retries=2 install --assumeyes \
      ca-certificates \
      findutils \
      gawk \
      grep \
      gzip \
      iproute \
      openssl \
      procps-ng \
      sed \
      shadow-utils \
      sudo \
      systemd \
      tar \
      util-linux \
    && dnf clean all \
    && rm -rf /var/cache/dnf \
    && systemctl mask dev-hugepages.mount sys-fs-fuse-connections.mount
RUN truncate -s 0 /etc/machine-id && rm -f /var/lib/dbus/machine-id
RUN mkdir -p /usr/share/aster-ci \
    && rpm -qa --qf '%{NAME}\t%{VERSION}-%{RELEASE}\n' | sort > /usr/share/aster-ci/packages.tsv

STOPSIGNAL SIGRTMIN+3
CMD ["/sbin/init"]
