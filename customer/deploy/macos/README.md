# Aster Team macOS package

This signed offline package contains the complete Aster Team runtime for one macOS CPU architecture. From the extracted directory, run:

```sh
sudo ./init-macos.sh
sudo aster-team-cli install
```

The signed layout contract selects the default installation root. Choose another absolute root only during bootstrap:

```sh
sudo ./init-macos.sh --install-root '/Volumes/Data/Aster Team'
```

All programs, releases, configuration, keys, SQLCipher data, state, logs, restore workspaces and backups remain below that root. `/Library/LaunchDaemons` and `/usr/local/bin` contain only platform registrations that reference the selected root.

The macOS lifecycle is implemented but is not declared production-verified until it has passed installation, upgrade and rollback tests on real macOS hosts.
