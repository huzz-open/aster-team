import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { afterEach, describe, expect, test } from 'vitest'
import { readLocalAdminCredentials } from '../../../scripts/local-admin-credentials-vite'

const directories: string[] = []

afterEach(async () => {
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })))
})

describe('local administrator credential loader', () => {
  test('selects one side and changes its digest when the file changes', async () => {
    const directory = await mkdtemp(join(tmpdir(), 'aster-local-credentials-'))
    directories.push(directory)
    const path = join(directory, 'local-admin-credentials.env')
    await writeFile(path, [
      'ASTER_LOCAL_CUSTOMER_EMAIL=customer@example.com',
      'ASTER_LOCAL_CUSTOMER_PASSWORD=customer-password',
      'ASTER_LOCAL_OPERATIONS_EMAIL=operations@example.com',
      'ASTER_LOCAL_OPERATIONS_PASSWORD=operations-password',
      'ASTER_LOCAL_MEMBER_EMAIL=test@at.com',
      'ASTER_LOCAL_MEMBER_PASSWORD=member-password',
      '',
    ].join('\n'))

    const customer = await readLocalAdminCredentials('customer', path)
    const operations = await readLocalAdminCredentials('operations', path)
    const member = await readLocalAdminCredentials('member', path)
    expect(customer).toMatchObject({ email: 'customer@example.com', password: 'customer-password' })
    expect(operations).toMatchObject({ email: 'operations@example.com', password: 'operations-password' })
    expect(member).toMatchObject({ email: 'test@at.com', password: 'member-password' })
    expect(customer.sha256).toBe(operations.sha256)
    expect(customer.sha256).toBe(member.sha256)

    await writeFile(path, (await readFile(path, 'utf8')).replace('customer-password', 'new-customer-password'))
    expect((await readLocalAdminCredentials('customer', path)).sha256).not.toBe(customer.sha256)
  })
})
