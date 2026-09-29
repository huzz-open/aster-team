---
name: build-aster-linux-amd64
description: Build and verify the signed Aster Team Rust-native Customer Linux amd64 tar.gz release. Use for production Customer builds, packaging, rebuilds, or releases; do not use for local demo, Operations, Windows, or website-only artifacts.
---

# Build Aster Linux amd64

Every production package must bundle the same signed, unbound v2 `free-license.json`. `release:tag` freezes its distribution ID and SHA-256 into the annotated tag; Actions rejects Environment bytes that do not match that tag. Do not create a bare release tag by hand.

After the version PR has merged into main and required verification passes, use `npm run release:tag -- --version VERSION --tag-only`. It pushes the annotated `vVERSION` tag for the already-committed version; do not also dispatch the workflow manually. The tag workflow validates the committed version and source, reuses all platform gates, and publishes both signed Customer packages and checksums to GitHub Release only after successful checks. Pre-release versions become GitHub Pre-releases. Use `--tag-only` for an already-pushed version; never move an existing tag or overwrite published assets. Retry failures through the existing Actions run. The `customer-release` Environment needs a tag-type `v*` rule and a `main` branch rule for verified recovery. See `README-LOCAL.md` under “Tag 一键发布到 GitHub Release” for recovery and public release downloads. Do not create a real release tag merely to test tooling changes.

When the user explicitly chooses a direct-to-main release with verification delegated to GitHub Actions, use `npm run release:github -- --version VERSION` from the clean primary `main` checkout. It synchronizes main, prepares and commits the four version files, pushes, and dispatches the same `customer-release.yml` workflow used by Operations with the exact source SHA. It runs no local verification suites and does not bypass Git hooks or remote protection. The workflow builds both Customer platforms; it does not create an Operations task record. For an already-pushed version, use `--dispatch-only`; an existing run is returned instead of dispatched again. See `README-LOCAL.md` under “GitHub Actions 一键出包” for prerequisites and recovery. This path does not change the repository's default PR or local-release rules.

For local builds, follow the instructions below.

Use `scripts/build-linux-amd64.sh`; do not reconstruct its lower-level environment variables or invoke `build-linux-bundle.mjs` directly.

On a Windows x64 release host, prepare a new version with `npm run release:prepare -- --version=VERSION`, review and commit the four resulting manifest/lockfile changes, and make that commit the synchronized `main`. Then use `npm run release:local -- --platform=linux` (or `--platform=all`); the entrypoint reads the committed version automatically. An optional `--version=VERSION` is an assertion only and must never override the Cargo package version. The optional `--build-id=ID` defaults to the 12-character HEAD SHA and identifies only logs and staging. Source verification defaults to `--verify=full`; an explicitly requested local package may use `--verify=changed`, `--verify=none`, or a fixed allowlist such as `--checks=lint,unit,e2e`. Skipping source verification never skips signing, SBOM, archive integrity, checksum, or platform smoke checks. The entrypoint validates the external security directory, discovers Docker Desktop and its configured credential helpers without changing the system `PATH`, preflights the digest-pinned images, and runs this same canonical script inside the pinned Docker toolchain. An `all` build must run from an Administrator PowerShell and requires a complete system Perl on `PATH` for the native Windows SQLCipher/OpenSSL build; the local entrypoint never downloads or installs Perl. Packages remain below `target/release-local/staging` until every selected platform smoke test passes, then move to `dist`; failures retain a run record below `target/release-local/runs` without publishing partial artifacts. `npm run test:linux:primary` uses ephemeral lab keys and never produces a deliverable package.

Before building:

1. Confirm the requested version is committed consistently in `package.json`, `package-lock.json`, `Cargo.toml`, and `Cargo.lock`. Use `release:prepare` when it differs. Never silently overwrite an artifact whose hash was already reported.
2. Confirm the build host is Linux amd64, Windows x64 with Docker Desktop, or Intel/Apple Silicon macOS with Docker Desktop, and the source worktree is clean. On non-Windows hosts, provide a same-commit native Windows x64 `asterctl.exe`. The canonical script also enforces `main` and optional `origin/main` parity unless its documented CI-only override is intentionally used.
3. Obtain explicit filesystem paths for:
   - the License trusted-public-key ring JSON;
   - the Release trusted-public-key ring JSON;
   - the repository-external raw 32-byte Ed25519 Release signing seed;
   - the Release `key_id` present in the Release keyring.
   - the exact signed, unbound v2 free-distribution document to bundle as `licenses/free-license.json`;
   - a native Windows x64 `asterctl.exe` built from the same source commit. On a Windows local release host, `release:local` builds this automatically; CI must pass its cross-job artifact explicitly.

Never derive or reuse the Release private key from Operations configuration. The License and Release public keys must be cryptographically different, and no private key may be tracked in the repository or printed.

Run:

```bash
bash scripts/build-linux-amd64.sh --version VERSION \
  --license-keyring /secure/license-public-keys.json \
  --release-keyring /secure/release-public-keys.json \
  --release-signing-key /secure/release-v1.seed \
  --release-signing-key-id release-v1 \
  --free-license /secure/free-license.json \
  --asterctl-windows-x64 /secure/build-inputs/asterctl.exe
```

A successful build produces one self-contained Customer archive under `dist/linux/`, its `.sha256`, and an extracted signed bundle. The archive must contain Rust `aster-team-cli`, Rust `aster-control`, Rust `aster-runner`, the downloadable Windows x64 client tool at `client-tools/asterctl/windows-x86_64/asterctl.exe`, static Admin/Member assets, systemd units, the sole public bootstrap `init.sh`, private lifecycle engines under `libexec/`, `VERSION`, the minimal bilingual `README.md`, the SBOM, and signed `RELEASE.json`. The asterctl payload must be listed in the SBOM and signed file tree. The archive must not contain Node.js, CJS, Go customer binaries, License Guard, source maps, environment files, or private keys. Customer instructions must never expose `libexec/install.sh` or `libexec/restore-backup.sh` as public commands.

Treat a nonzero exit, dirty-worktree refusal, keyring mismatch, signature failure, release-tree mismatch, boundary violation, or missing output as a failed build. Do not bypass a check or hand off a partial archive.

Report the source commit and the archive's absolute path, byte size, SHA-256, Release manifest SHA-256, version, architecture, and offline status.
