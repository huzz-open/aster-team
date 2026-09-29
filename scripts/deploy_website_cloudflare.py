#!/usr/bin/env python3
"""Build and deploy the Aster Team website to Cloudflare Pages."""

from __future__ import annotations

import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.parse
import urllib.request
from pathlib import Path
from typing import Any


ROOT = Path(__file__).resolve().parents[1]
WEBSITE = ROOT / "website"
WRANGLER_CONFIG = WEBSITE / "wrangler.jsonc"
DEFAULT_SECRETS_FILE = WEBSITE / ".prod.vars"
PROJECT_NAME = "aster-team-website"
DATABASE_NAME = "aster-team-website-leads"
DATABASE_BINDING = "LEADS_DB"
PRODUCTION_BRANCH = "main"
REQUIRED_SECRETS = ("TURNSTILE_SECRET", "RATE_LIMIT_SALT", "GITHUB_RELEASES_TOKEN")
OPTIONAL_SECRETS = ("SMTP_PASSWORD",)
PUBLIC_TURNSTILE_TEST_SECRET = "1x0000000000000000000000000000000AA"
ANSI_ESCAPE = re.compile(r"\x1b(?:\[[0-?]*[ -/]*[@-~]|\][^\x07]*(?:\x07|\x1b\\))")


class DeploymentError(RuntimeError):
    pass


def command_text(arguments: list[str]) -> str:
    return subprocess.list2cmdline(arguments) if os.name == "nt" else " ".join(arguments)


def run(
    arguments: list[str],
    *,
    cwd: Path = ROOT,
    capture: bool = False,
    input_text: str | None = None,
) -> subprocess.CompletedProcess[str]:
    print(f"\n> {command_text(arguments)}", flush=True)
    environment = os.environ.copy()
    environment.update({
        "WRANGLER_WRITE_LOGS": "false",
        "WRANGLER_LOG": "log",
        "WRANGLER_LOG_SANITIZE": "true",
    })
    result = subprocess.run(
        arguments,
        cwd=cwd,
        env=environment,
        input=input_text,
        text=True,
        encoding="utf-8",
        errors="replace",
        stdout=subprocess.PIPE if capture else None,
        stderr=subprocess.STDOUT if capture else None,
        check=False,
    )
    if result.returncode != 0:
        if capture and result.stdout:
            print(result.stdout, file=sys.stderr)
        raise DeploymentError(f"command failed with exit code {result.returncode}: {command_text(arguments)}")
    return result


def tool(name: str) -> str:
    candidates = (f"{name}.cmd", name) if os.name == "nt" else (name,)
    executable = next((shutil.which(candidate) for candidate in candidates if shutil.which(candidate)), None)
    if not executable:
        raise DeploymentError(f"required executable is missing: {name}")
    return executable


def wrangler_command() -> list[str]:
    entrypoint = ROOT / "node_modules" / "wrangler" / "bin" / "wrangler.js"
    if not entrypoint.is_file():
        raise DeploymentError("Wrangler is not installed. Run `npm ci` from the repository root first.")
    return [tool("node"), str(entrypoint)]


def parse_json_output(output: str) -> Any:
    clean = ANSI_ESCAPE.sub("", output)
    decoder = json.JSONDecoder()
    for index, character in enumerate(clean):
        if character not in "[{":
            continue
        try:
            value, _end = decoder.raw_decode(clean[index:])
        except json.JSONDecodeError:
            continue
        if isinstance(value, (list, dict)):
            return value
    raise DeploymentError("Wrangler did not return a JSON object or array")


def parse_env_file(path: Path) -> dict[str, str]:
    values: dict[str, str] = {}
    for number, source in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
        line = source.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("export "):
            line = line[7:].lstrip()
        if "=" not in line:
            raise DeploymentError(f"invalid secret entry at {path}:{number}")
        name, value = line.split("=", 1)
        name = name.strip()
        if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
            raise DeploymentError(f"invalid secret name at {path}:{number}")
        value = value.strip()
        if len(value) >= 2 and value[0] == value[-1] and value[0] in {'"', "'"}:
            value = value[1:-1]
        values[name] = value
    return values


def require_ignored_secret_file(path: Path) -> None:
    if not path.is_file():
        raise DeploymentError(
            f"production secret file is missing: {path}\n"
            "Copy website/.prod.vars.example to website/.prod.vars and fill it locally."
        )
    resolved = path.resolve()
    try:
        relative = resolved.relative_to(ROOT.resolve())
    except ValueError:
        return
    result = subprocess.run(
        [tool("git"), "check-ignore", "--quiet", "--", str(relative)],
        cwd=ROOT,
        check=False,
    )
    if result.returncode != 0:
        raise DeploymentError(f"refusing to read a secret file that is not gitignored: {path}")


def load_production_secrets(path: Path) -> dict[str, str]:
    require_ignored_secret_file(path)
    file_values = parse_env_file(path)
    values = {name: os.environ.get(name, file_values.get(name, "")) for name in (*REQUIRED_SECRETS, *OPTIONAL_SECRETS)}
    missing = [name for name in REQUIRED_SECRETS if not values[name]]
    if missing:
        raise DeploymentError(f"missing required production secrets: {', '.join(missing)}")
    if values["TURNSTILE_SECRET"] == PUBLIC_TURNSTILE_TEST_SECRET:
        raise DeploymentError("the public Turnstile test secret cannot be used in production")
    if len(values["RATE_LIMIT_SALT"]) < 32 or values["RATE_LIMIT_SALT"] == "local-development-only":
        raise DeploymentError("RATE_LIMIT_SALT must be a unique production value of at least 32 characters")
    return values


def validate_turnstile_secret(secret: str) -> None:
    body = urllib.parse.urlencode({"secret": secret, "response": "XXXX.DUMMY.TOKEN.XXXX"}).encode("ascii")
    request = urllib.request.Request(
        "https://challenges.cloudflare.com/turnstile/v0/siteverify",
        data=body,
        headers={"Content-Type": "application/x-www-form-urlencoded"},
        method="POST",
    )
    try:
        with urllib.request.urlopen(request, timeout=15) as response:
            payload = json.load(response)
    except (OSError, ValueError) as exc:
        raise DeploymentError(f"Turnstile secret validation failed: {exc}") from exc
    errors = payload.get("error-codes") if isinstance(payload, dict) else None
    if not isinstance(errors, list) or "invalid-input-response" not in errors or "invalid-input-secret" in errors:
        raise DeploymentError("Turnstile rejected the configured production secret")


def wrangler_json(arguments: list[str]) -> Any:
    output = run([*wrangler_command(), *arguments], capture=True).stdout or ""
    return parse_json_output(output)


def ensure_pages_project(project_name: str) -> set[str]:
    projects = wrangler_json(["pages", "project", "list", "--json"])
    project = next((item for item in projects if (item.get("name") or item.get("Project Name")) == project_name), None)
    if project:
        print(f"Cloudflare Pages project exists: {project_name}")
        domains = project.get("domains") or project.get("Project Domains") or []
        if isinstance(domains, str):
            return {domain.strip() for domain in domains.split(",") if domain.strip()}
        return {str(domain).strip() for domain in domains if str(domain).strip()}
    run([
        *wrangler_command(), "pages", "project", "create", project_name,
        "--production-branch", PRODUCTION_BRANCH,
    ])
    return {f"{project_name}.pages.dev"}


def update_database_id(database_id: str) -> bool:
    configuration = json.loads(WRANGLER_CONFIG.read_text(encoding="utf-8"))
    databases = configuration.get("d1_databases")
    if not isinstance(databases, list):
        raise DeploymentError("website/wrangler.jsonc has no d1_databases array")
    binding = next(
        (item for item in databases if item.get("binding") == DATABASE_BINDING and item.get("database_name") == DATABASE_NAME),
        None,
    )
    if not binding:
        raise DeploymentError(f"website/wrangler.jsonc has no {DATABASE_BINDING} binding for {DATABASE_NAME}")
    if binding.get("database_id") == database_id:
        return False
    binding["database_id"] = database_id
    WRANGLER_CONFIG.write_text(json.dumps(configuration, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    return True


def ensure_database(location: str) -> str:
    databases = wrangler_json(["d1", "list", "--json"])
    database = next((item for item in databases if item.get("name") == DATABASE_NAME), None)
    if database is None:
        run([*wrangler_command(), "d1", "create", DATABASE_NAME, "--location", location])
        databases = wrangler_json(["d1", "list", "--json"])
        database = next((item for item in databases if item.get("name") == DATABASE_NAME), None)
    database_id = str((database or {}).get("uuid") or "")
    if not database_id:
        raise DeploymentError(f"Cloudflare did not return an ID for D1 database {DATABASE_NAME}")
    if update_database_id(database_id):
        print(f"Updated website/wrangler.jsonc with D1 database ID {database_id}")
    else:
        print(f"Cloudflare D1 database exists: {DATABASE_NAME}")
    return database_id


def pages_secret_names(project_name: str) -> set[str]:
    output = run(
        [*wrangler_command(), "pages", "secret", "list", "--project-name", project_name],
        capture=True,
    ).stdout or ""
    return set(re.findall(r"^\s*-\s+([A-Za-z_][A-Za-z0-9_]*):", ANSI_ESCAPE.sub("", output), re.MULTILINE))


def upload_pages_secrets(project_name: str, values: dict[str, str]) -> None:
    names = [*REQUIRED_SECRETS, *(name for name in OPTIONAL_SECRETS if values.get(name))]
    if not values.get("SMTP_PASSWORD"):
        print("Warning: SMTP_PASSWORD is empty; trial requests will be stored but email notification is disabled.")
    for name in names:
        run([*wrangler_command(), "pages", "secret", "list", "--project-name", project_name], capture=True)
        run(
            [*wrangler_command(), "pages", "secret", "put", name, "--project-name", project_name],
            input_text=values[name] + "\n",
        )
    listed = pages_secret_names(project_name)
    missing = [name for name in names if name not in listed]
    if missing:
        raise DeploymentError(f"Pages did not confirm secret bindings: {', '.join(missing)}")


def attach_custom_domain(project_name: str, domain: str, account_id: str, known_domains: set[str]) -> None:
    if domain in known_domains:
        print(f"Cloudflare Pages custom domain exists: {domain}")
        return
    token = os.environ.get("CLOUDFLARE_API_TOKEN", "")
    if not token:
        raise DeploymentError("--domain requires CLOUDFLARE_API_TOKEN in the environment")
    base = f"https://api.cloudflare.com/client/v4/accounts/{account_id}/pages/projects/{project_name}/domains"
    headers = {"Authorization": f"Bearer {token}", "Content-Type": "application/json"}
    request = urllib.request.Request(base, headers=headers)
    try:
        with urllib.request.urlopen(request, timeout=20) as response:
            payload = json.load(response)
        current = payload.get("result", []) if isinstance(payload, dict) else []
        if any(item.get("name") == domain for item in current):
            print(f"Cloudflare Pages custom domain exists: {domain}")
            return
        create = urllib.request.Request(
            base,
            data=json.dumps({"name": domain}).encode("utf-8"),
            headers=headers,
            method="POST",
        )
        with urllib.request.urlopen(create, timeout=20) as response:
            payload = json.load(response)
    except (urllib.error.HTTPError, urllib.error.URLError, ValueError) as exc:
        raise DeploymentError(f"failed to attach Pages custom domain {domain}: {exc}") from exc
    if not isinstance(payload, dict) or not payload.get("success"):
        raise DeploymentError(f"Cloudflare rejected custom domain {domain}")
    print(f"Attached Cloudflare Pages custom domain: {domain}")


def verify_site(url: str) -> None:
    last_error: Exception | None = None
    for _attempt in range(8):
        try:
            request = urllib.request.Request(url, headers={"User-Agent": "aster-team-deploy/1"})
            with urllib.request.urlopen(request, timeout=20) as response:
                if response.status == 200:
                    print(f"Website health check passed: {url}")
                    return
        except (urllib.error.HTTPError, urllib.error.URLError, TimeoutError) as exc:
            last_error = exc
        time.sleep(2)
    raise DeploymentError(f"website health check failed for {url}: {last_error or 'unexpected HTTP status'}")


def confirm_secret_write(assume_yes: bool, project_name: str, names: list[str]) -> None:
    if assume_yes:
        return
    print(f"The script is ready to write Pages Secrets to {project_name}: {', '.join(names)}")
    answer = input("Continue with production secret writes and deployment? [y/N] ").strip().lower()
    if answer not in {"y", "yes"}:
        raise DeploymentError("deployment cancelled before secret writes")


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--location", default="apac", choices=("weur", "eeur", "apac", "oc", "wnam", "enam"))
    parser.add_argument("--secrets-file", type=Path, default=DEFAULT_SECRETS_FILE)
    parser.add_argument("--domain", default="", help="optional Pages custom domain to attach")
    parser.add_argument("--account-id", default=os.environ.get("CLOUDFLARE_ACCOUNT_ID", ""))
    parser.add_argument("--sync-secrets", action="store_true", help="replace existing Pages Secrets from --secrets-file")
    parser.add_argument("--yes", action="store_true", help="confirm production secret writes non-interactively")
    arguments = parser.parse_args()

    try:
        npm = tool("npm")
        wrangler = wrangler_command()
        run([*wrangler, "--version"])
        run([*wrangler, "whoami"])
        project_domains = ensure_pages_project(PROJECT_NAME)
        ensure_database(arguments.location)

        existing_secrets = pages_secret_names(PROJECT_NAME)
        missing_required_secrets = [name for name in REQUIRED_SECRETS if name not in existing_secrets]
        production_secrets: dict[str, str] | None = None
        if arguments.sync_secrets or missing_required_secrets:
            production_secrets = load_production_secrets(arguments.secrets_file.resolve())
            validate_turnstile_secret(production_secrets["TURNSTILE_SECRET"])
        else:
            print(f"Reusing existing Cloudflare Pages Secrets: {', '.join(sorted(existing_secrets))}")
            if "SMTP_PASSWORD" not in existing_secrets:
                print("Warning: SMTP_PASSWORD is not configured; trial requests will be stored without email notification.")

        run([npm, "run", "verify"])
        run([
            *wrangler, "d1", "migrations", "apply", DATABASE_NAME, "--remote",
            "--config", str(WRANGLER_CONFIG),
        ], cwd=WEBSITE)

        if production_secrets is not None:
            secret_names = [*REQUIRED_SECRETS, *(name for name in OPTIONAL_SECRETS if production_secrets.get(name))]
            confirm_secret_write(arguments.yes, PROJECT_NAME, secret_names)
            upload_pages_secrets(PROJECT_NAME, production_secrets)

        commit_hash = run([tool("git"), "rev-parse", "HEAD"], capture=True).stdout.strip()
        dirty = bool(run([tool("git"), "status", "--porcelain"], capture=True).stdout.strip())
        run([
            *wrangler, "pages", "deploy", "dist", "--project-name", PROJECT_NAME,
            "--branch", PRODUCTION_BRANCH, "--commit-hash", commit_hash,
            f"--commit-dirty={'true' if dirty else 'false'}",
        ], cwd=WEBSITE)

        if arguments.domain:
            if arguments.domain not in project_domains and not arguments.account_id:
                raise DeploymentError("--domain requires --account-id or CLOUDFLARE_ACCOUNT_ID")
            attach_custom_domain(PROJECT_NAME, arguments.domain, arguments.account_id, project_domains)

        pages_url = f"https://{PROJECT_NAME}.pages.dev"
        verify_site(pages_url)
        print("\nAster Team website deployment completed.")
        print(f"Pages URL: {pages_url}")
        if arguments.domain:
            print(f"Custom domain: https://{arguments.domain}")
        return 0
    except (DeploymentError, OSError, json.JSONDecodeError) as exc:
        print(f"Deployment failed: {exc}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
