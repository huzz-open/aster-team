import assert from 'node:assert/strict'
import test from 'node:test'
import { addAccount, addMember, balance, createDemoState, createKey, demoDays, grantQuota, memberUsage, revokeKey, usage } from '../src/product-demo'

test('all demo summaries, charts and personal records derive from the same ledger', () => {
  const state = createDemoState()
  const team = usage(state.logs)
  assert.equal(team.billed, 6_420_000)
  assert.equal(team.raw, team.billed)
  assert.equal(team.requests, 21)
  assert.equal(team.daily.reduce((sum,value)=>sum+value,0), team.billed)
  assert.equal(team.models.reduce((sum,value)=>sum+value.requests,0), team.requests)
  assert.equal(Math.round(team.cache/team.raw*100),31)
  assert.equal(state.members.reduce((sum,member)=>sum+memberUsage(state,member.id).billed,0), team.billed)
  for (const member of state.members) {
    assert.equal(balance(state,member.id)+memberUsage(state,member.id).billed,member.granted)
  }
  for (const log of state.logs) {
    assert.ok(demoDays.includes(log.day))
    assert.ok(state.members.some(member=>member.id===log.memberId))
    assert.equal(log.input+log.cache+log.output,log.billed)
  }
})

test('quota, account, member and key actions remain consistent and resettable', () => {
  const state = createDemoState()
  const original = createDemoState()
  for (let index=0;index<9;index++) {addMember(state);addAccount(state);createKey(state)}
  assert.equal(state.members.length,13)
  assert.equal(new Set(state.members.map(member=>member.id)).size,13)
  assert.equal(state.accounts.length,12)
  assert.equal(state.keys.filter(key=>key.active).length,12)
  assert.equal(balance(state,13),0)
  grantQuota(state,1)
  assert.equal(balance(state,1),balance(original,1)+1_000_000)
  grantQuota(state,13)
  assert.equal(balance(state,13),1_000_000)
  const before = memberUsage(state,1)
  revokeKey(state,1)
  revokeKey(state,1)
  assert.equal(state.keys.filter(key=>key.active).length,11)
  assert.deepEqual(memberUsage(state,1),before)
  Object.assign(state,createDemoState())
  assert.deepEqual(state,original)
})
