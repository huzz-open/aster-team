import assert from 'node:assert/strict'
import { readFile } from 'node:fs/promises'
import test from 'node:test'

test('fake upstream OAuth contract matches the Customer Control request', async () => {
  const control = await readFile(new URL('../../customer/backend/control/src/lib.rs', import.meta.url), 'utf8')
  const fake = await readFile(new URL('./fake-upstream.mjs', import.meta.url), 'utf8')
  const rustConstant = name => {
    const match = control.match(new RegExp(`const ${name}: &str =\\s*"([^"]+)";`))
    assert.ok(match, `${name} must be declared exactly once`)
    return match[1]
  }
  const fakeValue = pattern => {
    const match = fake.match(pattern)
    assert.ok(match, `fake upstream must specify ${pattern}`)
    return match[1]
  }
  assert.equal(fakeValue(/const oauthClientID = '([^']+)'/), rustConstant('OPENAI_OAUTH_CLIENT_ID'))
  assert.equal(fakeValue(/const oauthRedirectURI = '([^']+)'/), rustConstant('OPENAI_OAUTH_REDIRECT_URI'))
  assert.equal(fakeValue(/scope: '([^']+)', code_challenge_method/), rustConstant('OPENAI_OAUTH_SCOPE'))
})
