import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('container-generated request fixtures are readable by host Playwright without exposing the owner password', async () => {
  const source = await readFile(new URL('./run.mjs', import.meta.url), 'utf8')
  assert.match(source, /license request --output \/e2e\/request\.json > \/e2e\/terminal\.txt\r?\n[\s\S]*?chmod 0644 \/e2e\/request\.json \/e2e\/request\.qr\.png/)
  assert.match(source, /chmod 0600 \/e2e\/owner\.password/)
  assert.doesNotMatch(source, /chmod 0644 \/e2e\/owner\.password/)
})
