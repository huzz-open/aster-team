import assert from 'node:assert/strict'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'
import { parseEnv } from 'node:util'
import { updateEnvFile } from './local-env-file.mjs'

test('updateEnvFile preserves unrelated settings and safely persists special characters', async () => {
  const directory = await mkdtemp(resolve(tmpdir(), 'aster-local-env-'))
  const path = resolve(directory, 'credentials.env')
  try {
    await writeFile(path, '# local only\nKEEP=value\nPASSWORD=old\n', 'utf8')
    updateEnvFile(path, { PASSWORD: 'new value!"quoted"', ADDED: 'a=b' })

    const source = await readFile(path, 'utf8')
    const parsed = parseEnv(source)
    assert.match(source, /^# local only$/m)
    assert.equal(parsed.KEEP, 'value')
    assert.equal(parsed.PASSWORD, 'new value!"quoted"')
    assert.equal(parsed.ADDED, 'a=b')
  } finally {
    await rm(directory, { recursive: true, force: true })
  }
})
