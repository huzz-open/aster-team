import assert from 'node:assert/strict'
import test from 'node:test'
import { tableCellViolations } from './verify-flat-table-cells.mjs'

const table = cells => `<table><thead><tr><th scope="col">Feature</th><th v-for="plan in plans" scope="col">{{ plan.name }}</th></tr></thead><tbody><tr>${cells}</tr></tbody></table>`

test('semantic row headers and ordinary data rows have the same column count', () => {
  assert.deepEqual(tableCellViolations(table('<th scope="row">Members</th><td v-for="plan in plans">{{ plan.seats }}</td>')), [])
  assert.deepEqual(tableCellViolations(table('<td>Members</td><td v-for="plan in plans">{{ plan.seats }}</td>')), [])
})

test('missing and surplus columns are still rejected with a row header', () => {
  assert.match(tableCellViolations(table('<th scope="row">Members</th>'))[0].reason, /2 headers but 1 cells/)
  assert.match(tableCellViolations(table('<th scope="row">Members</th><td>3</td><td>Extra</td>'))[0].reason, /2 headers but 3 cells/)
})

test('stacked secondary content is rejected in both data and header cells', () => {
  for (const tag of ['td', 'th']) {
    for (const [content, reason] of [['Members<br>Runner', 'line break'], ['Members<small>Secondary</small>', 'secondary small text'],
      ['Members<span class="muted">Secondary</span>', 'secondary muted block']]) {
      assert.ok(tableCellViolations(table(`<${tag}>${content}</${tag}><td>3</td>`)).some(value => value.reason === reason))
    }
  }
})
