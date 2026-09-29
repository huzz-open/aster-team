import assert from 'node:assert/strict'
import { mkdtempSync, mkdirSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { resolve } from 'node:path'
import test from 'node:test'

import { assetRootsFromArguments, verifyCustomerProductionAssets } from './verify-customer-production-assets.mjs'

function fixture() {
  const directory = mkdtempSync(resolve(tmpdir(), 'aster-customer-assets-'))
  const admin = resolve(directory, 'admin')
  const member = resolve(directory, 'member')
  mkdirSync(admin)
  mkdirSync(member)
  writeFileSync(resolve(admin, 'index.html'), '<main>Aster Admin</main>')
  writeFileSync(resolve(member, 'index.js'), 'document.title="Aster Member"')
  return { directory, roots: [admin, member] }
}

test('customer production asset check accepts clean builds', () => {
  const value = fixture()
  try {
    assert.doesNotThrow(() => verifyCustomerProductionAssets(value.roots))
  } finally {
    rmSync(value.directory, { recursive: true, force: true })
  }
})

test('customer production asset check rejects Demo and MSW markers', () => {
  const value = fixture()
  try {
    writeFileSync(resolve(value.roots[0], 'index.js'), 'setupWorker(); fetch("/mockServiceWorker.js")')
    assert.throws(() => verifyCustomerProductionAssets(value.roots), /contains demo marker/)
  } finally {
    rmSync(value.directory, { recursive: true, force: true })
  }
})

test('customer production asset check accepts scoped command-line roots', () => {
  const [root] = assetRootsFromArguments(['--root=customer/admin/dist'])

  assert.equal(root, resolve('customer/admin/dist'))
  assert.throws(() => assetRootsFromArguments(['customer/admin/dist']), /Usage:/)
})
