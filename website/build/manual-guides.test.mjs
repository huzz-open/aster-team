import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'
import test from 'node:test'
import { manualRegion, manualSourceHash, manualTopics, validateManualGuides } from './manual-guides.mjs'

const root = resolve(dirname(fileURLToPath(import.meta.url)), '../..')

function fixture(t) {
  const directory = mkdtempSync(join(tmpdir(), 'aster-manual-guides-'))
  t.after(() => rmSync(directory, { recursive: true, force: true }))
  for (const name of ['docs/user-manual.md', ...manualTopics.flatMap(([slug]) =>
    ['zh-cn', 'en'].map(locale => `website/docs/${locale}/administration/${slug}.md`))]) {
    const target = join(directory, name)
    mkdirSync(dirname(target), { recursive: true })
    writeFileSync(target, readFileSync(join(root, name)))
  }
  return directory
}

test('all public task guides have bounded canonical includes and reviewed English sources', () => {
  validateManualGuides(root)
})

test('missing, reversed, duplicate or empty regions cannot expand the entire private manual', () => {
  for (const source of [
    'private preface\n<!-- #region setup -->\npublic instructions',
    '<!-- #endregion setup -->\nprivate text\n<!-- #region setup -->',
    '<!-- #region setup -->\na\n<!-- #endregion setup -->\n<!-- #region setup -->\nb\n<!-- #endregion setup -->',
    '<!-- #region setup -->\n\n<!-- #endregion setup -->',
  ]) assert.throws(() => manualRegion(source, 'setup'))
  assert.equal(manualRegion('private preface\n<!-- #region setup -->\npublic instructions\n<!-- #endregion setup -->\nprivate appendix', 'setup'), 'public instructions')
})

test('public includes reject nested content and repository-relative links', () => {
  for (const body of [
    '<!--@include: private.md-->',
    '[private](../../docs/internal/design.md)',
    '[architecture](runner-trust-model.md#trust)',
  ]) assert.throws(() => manualRegion(`<!-- #region setup -->\n${body}\n<!-- #endregion setup -->`, 'setup'))
})

test('source hashes are stable across Windows line endings but detect changed instructions', () => {
  assert.equal(manualSourceHash('line one\nline two'), manualSourceHash('\r\nline one\r\nline two\r\n'))
  assert.notEqual(manualSourceHash('Stop before upgrade'), manualSourceHash('Upgrade while running'))
})

test('a canonical instruction change blocks stale English even when all files exist', t => {
  const directory = fixture(t)
  const manual = join(directory, 'docs/user-manual.md')
  writeFileSync(manual, readFileSync(manual, 'utf8').replace('<!-- #region installation -->', '<!-- #region installation -->\nChanged installation prerequisite.'))
  assert.throws(() => validateManualGuides(directory), /review the English guide/)
})

test('an unbounded include or missing translation fails closed', t => {
  const directory = fixture(t)
  const chinese = join(directory, 'website/docs/zh-cn/administration/preparation.md')
  const original = readFileSync(chinese, 'utf8')
  writeFileSync(chinese, original.replace('user-manual.md#preparation', 'user-manual.md'))
  assert.throws(() => validateManualGuides(directory), /canonical manual region/)
  writeFileSync(chinese, original)
  rmSync(join(directory, 'website/docs/en/administration/preparation.md'))
  assert.throws(() => validateManualGuides(directory), /ENOENT/)
})
