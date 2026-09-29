from __future__ import annotations

import json
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

from scripts import deploy_website_cloudflare as deploy


class WebsiteCloudflareDeployTest(unittest.TestCase):
    def test_parse_json_output_skips_wrangler_banner(self) -> None:
        payload = deploy.parse_json_output("Wrangler 4.127.1\n[{[0m\"name\":\"site\"}]")
        self.assertEqual(payload, [{"name": "site"}])

    def test_parse_env_file_handles_comments_exports_and_quotes(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / ".prod.vars"
            path.write_text("# private\nexport TURNSTILE_SECRET='secret'\nRATE_LIMIT_SALT=abc\n", encoding="utf-8")
            self.assertEqual(
                deploy.parse_env_file(path),
                {"TURNSTILE_SECRET": "secret", "RATE_LIMIT_SALT": "abc"},
            )

    def test_update_database_id_updates_only_the_expected_binding(self) -> None:
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "wrangler.jsonc"
            path.write_text(json.dumps({
                "d1_databases": [{
                    "binding": deploy.DATABASE_BINDING,
                    "database_name": deploy.DATABASE_NAME,
                    "database_id": "old",
                }],
            }), encoding="utf-8")
            with patch.object(deploy, "WRANGLER_CONFIG", path):
                self.assertTrue(deploy.update_database_id("new"))
                self.assertFalse(deploy.update_database_id("new"))
            self.assertEqual(json.loads(path.read_text(encoding="utf-8"))["d1_databases"][0]["database_id"], "new")

    def test_pages_secret_names_extracts_only_binding_names(self) -> None:
        output = """The production environment has access to:\n  - RATE_LIMIT_SALT: Value Encrypted\n  - TURNSTILE_SECRET: Value Encrypted\n"""
        with patch.object(deploy, "run") as run:
            run.return_value.stdout = output
            self.assertEqual(
                deploy.pages_secret_names(deploy.PROJECT_NAME),
                {"RATE_LIMIT_SALT", "TURNSTILE_SECRET"},
            )


if __name__ == "__main__":
    unittest.main()
