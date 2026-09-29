<script setup lang="ts">
import { computed, onUnmounted, reactive, ref, watch } from 'vue'
import SiteIcon from './SiteIcon.vue'
import DemoUsage from './DemoUsage.vue'
import { addAccount, addMember, balance, createDemoState, createKey, demoDays, grantQuota, memberUsage, revokeKey, tokens, usage, type DemoMember } from './product-demo'
const props = defineProps<{ locale: 'zh' | 'en' }>()
const state = reactive(createDemoState())
const mode = ref<'admin' | 'member'>('admin')
const adminPanel = ref<'overview' | 'accounts' | 'users' | 'logs'>('overview')
const memberPanel = ref<'dashboard' | 'keys' | 'usage' | 'logs'>('dashboard')
const page = ref(0)
const notice = ref('')
const content = ref<HTMLElement | null>(null)
let timer: ReturnType<typeof setTimeout> | undefined
const tx = (zh: string,en: string) => props.locale === 'zh' ? zh : en
const name = (member: DemoMember) => props.locale === 'zh' ? member.zh : member.en
const panel = computed(() => mode.value === 'admin' ? adminPanel.value : memberPanel.value)
const adminNav = computed(() => [
  { id:'overview' as const,label:tx('运行概览','Operations overview'),icon:'chart' as const },
  { id:'accounts' as const,label:tx('订阅/账号','Subscriptions & accounts'),icon:'key' as const },
  { id:'users' as const,label:tx('成员与额度','Members and quota'),icon:'users' as const },
  { id:'logs' as const,label:tx('全团队消费日志','Team consumption logs'),icon:'file' as const },
])
const memberNav = computed(() => [
  { id:'dashboard' as const,label:tx('工作台','Dashboard'),icon:'chart' as const },
  { id:'keys' as const,label:tx('API Key 管理','API Key Management'),icon:'key' as const },
  { id:'usage' as const,label:tx('用量分析','Usage Analytics'),icon:'chart' as const },
  { id:'logs' as const,label:tx('消费日志','Consumption Logs'),icon:'file' as const },
])
const title = computed(() => (mode.value==='admin'?adminNav.value:memberNav.value).find(item=>item.id===panel.value)?.label)
const team = computed(() => usage(state.logs))
const personalLogs = computed(() => state.logs.filter(log=>log.memberId===1))
const personal = computed(() => memberUsage(state,1))
const visibleLogs = computed(() => mode.value==='admin'?state.logs:personalLogs.value)
const granted = computed(() => state.members.reduce((sum,member)=>sum+member.granted,0))
const activeMembers = computed(() => new Set(state.logs.map(log=>log.memberId)).size)
const activeKeys = computed(() => state.keys.filter(key=>key.active))
const memberKeys = computed(() => state.keys.filter(key=>key.memberId===1))
const metrics = computed(() => [
  [tx('本期 Token','Period tokens'),tokens(team.value.billed)], [tx('API 请求','API requests'),String(team.value.requests)],
  [tx('额度利用率','Quota utilization'),`${(team.value.billed/granted.value*100).toFixed(1)}%`],
  [tx('活跃成员率','Active member rate'),`${(activeMembers.value/state.members.length*100).toFixed(0)}%`],
  [tx('缓存 Token 占比','Cached token share'),`${(team.value.cache/team.value.raw*100).toFixed(0)}%`],
  [tx('账号共享效率','Account sharing'),(activeMembers.value/state.accounts.length).toFixed(1)],
])
const total = computed(() => panel.value==='accounts'?state.accounts.length:panel.value==='users'?state.members.length:panel.value==='keys'?memberKeys.value.length:visibleLogs.value.length)
const pages = computed(() => Math.max(1,Math.ceil(total.value/5)))
function slice<T>(rows:T[]):T[] {return rows.slice(page.value*5,(page.value+1)*5)}
function notify(zh:string,en:string) {clearTimeout(timer);notice.value=tx(zh,en);timer=setTimeout(()=>{notice.value=''},3500)}
function newAccount() {addAccount(state);page.value=Math.floor((state.accounts.length-1)/5);notify('已添加示例订阅/账号','A sample subscription account was added')}
function newMember() {addMember(state);page.value=Math.floor((state.members.length-1)/5);notify('已添加示例成员，可为其分配额度','Sample member added. You can now allocate quota.')}
function grant(id:number) {grantQuota(state,id);notify('已发放 1M Token 示例额度','Granted 1M tokens of sample quota')}
function newKey() {createKey(state);page.value=Math.floor((memberKeys.value.length-1)/5);notify('示例 Key 已创建，不可用于真实请求','Sample key created. It cannot authorize real requests.')}
function removeKey(id:number) {revokeKey(state,id);notify('示例 Key 已撤销','Sample key revoked')}
function reset() {Object.assign(state,createDemoState());mode.value='admin';adminPanel.value='overview';memberPanel.value='dashboard';page.value=0;clearTimeout(timer);notice.value=''}
watch([mode,adminPanel,memberPanel],()=>{page.value=0;notice.value='';content.value?.scrollTo({top:0,behavior:'instant'})})
watch(()=>props.locale,()=>{notice.value=''})
onUnmounted(()=>clearTimeout(timer))
</script>

<template>
  <div class="demo-shell" data-product-demo>
    <header class="demo-toolbar"><div class="demo-brand"><img src="/logo-mark.svg" alt=""><strong>Aster <span>Team</span></strong></div><div class="demo-toolbar-actions"><span class="demo-badge">{{ tx('示例数据','Sample data') }}</span><button class="demo-reset" type="button" @click="reset"><SiteIcon name="clock" :size="16" />{{ tx('重置体验','Reset demo') }}</button></div></header>
    <div class="demo-app">
      <aside class="demo-sidebar">
        <div class="demo-mode-switch" role="tablist" :aria-label="tx('切换体验角色','Demo role')"><button type="button" role="tab" :aria-selected="mode==='admin'" @click="mode='admin'">{{ tx('管理端','Admin') }}</button><button type="button" role="tab" :aria-selected="mode==='member'" @click="mode='member'">{{ tx('成员端','Member') }}</button></div>
        <nav v-if="mode==='admin'" aria-label="Aster admin demo"><button v-for="item in adminNav" :key="item.id" type="button" :class="{active:adminPanel===item.id}" :aria-current="adminPanel===item.id?'page':undefined" @click="adminPanel=item.id"><SiteIcon :name="item.icon" :size="17" /><span>{{ item.label }}</span></button></nav>
        <nav v-else aria-label="Aster member demo"><button v-for="item in memberNav" :key="item.id" type="button" :class="{active:memberPanel===item.id}" :aria-current="memberPanel===item.id?'page':undefined" @click="memberPanel=item.id"><SiteIcon :name="item.icon" :size="17" /><span>{{ item.label }}</span></button></nav>
        <div class="demo-user"><SiteIcon name="user" :size="20" /><span><strong>{{ mode==='admin'?tx('示例管理员','Demo admin'):name(state.members[0]) }}</strong><small>{{ mode==='admin'?'admin@example.invalid':'member-1@example.invalid' }}</small></span></div>
      </aside>
      <main ref="content" class="demo-content" tabindex="0" :aria-label="title" data-lenis-prevent>
        <header class="demo-page-heading"><div><h3>{{ title }}</h3><p>{{ tx('固定示例周期：2026-09-01 至 2026-09-07','Sample period: Sep 1–7, 2026') }}</p></div><div><button v-if="mode==='admin' && panel==='accounts'" type="button" class="demo-primary" @click="newAccount"><SiteIcon name="plus" :size="16" />{{ tx('新增示例账号','Add sample account') }}</button><button v-if="mode==='admin' && panel==='users'" type="button" class="demo-primary" @click="newMember"><SiteIcon name="plus" :size="16" />{{ tx('新增成员','Add member') }}</button><button v-if="mode==='member' && panel==='keys'" type="button" class="demo-primary" @click="newKey"><SiteIcon name="plus" :size="16" />{{ tx('创建 Key','Create key') }}</button></div></header>
        <template v-if="mode==='admin' && panel==='overview'">
          <section class="demo-metrics"><article v-for="(metric,index) in metrics" :key="index"><span>{{ metric[0] }}</span><strong>{{ metric[1] }}</strong><small v-if="index===2">{{ tokens(team.billed) }} / {{ tokens(granted) }}</small><small v-else-if="index===3">{{ activeMembers }} / {{ state.members.length }} {{ tx('位成员','members') }}</small><small v-else-if="index===5">{{ tx('活跃成员 / 账号','Active members / accounts') }}</small></article></section>
          <DemoUsage :logs="state.logs" :locale="locale" />
          <section class="demo-runtime"><span>{{ tx('订阅/账号','Subscriptions & accounts') }} <b data-demo-account-count>{{ state.accounts.length }}</b></span><span>{{ tx('团队成员','Team members') }} <b data-demo-member-count>{{ state.members.length }}</b></span><span>{{ tx('有效 Key','Active keys') }} <b data-demo-key-count>{{ activeKeys.length }}</b></span></section>
        </template>
        <template v-else-if="mode==='member' && (panel==='dashboard' || panel==='usage')">
          <section v-if="panel==='dashboard'" class="demo-member-overview"><article class="demo-balance"><span>{{ tx('当前余额','Current balance') }}</span><strong data-demo-balance>{{ tokens(balance(state,1)) }} <small>Token</small></strong><div class="demo-balance-track"><i :style="{width:`${balance(state,1)/state.members[0].granted*100}%`}"></i></div><footer><span>{{ tx('累计发放','Granted') }} {{ tokens(state.members[0].granted) }}</span><span>{{ tx('累计使用','Used') }} {{ tokens(personal.billed) }}</span></footer></article><div class="demo-member-metrics"><article><span>{{ tx('请求数','Requests') }}</span><strong>{{ personal.requests }}</strong></article><article><span>{{ tx('结算 Token','Billed tokens') }}</span><strong>{{ tokens(personal.billed) }}</strong></article><article><span>{{ tx('有效 Key','Active keys') }}</span><strong>{{ activeKeys.filter(key=>key.memberId===1).length }}</strong></article></div></section>
          <section v-else class="demo-metrics"><article><span>{{ tx('原始 Token','Raw tokens') }}</span><strong>{{ tokens(personal.raw) }}</strong></article><article><span>{{ tx('结算 Token','Billed tokens') }}</span><strong>{{ tokens(personal.billed) }}</strong></article><article><span>{{ tx('平均每次结算','Average per request') }}</span><strong>{{ tokens(personal.billed/personal.requests) }}</strong></article></section>
          <DemoUsage :logs="personalLogs" :locale="locale" />
          <section v-if="panel==='usage'" class="demo-table-card demo-daily-usage"><div class="demo-table-scroll" tabindex="0" :aria-label="tx('每日用量','Daily usage')"><table><thead><tr><th>{{ tx('日期','Date') }}</th><th>{{ tx('请求数','Requests') }}</th><th>{{ tx('结算 Token','Billed tokens') }}</th></tr></thead><tbody><tr v-for="(day,index) in demoDays" :key="day"><td>{{ day }}</td><td>{{ personalLogs.filter(log=>log.day===day).length }}</td><td>{{ personal.daily[index].toLocaleString('en-US') }}</td></tr></tbody></table></div></section>
          <button v-if="panel==='dashboard'" type="button" class="demo-secondary" @click="memberPanel='logs'">{{ tx('查看我的消费日志','View my consumption logs') }}<SiteIcon name="chevron-right" :size="16" /></button>
        </template>
        <section v-else class="demo-table-card" :data-demo-page="`${mode}-${panel}`">
          <div class="demo-table-scroll" tabindex="0" :aria-label="title">
            <table v-if="mode==='admin' && panel==='accounts'"><thead><tr><th>{{ tx('账号','Account') }}</th><th>{{ tx('提供方','Provider') }}</th><th>{{ tx('套餐','Plan') }}</th><th>{{ tx('凭据池','Credentials') }}</th><th>{{ tx('状态','Status') }}</th></tr></thead><tbody><tr v-for="account in slice(state.accounts)" :key="account.id" class="demo-table-row"><td>{{ account.email }}</td><td>{{ account.provider }}</td><td>{{ account.plan }}</td><td>{{ account.credentials }}</td><td><span class="demo-status">{{ tx('启用','Active') }}</span></td></tr></tbody></table>
            <table v-else-if="mode==='admin' && panel==='users'"><thead><tr><th>{{ tx('成员','Member') }}</th><th>{{ tx('邮箱','Email') }}</th><th>{{ tx('剩余 Token','Token balance') }}</th><th>{{ tx('请求数','Requests') }}</th><th>{{ tx('额度操作','Quota action') }}</th></tr></thead><tbody><tr v-for="member in slice(state.members)" :key="member.id" class="demo-table-row"><td>{{ name(member) }}</td><td>member-{{ member.id }}@example.invalid</td><td>{{ tokens(balance(state,member.id)) }}</td><td>{{ memberUsage(state,member.id).requests }}</td><td><button type="button" class="demo-inline-action" :aria-label="`${tx('为','Grant to')} ${name(member)} +1M Token`" @click="grant(member.id)">+1M Token</button></td></tr></tbody></table>
            <table v-else-if="mode==='member' && panel==='keys'"><thead><tr><th>{{ tx('名称','Name') }}</th><th>Key</th><th>{{ tx('状态','Status') }}</th><th>{{ tx('操作','Action') }}</th></tr></thead><tbody><tr v-for="key in slice(memberKeys)" :key="key.id" class="demo-table-row"><td>{{ tx('示例 Key','Sample key') }} {{ key.id }}</td><td class="demo-key-value"><code>ask_demo_{{ key.id }}_••••</code></td><td>{{ key.active?tx('有效','Active'):tx('已撤销','Revoked') }}</td><td><button v-if="key.active" type="button" class="demo-inline-action" :aria-label="`${tx('撤销示例 Key','Revoke sample key')} ${key.id}`" @click="removeKey(key.id)">{{ tx('撤销','Revoke') }}</button></td></tr></tbody></table>
            <table v-else-if="panel==='logs'"><thead><tr><th>{{ tx('时间','Time') }}</th><th>{{ tx('成员','Member') }}</th><th>{{ tx('模型','Model') }}</th><th>{{ tx('结算 Token','Billed tokens') }}</th><th>{{ tx('状态','Status') }}</th></tr></thead><tbody><tr v-for="log in slice([...visibleLogs].reverse())" :key="log.id" class="demo-table-row"><td>{{ log.day }} 10:0{{ log.memberId }}</td><td>{{ name(state.members.find(member=>member.id===log.memberId)!) }}</td><td>{{ log.model }}</td><td>{{ log.billed.toLocaleString('en-US') }}</td><td><span class="demo-status">{{ tx('成功','Success') }}</span></td></tr></tbody></table>
          </div>
          <footer class="demo-pagination"><span>{{ tx('共','Total') }} {{ total }} {{ tx('条','records') }}</span><div><button type="button" :disabled="page===0" @click="page--">{{ tx('上一页','Previous') }}</button><span>{{ page+1 }} / {{ pages }}</span><button type="button" :disabled="page+1>=pages" @click="page++">{{ tx('下一页','Next') }}</button></div></footer>
        </section>
        <div class="demo-notice" role="status" aria-live="polite">{{ notice }}</div>
      </main>
    </div>
  </div>
</template>

<style scoped>
.demo-daily-usage{margin-top:20px}
.demo-shell .demo-app{height:clamp(480px,72svh,760px);min-height:0;overflow:hidden}
.demo-shell .demo-sidebar{min-height:0;overflow:auto}
.demo-shell .demo-content{min-height:0;overflow:auto;overscroll-behavior:contain}
.demo-notice:empty{display:none}
@media(max-width:620px){.demo-shell .demo-app{display:grid;grid-template-columns:minmax(0,1fr);grid-template-rows:auto minmax(0,1fr)}.demo-shell .demo-sidebar{overflow:visible}}
.demo-shell{width:100%;overflow:hidden;border:1px solid #dfdde7;border-radius:18px;background:#fff;box-shadow:0 24px 70px #25213c12;color:#272735}.demo-toolbar{min-height:62px;padding:12px 20px;display:flex;justify-content:space-between;align-items:center;border-bottom:1px solid #e6e5ec;background:#fbfbfc;gap:12px}.demo-brand{display:flex;align-items:center;gap:9px;font-size:var(--font-size-body)}.demo-brand img{width:26px;height:26px}.demo-brand span{font-weight:400;color:#827d8c}.demo-toolbar-actions{display:flex;gap:12px;align-items:center}.demo-badge{padding:6px 9px;border-radius:7px;background:#f0edf9;color:#736497;font-size:var(--font-size-body)}.demo-reset{display:flex;gap:6px;align-items:center;border:0;background:transparent;color:#6d657c;cursor:pointer;font-size:var(--font-size-body);min-height:36px}
.demo-app{min-height:690px;display:grid;grid-template-columns:225px minmax(0,1fr)}.demo-sidebar{padding:18px 12px;display:flex;flex-direction:column;border-right:1px solid #e7e5ed;background:#f8f7fa}.demo-mode-switch{padding:3px;display:grid;grid-template-columns:1fr 1fr;gap:3px;background:#eceaf1;border-radius:9px}.demo-mode-switch button{min-height:36px;border:0;border-radius:7px;background:transparent;color:#77707f;font-size:var(--font-size-body);cursor:pointer}.demo-mode-switch button[aria-selected=true]{background:#fff;color:#2e283b;font-weight:700;box-shadow:0 2px 6px #30254514}.demo-sidebar nav{display:grid;gap:7px;margin-top:22px}.demo-sidebar nav button{min-height:44px;padding:9px 10px;display:flex;align-items:center;gap:10px;color:#696272;background:transparent;border:0;border-radius:8px;font-size:var(--font-size-body);line-height:1.4;text-align:left;cursor:pointer}.demo-sidebar nav .site-icon{flex-shrink:0}.demo-sidebar nav button.active{color:#6050c6;background:#ede9f8;font-weight:650}.demo-user{display:flex;gap:10px;align-items:center;margin-top:auto;padding:16px 7px 0;border-top:1px solid #dedbe5;color:#777080}.demo-user span{min-width:0;display:grid;gap:4px}.demo-user strong{font-size:var(--font-size-body)}.demo-user small{font-size:var(--font-size-caption);overflow-wrap:anywhere}
.demo-content{min-width:0;padding:26px;background:#fdfdfd}.demo-page-heading{display:flex;justify-content:space-between;align-items:flex-start;gap:16px;margin-bottom:22px}.demo-page-heading h3{margin:0;font-size:var(--font-size-title);letter-spacing:-.025em}.demo-page-heading p{margin:8px 0 0;font-size:var(--font-size-body);color:#837b8d;line-height:1.6}.demo-primary,.demo-secondary,.demo-inline-action{display:inline-flex;align-items:center;justify-content:center;gap:7px;min-height:38px;padding:8px 12px;border:1px solid #dcd7e9;border-radius:8px;background:#fff;color:#62529d;font-size:var(--font-size-body);cursor:pointer}.demo-primary{background:#6956d7;border-color:#6956d7;color:#fff}.demo-inline-action{min-height:32px;padding:4px 8px;font-size:var(--font-size-body);white-space:nowrap}.demo-metrics{display:grid;grid-template-columns:repeat(3,minmax(0,1fr));gap:12px}.demo-metrics article{min-width:0;min-height:98px;padding:15px;border:1px solid #e6e3ec;border-radius:10px;background:#fff}.demo-metrics span{display:block;font-size:var(--font-size-body);color:#736a81}.demo-metrics strong{display:block;margin-top:8px;font-size:var(--font-size-title)}.demo-metrics small{display:block;margin-top:6px;font-size:var(--font-size-caption);color:#877e91}.demo-runtime{display:flex;flex-wrap:wrap;gap:18px 26px;margin-top:20px;padding:16px;border:1px solid #e6e3ec;border-radius:10px;background:#fff;font-size:var(--font-size-body);color:#736a81}.demo-runtime b{margin-left:8px;color:#2b9879}
.demo-member-overview{display:grid;grid-template-columns:1fr 1fr;gap:16px}.demo-balance{padding:18px;border:1px solid #e6e3ec;border-radius:12px;background:#fff}.demo-balance>span{font-size:var(--font-size-body);color:#736a81}.demo-balance>strong{display:block;margin:10px 0 16px;font-size:var(--font-size-display)}.demo-balance small{font-size:var(--font-size-body);color:#82788e}.demo-balance-track{height:6px;background:#ebe8f4;border-radius:9px;overflow:hidden}.demo-balance-track i{display:block;height:100%;background:linear-gradient(90deg,#7760dd,#46bca9)}.demo-balance footer{display:flex;flex-wrap:wrap;justify-content:space-between;gap:8px;margin-top:12px;font-size:var(--font-size-caption);color:#82788e}.demo-member-metrics{display:grid;gap:8px}.demo-member-metrics article{padding:12px 15px;border:1px solid #e6e3ec;border-radius:9px;display:flex;justify-content:space-between;align-items:center;font-size:var(--font-size-body);background:#fff}.demo-member-metrics span{color:#736a81}.demo-member-metrics strong{font-size:var(--font-size-title)}
.demo-table-card{border:1px solid #e6e3ec;border-radius:12px;background:#fff;overflow:hidden}.demo-table-scroll{overflow:auto;scrollbar-width:none}.demo-table-scroll::-webkit-scrollbar{display:none}.demo-table-scroll table{width:100%;border-collapse:collapse;font-size:var(--font-size-body);text-align:left}.demo-table-scroll th{padding:15px 16px;background:#f8f7fa;color:#80758e;font-size:var(--font-size-body);font-weight:550;white-space:nowrap}.demo-table-scroll td{padding:18px 16px;border-top:1px solid #eeebf2;white-space:nowrap}.demo-status{color:#269572}.demo-key-value code{color:#6956b2;font-size:var(--font-size-body)}.demo-pagination{display:flex;justify-content:space-between;align-items:center;gap:12px;flex-wrap:wrap;padding:14px 16px;border-top:1px solid #eeebf2;font-size:var(--font-size-body);color:#80758e}.demo-pagination>div{display:flex;gap:12px;align-items:center}.demo-pagination button{min-height:32px;padding:6px 9px;border:1px solid #ded8e8;border-radius:6px;background:#fff;font-size:var(--font-size-body);color:#605479;cursor:pointer}.demo-pagination button:disabled{opacity:.4;cursor:default}.demo-notice{min-height:20px;margin-top:15px;color:#4f826f;font-size:var(--font-size-body);line-height:1.5}button:hover:not(:disabled){filter:brightness(.98)}
@media(max-width:1000px){.demo-app{grid-template-columns:190px minmax(0,1fr)}.demo-content{padding:20px}.demo-sidebar nav button{font-size:var(--font-size-body)}.demo-page-heading{flex-wrap:wrap}.demo-member-overview{grid-template-columns:1fr}}
@media(max-width:620px){.demo-toolbar{padding:10px 12px}.demo-toolbar-actions{gap:5px}.demo-reset{font-size:var(--font-size-caption)}.demo-badge{font-size:var(--font-size-caption)}.demo-brand{font-size:var(--font-size-body)}.demo-app{display:block;min-height:0}.demo-sidebar{border-right:0;border-bottom:1px solid #e7e5ed;padding:12px}.demo-mode-switch{max-width:240px;margin:auto}.demo-sidebar nav{display:flex;overflow:auto;scrollbar-width:none;margin-top:14px;gap:5px}.demo-sidebar nav::-webkit-scrollbar{display:none}.demo-sidebar nav button{flex:0 0 auto;font-size:var(--font-size-body);padding:9px;min-height:38px}.demo-sidebar nav .site-icon{display:none}.demo-user{display:none}.demo-content{padding:20px 14px}.demo-metrics{grid-template-columns:repeat(2,minmax(0,1fr));gap:9px}.demo-metrics article{padding:12px}.demo-page-heading h3{font-size:var(--font-size-title)}.demo-runtime{gap:12px;font-size:var(--font-size-body)}.demo-pagination{padding:12px}.demo-pagination>div{gap:8px}}
</style>
