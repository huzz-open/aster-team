import assert from 'node:assert/strict'
import test from 'node:test'

import { runCommandLanes } from './verification-runner.mjs'

function nodeCommand(id, source) {
  return { id, executable: process.execPath, arguments: ['-e', source] }
}

test('verification lanes run successful commands in every lane', async () => {
  await runCommandLanes([
    [nodeCommand('first', 'process.exit(0)'), nodeCommand('second', 'process.exit(0)')],
    [nodeCommand('independent', 'process.exit(0)')],
  ])
})

test('verification lanes fail closed when any lane command fails', async () => {
  await assert.rejects(
    runCommandLanes([
      [nodeCommand('failure', 'process.exit(7)')],
      [nodeCommand('success', 'process.exit(0)')],
    ]),
    error => error.exitCode === 7 && /failure failed/.test(error.message),
  )
})

test('verification lanes reject empty or malformed plans', async () => {
  await assert.rejects(runCommandLanes([]), /non-empty/)
  await assert.rejects(runCommandLanes([[]]), /non-empty/)
})
