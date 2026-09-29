# Aster Team product website

The product website is a Vue 3 application with Cloudflare Pages Functions for capacity-assessment trial requests and ordinary product inquiries. Both validate Turnstile, save to D1 before optional email notification, and preserve their separate data models.

The product page, interactive demo and verified download path have local build and test entry points. A production build is an artifact, not evidence of a deployment or formally approved catalog/release. Deployment status and authorization belong to the current task and actual deployment records, not to an earlier redesign task.

## Directory layout

```text
website/
├─ src/                       Vue website source
├─ public/                    Static assets and Pages route policy
├─ functions/api/trial.ts     Pages Function route: POST /api/trial
├─ functions/api/inquiries.ts Pages Function route: POST /api/inquiries
├─ server/                    Validation, Turnstile, D1 rate limiting, SMTP
├─ server/schema/             D1 schema migrations
├─ build/                     Fixed public catalog validation and Vite plugin
├─ shared/generated/          Public schema/types and inquiry validator
├─ tests/                     Isolated runtime and production-browser integration
├─ deploy/README.md           Deployment and operations guide
├─ wrangler.jsonc             Cloudflare Pages and binding configuration
├─ worker-configuration.d.ts  Generated Cloudflare binding types
└─ .dev.vars.example          Local Pages Function variables
```

Only `/api/*` invokes Pages Functions. All other requests are served as static Pages assets.

## Commands

For frontend-only acceptance against an already running local preview, run `npm run test:website:preview -- http://127.0.0.1:14081/` from the integration workspace (replace the URL if needed). This checks bilingual demo interactions, bounded internal scrolling and transient indicators, the public site's six-step typography scale, responsive layouts, client guides and unconfigured download/pricing fallbacks. It writes screenshots under `website/target/website-review/`, does not submit forms or start backend services, and does not replace `npm run verify:changed` before PR readiness.

Run these commands from the repository root:

```powershell
npm run dev:website
npm run dev:website:cf
npm run build --workspace @aster/website
npm run test --workspace @aster/website
npm run test:website:browser
npm run cf:website:types:check
```

`npm run dev:website` starts the UI-only Vite server on port 14080. `npm run dev:website:cf` starts the complete development stack: Vite serves the frontend with HMR on port 14080, Wrangler runs Pages Functions and local D1 on port 8788, and Vite proxies `/api/*` to Wrangler. Frontend edits are therefore visible immediately without rebuilding. The Python local development manager uses this complete stack whenever **包含官网（含表单后端）** is selected.

The runtime tests compile the real Pages routes with Wrangler and apply the actual SQL migrations to fresh local Miniflare D1 databases. The browser suite builds the actual website, serves it on `127.0.0.1:26990`, and checks saved rows directly through the test runtime. It refuses to reuse another process on that port and disposes its runtime after completion. Captcha responses are explicit fixtures; other external requests and SMTP notifications are disabled. It does not read local production credentials or use the real leads database. Screenshots and failure traces are under the ignored `dist/website-validation/` directory. Run the runtime and browser commands sequentially because both compile the same local Functions output.

The home page leads with the product outcome, then provides an interactive Admin/Member demo, text-first capabilities, deployment and client connection sections, dynamic plan cards and trial/inquiry entry points. The demo models representative Aster interactions in the website bundle; it does not connect to a Customer deployment or expose Customer data. The subscription-savings calculator and Capacity navigation have been removed. The team form collects usage context without displaying or sending frontend-generated Pro subscription recommendations; the existing trial API and historical storage model remain unchanged in this frontend-only change.

## Documentation

Public bilingual Markdown lives in `website/docs`. VitePress is installed and the website build includes the static `/docs/` output; the member build uses the same source for offline documentation. The [documentation index](../docs/README.md) lists the public guides.

API capability generation, rule versions and member permission-aware lookup require implementation and verification before being presented as available. Existing pages do not certify completion of every planned gateway capability. The canonical installation/troubleshooting manual remains [docs/user-manual.md](../docs/user-manual.md).

Deployment procedures are in [deploy/README.md](deploy/README.md); a local build does not establish deployment status.

## Approved public catalog input

Operations exports an immutable, approved public catalog. Configure all four inputs before building or starting Vite:

| Environment variable | Value |
| --- | --- |
| `ASTER_WEBSITE_CATALOG_PATH` | Absolute path to the exact exported `plans.json` |
| `ASTER_WEBSITE_CATALOG_SHA256` | Lowercase SHA-256 from the controlled approval/export handoff |
| `ASTER_WEBSITE_CATALOG_REVISION` | Exact `catalog_…` revision from that same handoff |
| `ASTER_WEBSITE_CATALOG_ENVIRONMENT` | `local` or `production`, matching the file |

Do not calculate a new expected hash from an unknown file and treat that as publisher approval. Configured missing, malformed, noncanonical, oversized or mismatched input fails the build; the site never silently substitutes a hand-maintained price list. With no input, or an approved empty array, the local page offers contact only. This unconfigured build is not evidence that formal pricing/download delivery is complete.

The Vite plugin freezes one snapshot for cards, comparison, term totals, selected inquiry references, HTML identity metadata, `catalog-manifest.json` and `/catalog/<revision>/plans.json`. The public file retains the approved bytes. The browser only formats published amounts and does not recalculate contractual discounts. Types and validation come from repository schemas through `npm run generate:contracts`; edit the schemas instead of the generated output. Operations-only approval details and signing material must never be added to the public projection.

`contracts/test-vectors/public-catalog.v1.json` is an actual local Operations test export, not a formal offer. Its names, quotas, prices and support references are fixtures. The browser test explicitly selects this fixture, a test contact address and a local CAPTCHA adapter; it does not use the proposed free/paid rules as approved customer terms.

## Verified product release input

### Latest-download button and Linux one-line install

Set `ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL` in the website build/dev process environment to an HTTPS **direct installer URL** (a CDN or GitHub asset URL, not a release landing page). This setting alone is sufficient for the main download button; no local archive or catalog is required. Optionally set `ASTER_WEBSITE_PRODUCT_RELEASE_VERSION` to the version label shown below it. With no version label, the button shows “Latest release” rather than inventing a version. Changing build configuration requires rebuilding the site (or restarting the dev server).

Priority for build-time overrides: explicit URL → existing verified release download. Otherwise, the public Linux download link and version selector load `/api/releases` at runtime. This Pages Function reads GitHub's latest published release and release list, accepts only complete stable Linux amd64 assets with an exact versioned URL, size and SHA-256 digest, and caches the validated catalog for one day. Publishing a new public Release does not require changing or rebuilding the website, but the selector may take up to one day to refresh. If the release API fails, the page uses the bundled public Linux release in website/src/release-fallback.ts; refresh that fallback version, URL, size, and SHA-256 when updating the website after a release. The latest install command still resolves the current release directly from GitHub. Configure a read-only `GITHUB_RELEASES_TOKEN` Pages Secret for reliable GitHub API access; unauthenticated requests can hit GitHub's shared rate limit. The lookup and validation code is ordinary TypeScript and can be moved behind the same HTTP route if hosting changes. Source archives, prereleases and experimental Windows packages are not selected.

The install surface displays `curl -fsSL <current website origin>/install.sh | bash` with an icon-only Copy action. Version, admin email, HTTP/HTTPS, and domain/IP controls above the command update it using `bash -s --` arguments. The public `install.sh` uses GitHub's `/releases/latest` redirect to resolve `latest`, or uses an explicitly selected tag. It downloads the matching versioned package and `.sha256` sidecar directly from GitHub, checks SHA-256 before extraction, and runs the signed package's initialization and installation commands with input from `/dev/tty`. The script has no dependency on the website's release API or hostname after it starts. With no configuration flags, installation stays interactive. The CLI accepts access and email flags only in unattended mode, so configured commands create a root-owned temporary password file, install a local Runner, and display the generated admin password once. The existing release card provides the package and installation guide. When no verified local release is configured, its pending card uses the runtime catalog and links to the public GitHub guide and releases. It supports fresh Linux x86-64 systemd Control installs; existing installations use the documented upgrade flow.

The Linux status icon is the original [Tux PNG](https://commons.wikimedia.org/wiki/File:Tux.png), by Larry Ewing and The GIMP. It is stored in `public/assets/linux-tux.png` under the author's attribution permission.

Invalid configured URLs fail the build; invalid GitHub release metadata fails closed at runtime. No release is guessed from a tag or asset name when its digest is missing.

These links do **not** certify package hashes, bundled licenses or pricing. The verified release card below remains governed by the existing manifest checks. Supplying any verified-package inputs still requires the complete, valid handoff described below; a direct URL does not bypass those checks. `product-release.json` continues to describe only that verified snapshot, not the live GitHub lookup.

Focused verification: `npm run test:website:downloads`; UI coverage is included in `npm run test:website:preview -- http://127.0.0.1:14081/`.

### Verified package and catalog handoff

The download card is configured independently from the pricing catalog but is accepted only when the package's bundled free License identifies exactly one free plan in that catalog. Configure these inputs together:

| Environment variable | Value |
| --- | --- |
| `ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_PATH` | Absolute path to `releases/<version>/manifest.json` from the reviewed public-support export handoff |
| `ASTER_WEBSITE_PRODUCT_RELEASE_MANIFEST_SHA256` | Lowercase SHA-256 fixed by the release handoff |
| `ASTER_WEBSITE_PRODUCT_RELEASE_ENVIRONMENT` | `local` or `production`, matching the support manifest and catalog |
| `ASTER_WEBSITE_PRODUCT_RELEASE_ARCHIVE_PATH` | Absolute path to the exact Linux amd64 archive named in the manifest |
| `ASTER_WEBSITE_PRODUCT_RELEASE_DOWNLOAD_URL` | With a verified production manifest: HTTPS URL whose final path segment is the exact archive name. May also be used alone for the primary download link above. |

Production download URLs point to immutable assets on `huzz-open/aster-team` Releases. This repository contains the source code and documentation; official packages require the protected release and signing process. Free and paid deployments use the same platform package: a fresh installation activates its bundled free License, and a paid License upgrades the existing installation.

The build recomputes the support manifest, archive and public-document hashes. It compares the package's signed free-License projection with the public free plan by plan ID and version, edition, minimum product version, quota policy, expiry, capabilities and every quota. Missing inputs leave `product-release.json` explicitly unconfigured. Partial, changed or inconsistent inputs fail the build instead of hiding the error or presenting a stale download.

A local release copies the verified archive into `/downloads/` so the built site can exercise an actual browser download. A production release keeps the archive on the separately verified HTTPS release channel and emits its fixed URL, size and SHA-256. The website also emits the exact canonical user manual, Linux guide and `SHA256SUMS` from the support bundle. Download archives remain outside `website-release.json`'s bounded runtime-file list and require their own package verification.

## Capabilities content

Capabilities is a bilingual text-first feature overview: private deployment, authorized customer-owned accounts, independent member keys, model/quota/usage controls, customer-controlled Runners and compatible APIs. Two short role descriptions explain administrator responsibilities and member self-service. This section has no screenshot gallery, video or guided playback. The separate interactive product demo and client connection guides remain available.

`npm run test:website:preview -- http://127.0.0.1:14081/` checks the six features, both roles, bilingual/responsive layout, body text size and absence of screenshot/video loading, alongside the existing website acceptance checks.

## Runtime release manifest

A configured production build also emits `website-release.json`, describing the exact emitted HTML, JavaScript, CSS, catalog manifest and approved catalog bytes. Entries contain public paths, sizes and SHA-256 values; internal module paths are excluded. There is no release manifest for an unconfigured catalog build. The output is limited to 500 runtime files, 10 MiB per file and 32 MiB total.

Operations prepares a candidate from the exported catalog and this manifest's exact digest. Its environment-bound, read-only verifier retrieves the actual home page and all listed runtime files, checks their bytes, and rereads the manifest to detect a change during verification. Local test evidence cannot activate production. Copied `public/` media and downloadable archives are not covered by this emitted-runtime list; `product-release.json` binds the separately verified archive and support documents. This mechanism is not a deployment command or proof that an unverified public site is current.

See [Operations instructions](../docs/operations-guide.md) and [commercial browser validation](../tests/commercial-operations/README.md) for the local reconciliation, response recovery and failure-history workflow. Visitor-selected publication references remain untrusted input: Operations must resolve an approved publication before creating a quote or order. Initial delivery, audited redelivery, machine transfer, renewal, upgrade and trial conversion now share the controlled commercial fulfillment lifecycle.

## Inquiry state and recovery

Ordinary inquiries require contact details and a short description. A selected plan contributes only an **unverified** catalog/plan/version/term reference. Saving this record does not approve a quote, create an order, confirm payment, or grant a License. Later Operations acceptance must resolve trusted publication and commercial records before pricing or fulfillment.

The first attempted submission fixes a UUID and business payload. Until a matching server acknowledgement arrives, this tab retains that payload in session storage, without CAPTCHA tokens or credentials. A response loss, language change or refresh retries the same inquiry with a fresh CAPTCHA. The server returns the original record for the same ID/content and rejects conflicting content. The form explains an unconfirmed result and offers an explicit new-inquiry action; starting a new inquiry may create another record if the first was received. Closing the tab does not promise recovery. Successful confirmation clears the recovery record.

Notification is asynchronous and has its own `pending`, `sent`, `failed` or `not_configured` state. A saved inquiry remains saved if mail fails. Replaying its request does not resend mail. There is no automatic notification retry/reconciliation worker in this increment; Operations integration and handling outstanding notification records remain delivery work. Never present an email attempt as confirmation that an operator has read or accepted an inquiry.

## Actual local package acceptance

```powershell
npm run test:website:release:windows
```

Requires a clean committed worktree, native Windows x64 with Visual Studio C++ Build Tools, Docker Linux containers and the repository's pinned Linux lab images. The command builds the actual Windows `asterctl` from that commit, passes it to the existing Linux lab through `--asterctl-windows-x64`, and runs the quick Customer/Runner installation checks on Ubuntu 20.04. Previously reported archives are copied aside before the lab updates its standard output path. This creates a test-signed package; it does not issue a formal release or export private signing keys.

The command exports the canonical public-support documents from the actual package, builds the production website with a clearly marked local test catalog matching the bundled License, and serves it through the existing isolated local Functions runtime. The catalog is test input, not a new Operations-approved commercial offer. Browser checks cover the desktop download card and mobile free-plan download action, exact archive bytes and SHA-256, four static ELF binaries, the actual embedded Windows tool, and the downloaded package's complete signature/file-tree verification inside an isolated container. User manual and Linux guide responses must match the repository's canonical files. The lab checks installation before downloading; identical downloaded archive hashes and the downloaded CLI's tree verification bind those results to the website payload.

Artifacts, local website, browser screenshots, downloaded files and `result.json` remain under `dist/website-release-validation/<run-id>/`; public-support output remains under `dist/public-support/website-<run-id>/`. An unsuccessful or interrupted run is not a passed acceptance. The source commit and worktree must remain unchanged throughout the run. This command complements the smaller catalog/inquiry browser matrix, whose archive is an explicit byte fixture. It never deploys the site, applies remote migrations, sends notifications or publishes GitHub Releases.

## Review confirmed pricing with an already accepted package

```powershell
npm run test:website:release:windows -- --reuse-acceptance <run-id>
```

This prepares a new complete local website and reruns desktop/mobile pricing and actual-download acceptance without rebuilding an unchanged Customer package. Only a successful test-package acceptance with matching input receipt, version, archive size and SHA-256 is reusable. The new record separately retains the frontend commit, original package commit and source acceptance run; a newer website does not claim that Customer was rebuilt or installed again.

All current actual-package runs include four local review plans: the exact bundled free rights, 20 seats at CNY 5999/year, 50 seats at CNY 9999/year, and contact pricing for larger installations. The two subscriptions include the standard feature set and the confirmed 1–5 year discounts. The local `labwebsitecatalog` tool calls Operations' actual `FreezePlan`, price calculation and `BuildPublicCatalog` functions and saves `review-plan-snapshots.json` beside the emitted catalog. It never connects to a database, approves a catalog, creates an order, signs a License or publishes anything. These marked review templates are not production defaults or a formally approved offer. Tax, transfer and support-channel details remain subject to confirmation; contact-plan quotas display as negotiated rather than advertising the template's baseline as a contracted limit.

Open the newly printed run ID with the preview command below. Pricing, demo and verified Linux download can then be reviewed together. This remains an isolated preview: the contact address ends in `.invalid`, CAPTCHA is disabled, and no inquiry or real notification is sent. Windows installation acceptance remains a separate administrator requirement and is not inferred from this Linux-package run.

## Reopen the preserved production build

After a successful actual-package acceptance, reuse its printed run ID:

```powershell
npm run preview:website:release -- <run-id>
```

This checks the recorded acceptance inputs, emitted runtime-file hashes and downloadable archive before serving the preserved frontend at `http://127.0.0.1:26992`. It reports the artifact source commit explicitly; it does not rebuild the frontend or claim that the saved artifact contains later source edits. The local Functions are compiled from the current checkout and use a fresh disposable database with outbound notifications disabled. Stop with Ctrl+C. No production credentials or CF deployment are required. A release acceptance build has no CAPTCHA key, so its inquiry UI shows the configured fallback instead of submitting; complete inquiry acceptance uses `npm run test:website:browser` with explicit local verification fixtures.

To capture the preserved frontend and exit automatically:

```powershell
npm run preview:website:release -- <run-id> --capture
```

This writes a new `visual-review/<timestamp>/` under that run directory, preserving older evidence. `index.html` links full-page images and overlapping screen-sized captures at 1920×960, 1366×648 and 390×720 content viewports, without browser chrome. `screenshots.json` records source identity, exact viewport dimensions, scroll positions and page errors. Browser requests to external services are blocked and recorded, so the capacity calculator uses its offline exchange-rate fallback. Capture checks page overflow and actual scroll positions; screenshots still require visual review and do not constitute user approval of the design.

## Security model

- Turnstile is validated only by the Pages Function; its secret is never sent to the browser.
- The server checks the Turnstile action and exact frontend hostname.
- Requests are restricted by exact origin, capped at 16 KiB for trial assessment and 32 KiB for product inquiries, validated field by field, protected by a honeypot, and rate-limited in D1.
- Client IP addresses are not stored. A salted SHA-256 fingerprint is used for rate limiting.
- Form data is stored before email delivery so an SMTP failure does not lose the request.
- Secrets are stored with Cloudflare Pages Secrets or in the ignored local `.dev.vars` file.


### Windows experimental download

Linux remains the recommended download. An additional Windows amd64 package can be exposed by setting all four `ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_PATH`, `ASTER_WEBSITE_WINDOWS_RELEASE_MANIFEST_SHA256`, `ASTER_WEBSITE_WINDOWS_RELEASE_ENVIRONMENT` and `ASTER_WEBSITE_WINDOWS_RELEASE_ARCHIVE_PATH` inputs. Production also requires `ASTER_WEBSITE_WINDOWS_RELEASE_DOWNLOAD_URL` with the exact HTTPS archive URL. These inputs follow the same byte verification and catalog matching as Linux. Windows must use the same public free plan ID/version and the manifest must explicitly declare `artifact.channel=experimental`; an incomplete configuration fails the build. The frontend displays the instability and slower-fix notice beside the Windows download.

Generate Windows support documents with `node scripts/build-public-support-bundle.mjs --platform=windows --version=<version> --archive=<absolute-path> --checksum=<absolute-path> --output=<absolute-path-below-dist/public-support>`. Verify with the same `--platform=windows` plus `--verify=<output>`. The bundle contains `README-WINDOWS.md`; the site serves its documents below `/downloads/windows/` so the two platform checksum files do not overwrite each other. This only prepares local artifacts and does not publish a GitHub Release or deploy a website.

### 真实双平台下载验收

在固定 `integration-test` 工作树应用当前 Draft PR 的精确提交并通过源码检查后，执行 `npm run test:windows-pipeline:prepare`。该命令使用隔离测试密钥构建基准和升级候选，输出 `target/wp/<id>/lifecycle.json`。随后执行：

```powershell
npm run test:website:release:windows -- --reuse-acceptance <Linux验收run-id> --windows-fixture <id>
```

不指定 `--reuse-acceptance` 时会重新构建并安装验证 Linux 包。Windows 输入必须来自当前集成提交；Linux 复用输入保留其原始提交与验收来源。验收会核对两平台属于同一免费套餐、真实下载两份包、校验摘要，并执行下载的 Windows CLI 验证 Release 文件树和随包免费证书。Windows 安装说明和校验文件也通过实际链接读取。这不是 Windows 管理员安装、恢复、升级与回滚验收，后者仍使用准备命令输出的 `npm run test:windows-pipeline -- -Configuration "..."`。

两平台测试免费证书的套餐字段都源自同一合同向量；构包版本与签发时间不改变固定套餐的最低软件版本。下载验收不读取测试私钥，不创建正式 Release，也不部署 CF。UI 视觉打磨可以独立延后。

若完整构包与安装已经通过，只有浏览器验收脚本需要修复，可保留原始验收为失败，并单独复查其已生成输入：先将 `ASTER_WEBSITE_RELEASE_TEST_INPUTS` 指向该次 `inputs.json`，在固定集成工作树运行 `npm exec -- playwright test --config website/tests/release.playwright.config.mjs`。保存独立日志及当前测试源码提交，不将旧运行的 `result.json` 改成通过，也不把包原来的源码提交改成新提交；此命令只证明保留产物的浏览器下载/验签链路。需要新的完整验收或可重开官网时，仍使用上面的完整命令。

Windows 下载验收执行包内程序时使用扩展长度路径，避免深层测试输出目录超过普通 Win32 路径上限而出现文件存在却无法启动的 `ENOENT`。此处理限于验收进程的启动路径，不改变包内容或产品授权。
