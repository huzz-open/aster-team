# Cloudflare Pages deployment

These steps deploy a copy of the optional website in your own Cloudflare account. Local tests use isolated data and do not send real inquiry notifications.

## Example resources

| Resource | Name |
| --- | --- |
| Pages project | `aster-team-website` |
| Pages hostname | `aster-team-website.pages.dev` |
| Production hostname | Your own domain |
| D1 database | `aster-team-website-leads` |
| D1 binding | `LEADS_DB` |
| Turnstile widget | `aster-team-website-trial` |
| API route | `POST /api/trial` |
| Notification mailbox | Your own contact address |

`website/wrangler.jsonc` contains placeholder origins, SMTP settings, and a zero D1 ID. Replace them for your Cloudflare account before deployment. The deployment script updates the D1 ID after creating or finding the named database. Set your own contact address and Turnstile site key in `website/.env.production`; configure matching server-side origins and hostnames in `website/wrangler.jsonc`.
The website's canonical URL, robots file, sitemap, and documentation links refer to the maintained public Aster site. If you serve a separate site under your domain, update those public URLs for that domain before publishing it.

## Prerequisites

- Node.js 22
- npm dependencies installed from the repository root
- Wrangler 4.127.1 or a compatible newer 4.x release
- A Cloudflare login with Pages, D1, Workers, and Turnstile write permissions
- SMTP credentials when email notifications are enabled

Never put the Turnstile secret, rate-limit salt, mailbox password, or authorization code in source control or chat.

## Local full-stack development

Copy the public test-variable template to the ignored local secret file:

```powershell
Copy-Item website/.dev.vars.example website/.dev.vars
```

The template and `.env.development` use Cloudflare's public always-pass Turnstile test key pair. Local development permits up to 100 accepted submissions per ten-minute window so repeated form testing does not immediately trigger production throttling. Add the mailbox client authorization code to `SMTP_PASSWORD` only when local email delivery is required.

Apply the local D1 migration and start the Vite + Pages development stack with one command:

```powershell
npm run dev:website:cf
```

Open `http://127.0.0.1:14080`. The command is idempotent: it checks and applies pending local migrations before every start, serves the frontend through Vite with HMR on port 14080, and runs Pages Functions plus local D1 through Wrangler on port 8788. Vite proxies `/api/*` to Wrangler, so frontend edits appear immediately while the complete form backend remains available. The Python local development manager starts this same complete stack when **包含官网（含表单后端）** is selected.

The ordinary `npm run dev:website` command remains available for UI-only work on port 14080; it does not start Wrangler, so `/api/*` is available only when another local Pages backend is already running on port 8788.

## Production secrets

These Pages Secrets must exist on `aster-team-website`:

- `TURNSTILE_SECRET`
- `RATE_LIMIT_SALT`
- `GITHUB_RELEASES_TOKEN` (read-only access to public `huzz-open/aster-team` Releases)
- `SMTP_PASSWORD`

Set the mailbox authorization code interactively so it does not enter shell history:

```powershell
wrangler pages secret put SMTP_PASSWORD --project-name aster-team-website
```

Configure `SMTP_HOST`, `SMTP_PORT`, `SMTP_USERNAME`, `LEAD_NOTIFICATION_FROM`, and `LEAD_NOTIFICATION_TO` for your own mail provider. The message sets `Reply-To` only when the applicant entered a valid email address. If email notification is disabled, accepted requests remain in D1.

## Validate and deploy

The repeatable production workflow is implemented in `scripts/deploy_website_cloudflare.py`. It creates or reuses the Pages project and D1 database, updates the tracked D1 ID when the account resource changes, runs the full repository verification, applies remote migrations, writes production secrets through standard input, deploys the Pages bundle, and checks the production Pages URL.

Existing Pages Secrets are reused by default, so routine releases do not require a local copy. For the first deployment or an intentional secret rotation, create the ignored production secret file:

```powershell
Copy-Item website/.prod.vars.example website/.prod.vars
```

Fill `TURNSTILE_SECRET`, `RATE_LIMIT_SALT`, and `GITHUB_RELEASES_TOKEN` locally. `SMTP_PASSWORD` is optional; when it is empty, accepted requests remain in D1 with email notification disabled. Use `--sync-secrets` only when bootstrapping missing secrets or intentionally replacing them.

Publish from the repository root:

```powershell
npm run cf:website:publish
```

For non-interactive CI after its secret environment is configured, pass `--yes`. To associate a custom hostname in the same run, provide `CLOUDFLARE_API_TOKEN` and `CLOUDFLARE_ACCOUNT_ID`, then append `-- --domain www.example.com --yes` with your actual hostname. DNS remains managed separately.

### Manual component commands

From the repository root:

```powershell
npm run cf:website:types:check
npm run test --workspace @aster/website
npm run build --workspace @aster/website
npm run cf:website:migrate:remote
npm run cf:website:deploy
```

Run the repository-wide verification before publishing or committing:

```powershell
npm run verify
```

## Custom domain with Aliyun DNS

First associate your hostname with the `aster-team-website` project in Cloudflare Pages. Then add the following DNS record with your DNS provider:

| Type | Host record | Target |
| --- | --- | --- |
| CNAME | Your chosen subdomain | `aster-team-website.pages.dev` |

Do not create only the CNAME without first associating the custom hostname in Pages; Cloudflare will not serve an unassociated hostname.

## Inspect submitted leads

List recent requests without exposing the rate-limit fingerprint:

```powershell
wrangler d1 execute aster-team-website-leads --remote --command "SELECT id, created_at, contact, company, team_size, active_users, evidence, weekly_tokens_100m, daily_time, recommended_pro_accounts, notification_status FROM trial_leads ORDER BY created_at DESC LIMIT 50"
```

Retrying a failed notification is intentionally not automated. The lead remains in D1 with `notification_status = 'failed'` and a short diagnostic in `notification_error`.

## Form processing contract

1. The browser obtains a single-use Turnstile token for action `trial_request`.
2. The Pages Function checks the exact Origin, request size, field schema, Turnstile action, and production hostname.
3. A salted client fingerprint is limited to three accepted attempts per ten-minute window.
4. The server recomputes usage level and recommended Pro 20x capacity instead of trusting browser calculations.
5. D1 stores the lead first and returns HTTP 202.
6. Email delivery runs through `waitUntil`; success or failure is written back to the same D1 row.
