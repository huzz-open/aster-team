<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref } from 'vue'
import SiteIcon from './SiteIcon.vue'

const props = defineProps<{ locale: 'zh' | 'en' }>()
const copy = computed(() => props.locale === 'zh' ? {
  label: 'Aster 将订阅/账号统一接入，为团队与应用提供独立 Key、成员额度和用量记录。',
  sources: '订阅/账号', consumers: '团队与应用', deployment: '部署在你的服务器',
  inputs: [['账号接入', '集中管理已授权账号'], ['凭据管理', '凭据集中加密保存'], ['模型同步', '按配置同步可用模型']],
  outputs: [['研发成员', '独立 Key · 成员额度'], ['产品成员', '独立 Key · 成员额度'], ['团队应用', '通过成员 Key 接入']],
  values: [['统一接入', '集中管理账号'], ['额度控制', '按成员分配'], ['用量审计', '每次调用可追溯']],
} : {
  label: 'Aster connects subscription accounts to team members and applications with independent keys, member quotas and usage records.',
  sources: 'Subscriptions & accounts', consumers: 'Teams & applications', deployment: 'On your own server',
  inputs: [['Account access', 'Manage authorized accounts'], ['Credentials', 'Stored centrally, encrypted'], ['Model sync', 'Sync available models']],
  outputs: [['Engineering', 'Independent keys · Quotas'], ['Product', 'Independent keys · Quotas'], ['Team applications', 'Connect with member keys']],
  values: [['Unified access', 'Central account management'], ['Quota control', 'Allocate by member'], ['Usage audit', 'Trace each request']],
})
const inputIcons = ['user', 'shield', 'model'] as const
const outputIcons = ['user-code', 'user-content', 'gateway'] as const
const valueIcons = ['model', 'users', 'file'] as const
const inputPaths = ['M4 36C92 36 78 104 180 104', 'M4 112H180', 'M4 188C92 188 78 120 180 120']
const outputPaths = ['M0 104C102 104 88 36 176 36', 'M0 112H176', 'M0 120C102 120 88 188 176 188']
const figure = ref<HTMLElement | null>(null)
const inView = ref(false)
const paused = ref(false)
const pageVisible = ref(true)
let observer: IntersectionObserver | undefined
function updateVisibility() { pageVisible.value = document.visibilityState === 'visible' }
onMounted(() => {
  updateVisibility()
  document.addEventListener('visibilitychange', updateVisibility)
  observer = new IntersectionObserver(([entry]) => { inView.value = entry.isIntersecting })
  if (figure.value) observer.observe(figure.value)
})
onUnmounted(() => {
  observer?.disconnect()
  document.removeEventListener('visibilitychange', updateVisibility)
})
</script>

<template>
  <figure ref="figure" class="hero-gateway" :class="{ 'is-flowing': inView && pageVisible && !paused }" :aria-label="copy.label">
    <div class="gateway-scene">
      <section class="gateway-endpoints gateway-inputs" :aria-label="copy.sources">
        <ul><li v-for="(item, index) in copy.inputs" :key="index"><i><SiteIcon :name="inputIcons[index]" :size="23" /></i><div><strong>{{ item[0] }}</strong><span>{{ item[1] }}</span></div></li></ul>
      </section>
      <svg class="gateway-links gateway-links-in" viewBox="0 0 180 220" preserveAspectRatio="none" aria-hidden="true">
        <g v-for="(path, index) in inputPaths" :key="path" :style="{ '--flow-delay': `${index * -1.3}s` }"><path :d="path" /><path class="gateway-current" :d="path" pathLength="100" /><path class="gateway-spark" :d="path" pathLength="100" /></g><circle cx="4" cy="36" r="3" /><circle cx="4" cy="112" r="3" /><circle cx="4" cy="188" r="3" />
      </svg>
      <div class="gateway-core">
        <div class="gateway-plinth" aria-hidden="true"></div>
        <div class="gateway-glass"><img src="/logo-mark.svg" width="72" height="72" alt=""><strong>Aster <span>Team</span></strong><p><SiteIcon name="server" :size="14" />{{ copy.deployment }}</p></div>
      </div>
      <svg class="gateway-links gateway-links-out" viewBox="0 0 180 220" preserveAspectRatio="none" aria-hidden="true">
        <g v-for="(path, index) in outputPaths" :key="path" :style="{ '--flow-delay': `${index * -1.3 - 1.8}s` }"><path :d="path" /><path class="gateway-current" :d="path" pathLength="100" /><path class="gateway-spark" :d="path" pathLength="100" /></g><circle cx="176" cy="36" r="3" /><circle cx="176" cy="112" r="3" /><circle cx="176" cy="188" r="3" />
      </svg>
      <section class="gateway-endpoints gateway-outputs" :aria-label="copy.consumers">
        <ul><li v-for="(item, index) in copy.outputs" :key="index"><i><SiteIcon :name="outputIcons[index]" :size="23" /></i><div><strong>{{ item[0] }}</strong><span>{{ item[1] }}</span></div></li></ul>
      </section>
    </div>
    <figcaption class="gateway-values"><div v-for="(item, index) in copy.values" :key="index"><SiteIcon :name="valueIcons[index]" :size="25" /><strong>{{ item[0] }}</strong><span>{{ item[1] }}</span></div></figcaption>
    <button class="gateway-motion-toggle" type="button" :aria-pressed="!paused" @click="paused = !paused">{{ locale === 'zh' ? (paused ? '播放光流' : '暂停光流') : (paused ? 'Play light flow' : 'Pause light flow') }}</button>
  </figure>
</template>

<style scoped>
.hero-gateway{width:min(100%,1320px);margin:0 auto;position:relative}
.gateway-scene{display:grid;grid-template-columns:minmax(210px,280px) minmax(56px,1fr) 240px minmax(56px,1fr) minmax(210px,280px);align-items:center;position:relative;isolation:isolate}
.gateway-scene::before{content:"";position:absolute;inset:0 16% -15%;z-index:-1;background:radial-gradient(ellipse at 50% 64%,#dcd5f76b,transparent 64%)}
.gateway-endpoints ul{display:grid;gap:clamp(14px,2svh,22px);padding:0;margin:0;list-style:none}
.gateway-endpoints li{display:flex;align-items:center;gap:13px;min-height:66px;padding:12px 15px;border:1px solid #fff;border-radius:12px;background:linear-gradient(120deg,#ffffffed,#ffffff80);box-shadow:0 8px 22px #38304c06}
.gateway-endpoints li>i{width:38px;height:38px;display:grid;place-items:center;flex-shrink:0;border-radius:10px;background:#ece9ff;color:#7561e8;font-style:normal}
.gateway-endpoints li:nth-child(2)>i{background:#e2f4ed;color:#37a58d}
.gateway-endpoints li>div{min-width:0;display:grid;gap:5px}
.gateway-endpoints strong{font-size:var(--font-size-body);line-height:1.3;font-weight:650;color:var(--ink)}
.gateway-endpoints li span{font-size:var(--font-size-body);line-height:1.5;color:var(--muted)}
.gateway-links{width:100%;height:238px;overflow:visible;color:#a398eb}
.gateway-links path{fill:none;stroke:currentColor;stroke-width:1.2;vector-effect:non-scaling-stroke}
.gateway-links circle{fill:currentColor}
.gateway-links-out{color:#8dc9bf}
.gateway-links .gateway-current,.gateway-links .gateway-spark{stroke:#8065ed;stroke-linecap:round;animation:gateway-current 4.8s linear infinite;animation-delay:var(--flow-delay);animation-play-state:paused;vector-effect:none}
.gateway-links .gateway-current{stroke-width:2;stroke-dasharray:9 111;filter:drop-shadow(0 0 3px #8d71ff90);opacity:.7}
.gateway-links .gateway-spark{stroke-width:2.8;stroke-dasharray:1 119;animation-name:gateway-spark;filter:drop-shadow(0 0 3px #9b83ff);stroke:#b3a0ff}
.gateway-links-out .gateway-current{stroke:#46bba5;filter:drop-shadow(0 0 3px #4ac6b290)}
.gateway-links-out .gateway-spark{stroke:#91e6d0;filter:drop-shadow(0 0 3px #58d9bd)}
.is-flowing .gateway-current,.is-flowing .gateway-spark{animation-play-state:running}
.gateway-motion-toggle{position:absolute;right:0;top:-49px;display:block;margin:0;padding:4px 10px;border:0;background:transparent;color:var(--muted);font-size:var(--font-size-body);text-decoration:underline;text-decoration-color:#b7b2c3;text-underline-offset:4px;cursor:pointer}
@keyframes gateway-current{from{stroke-dashoffset:12}to{stroke-dashoffset:-108}}
@keyframes gateway-spark{from{stroke-dashoffset:4}to{stroke-dashoffset:-116}}
.gateway-core{position:relative;display:grid;place-items:center;min-height:224px;isolation:isolate}
.gateway-glass{width:214px;min-height:206px;display:flex;align-items:center;justify-content:center;flex-direction:column;padding:22px 12px;border:1px solid #ffffffbd;border-radius:23px;background:linear-gradient(130deg,#e8fff680,#fffffff0 43%,#e7e3ffb3);box-shadow:inset 3px 3px 3px #ffffffcf,inset -3px -3px 3px #b3a9e53b,0 0 0 5px #ffffff4d,0 16px 32px #7464b319;position:relative}
.gateway-glass::before{content:"";position:absolute;inset:-2px;border-radius:24px;border-top:2px solid #b6e5d98a;border-left:2px solid #b6e5d94d;pointer-events:none}
.gateway-glass img{margin-bottom:13px;filter:drop-shadow(0 9px 12px #605ee329)}
.gateway-glass>strong{font-size:var(--font-size-title);letter-spacing:-.045em;line-height:1.2;color:var(--ink)}
.gateway-glass>strong span{font-weight:450;color:#74717c}
.gateway-glass p{display:flex;align-items:center;justify-content:center;gap:6px;margin:12px 0 0;color:#716c7d;font-size:var(--font-size-body)}
.gateway-plinth{position:absolute;z-index:-1;bottom:-3px;width:270px;height:58px;transform:perspective(280px) rotateX(48deg);border:1px solid #fff;border-radius:12px;background:linear-gradient(115deg,#e3ddf9,#fdfcff 48%,#cfece3);box-shadow:0 10px 0 -1px #e6e2ef,0 11px 0 #fff,0 22px 28px #44396d12}
.gateway-values{display:grid;grid-template-columns:repeat(3,1fr);margin-top:24px;padding:16px 0 0;border-top:1px solid #dedbe480}
.gateway-values>div{display:flex;align-items:center;justify-content:center;flex-wrap:wrap;gap:8px 11px;min-width:0;padding:0 12px}
.gateway-values>div+div{border-left:1px solid var(--line)}
.gateway-values .site-icon{color:#7963e9;flex-shrink:0}.gateway-values>div:nth-child(2) .site-icon{color:#39a38b}
.gateway-values strong{font-size:var(--font-size-body);font-weight:650}.gateway-values span{font-size:var(--font-size-body);color:var(--muted)}
@media(min-width:1101px) and (max-height:1000px){.gateway-endpoints li{padding-block:8px;min-height:0}.gateway-endpoints ul{gap:calc(12px + var(--hero-room,0px) * .12)}.gateway-links{height:calc(210px + var(--hero-room,0px) * .24)}.gateway-core{min-height:210px}.gateway-values{margin-top:calc(16px + var(--hero-room,0px) * .12);padding-top:12px}.gateway-motion-toggle{top:calc(-36px - var(--hero-room,0px) * .12)}}
@media(min-width:981px) and (max-height:800px){.gateway-endpoints li{min-height:55px;padding-block:8px}.gateway-endpoints ul{gap:14px}.gateway-links{height:192px}.gateway-core{min-height:208px}.gateway-glass{min-height:177px;padding:15px 10px}.gateway-glass img{width:54px;height:54px}.gateway-values{margin-top:24px;padding-top:16px}}
@media(max-width:980px){.gateway-scene{grid-template-columns:minmax(175px,1fr) 30px 190px 30px minmax(175px,1fr)}.gateway-glass{width:176px}.gateway-plinth{width:216px}.gateway-endpoints li{padding:10px;gap:9px}.gateway-values span{flex-basis:100%;text-align:center}}
@media(max-width:700px){
  .gateway-scene{grid-template-columns:minmax(0,1fr) minmax(0,1fr);gap:28px 12px}.gateway-links,.gateway-motion-toggle{display:none}.gateway-current,.gateway-spark{animation:none!important}
  .gateway-core{grid-column:1/-1;grid-row:2;min-height:210px;padding-top:0}.gateway-inputs{grid-column:1;grid-row:1}.gateway-outputs{grid-column:2;grid-row:1}
  .gateway-endpoints li{min-height:75px;gap:8px;padding:10px 8px}.gateway-endpoints li>i{width:26px;height:30px;background:transparent!important}.gateway-endpoints li>i .site-icon{width:20px;height:20px}.gateway-endpoints strong{font-size:var(--font-size-body)}.gateway-endpoints li span{font-size:var(--font-size-body)}.gateway-endpoints ul{gap:8px}
  .gateway-core::before{content:"";position:absolute;top:-29px;width:50%;height:30px;border:1px solid #b2aad5;border-top:0;border-radius:0 0 18px 18px}.gateway-core::after{content:"";position:absolute;top:0;height:14px;border-left:1px solid #b2aad5}.gateway-glass{min-height:186px;width:210px}.gateway-plinth{width:252px}
  .gateway-values{margin-top:26px;padding-top:18px;gap:14px}.gateway-values>div{flex-direction:column;justify-content:flex-start;gap:8px;padding:0 3px}.gateway-values>div+div{border-left:0}.gateway-values strong{font-size:var(--font-size-body)}.gateway-values span{font-size:var(--font-size-body);line-height:1.5;flex-basis:auto}.gateway-values .site-icon{width:22px;height:22px}
}
@media(prefers-reduced-motion:reduce){.gateway-links .gateway-current,.gateway-links .gateway-spark{display:none;animation:none}.gateway-motion-toggle{display:none}}
</style>
