export type DemoMember = { id: number; zh: string; en: string; granted: number }
export type DemoLog = { id: number; day: string; memberId: number; model: string; input: number; cache: number; output: number; billed: number }
export type DemoState = ReturnType<typeof createDemoState>
export const demoDays = ['09-01', '09-02', '09-03', '09-04', '09-05', '09-06', '09-07']
export const demoModels = ['gpt-5.3-codex', 'gpt-5.2', 'gpt-5.1']
export function createDemoState() {
  const daily = [600_000, 720_000, 840_000, 960_000, 1_080_000, 1_020_000, 1_200_000]
  return {
    accounts: [1, 2, 3].map(id => ({ id, email: `account-${id}@example.invalid`, provider: 'OpenAI', plan: 'Plus', credentials: id === 1 ? 2 : 1 })),
    members: [
      { id: 1, zh: '林默', en: 'Alex Morgan', granted: 10_000_000 },
      { id: 2, zh: '周然', en: 'Jordan Lee', granted: 8_000_000 },
      { id: 3, zh: '陈岚', en: 'Taylor Reed', granted: 6_000_000 },
      { id: 4, zh: '许青', en: 'Casey Chen', granted: 4_000_000 },
    ] as DemoMember[],
    logs: daily.flatMap((total, day) => [1, 2, 3].map(memberId => {
      const billed = total / 3
      return { id: day * 3 + memberId, day: demoDays[day], memberId, model: demoModels[(day + memberId - 1) % 3], input: Math.round(billed * .46), cache: Math.round(billed * .31), output: Math.round(billed * .23), billed }
    })) as DemoLog[],
    keys: [1, 2, 3].map(id => ({ id, memberId: id, active: true })),
    nextKeyId: 4,
  }
}
export function usage(logs: DemoLog[]) {
  const totals = logs.reduce((sum, log) => ({ input: sum.input + log.input, cache: sum.cache + log.cache, output: sum.output + log.output, billed: sum.billed + log.billed }), { input: 0, cache: 0, output: 0, billed: 0 })
  return { ...totals, raw: totals.input + totals.cache + totals.output, requests: logs.length, daily: demoDays.map(day => logs.filter(log => log.day === day).reduce((sum, log) => sum + log.billed, 0)), models: demoModels.map(model => ({ model, requests: logs.filter(log => log.model === model).length })) }
}
export function memberUsage(state: DemoState, memberId: number) { return usage(state.logs.filter(log => log.memberId === memberId)) }
export function balance(state: DemoState, memberId: number) { return (state.members.find(member => member.id === memberId)?.granted ?? 0) - memberUsage(state, memberId).billed }
export function addAccount(state: DemoState) {
  const id = state.accounts.length + 1
  state.accounts.push({ id, email: `account-${id}@example.invalid`, provider: 'OpenAI', plan: 'Plus', credentials: 1 })
}
export function addMember(state: DemoState) {
  const id = state.members.length + 1
  state.members.push({ id, zh: `示例成员 ${id}`, en: `Demo member ${id}`, granted: 0 })
}
export function grantQuota(state: DemoState, id: number) {
  const member = state.members.find(member => member.id === id)
  if (member) member.granted += 1_000_000
}
export function createKey(state: DemoState, memberId = 1) { state.keys.push({ id: state.nextKeyId++, memberId, active: true }) }
export function revokeKey(state: DemoState, id: number) { const key = state.keys.find(key => key.id === id); if (key) key.active = false }
export function tokens(value: number) { return new Intl.NumberFormat('en-US', { maximumFractionDigits: 2 }).format(value / 1_000_000) + 'M' }
