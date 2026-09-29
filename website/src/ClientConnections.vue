<script setup lang="ts">
import { computed, ref } from 'vue'
import SiteIcon from './SiteIcon.vue'
const props = defineProps<{ locale: 'zh' | 'en' }>()
const selected = ref(0)
// Aster-specific steps follow docs/user-manual.md. Provider docs describe client limits,
// not an assertion that every client feature or model has been verified with Aster.
const clients = [
  { name: 'Codex', guide: 'codex.md', protocol: 'OpenAI Responses', endpoint: 'https://aster.example.com/v1', docs: 'https://learn.chatgpt.com/docs/auth',
    zh: ['安装 Codex，在 Aster 成员端「接入文档」下载并安装适合当前设备的 asterctl。', '完全退出 Codex，运行页面生成的 Codex 初始化命令，按隐藏提示输入自己的成员 API Key。', '初始化完成后重新进入 Codex，使用管理员开放的模型。额度和请求记录统一回到 Aster。'],
    en: ['Install Codex. In Aster Member, open API Documentation and install asterctl for your device.', 'Quit Codex, then run the generated Codex setup command. Enter your member API key at the hidden prompt.', 'Open Codex after setup and use a model enabled by your administrator. Aster tracks the resulting usage.'],
    noteZh: '此处指支持的 Codex 本地工作流，不代表相同的 ChatGPT 云端功能或订阅权益。具体模型与功能以客户端版本、管理员配置和上游支持为准。',
    noteEn: 'This describes supported local Codex workflows, not equivalent ChatGPT cloud features or subscription benefits. Available models and features depend on your client version, administrator settings and upstream support.' },
  { name: 'Claude Code', guide: 'claude-code.md', protocol: 'Anthropic Messages', endpoint: 'https://aster.example.com', docs: 'https://code.claude.com/docs/en/llm-gateway-connect',
    zh: ['安装 Claude Code CLI，在 Aster 成员端「接入文档」确认客户端版本并安装 asterctl。', '在项目目录运行页面生成的 Claude 初始化命令，按隐藏提示输入成员 API Key。', 'Aster 根据开放模型生成本地配置和模型映射，再启动 Claude Code。'],
    en: ['Install Claude Code CLI. Check your client version and install asterctl from API Documentation in Aster Member.', 'In your project folder, run the generated Claude setup command and enter your member API key at the hidden prompt.', 'Aster generates local configuration and model mappings from the enabled models, then starts Claude Code.'],
    noteZh: 'Anthropic Base URL 使用服务根地址，不带 /v1。模型映射以成员端生成结果为准，配置文件不要提交到代码仓库。',
    noteEn: 'Use the service root without /v1 for the Anthropic base URL. Keep generated model mappings and never commit credentials.' },
  { name: 'Cursor', guide: 'member-guide.md', protocol: 'OpenAI-compatible · BYOK', endpoint: 'https://aster.example.com/v1', docs: 'https://cursor.com/help/models-and-usage/api-keys',
    zh: ['在 Cursor Settings 中打开 Models，查看当前版本是否提供 OpenAI Base URL 覆盖设置。', '如支持自定义地址，填写 Aster 的 /v1 地址和自己的成员 API Key，再选择两端都支持的模型。', '先发送一条测试消息，并在 Aster 消费日志确认请求。不能覆盖地址或模型不兼容时，不应视为已接入。'],
    en: ['Open Models in Cursor Settings and check whether your version provides an OpenAI base URL override.', 'If available, enter the Aster /v1 URL and your member API key, then select a model supported by both sides.', 'Send a test message and confirm it in Aster consumption logs. Without a compatible model and URL override, the connection is not established.'],
    noteZh: '按客户端版本与模型验证兼容性；自带 Key 不覆盖 Tab 补全。请求仍经过 Cursor 服务，纯内网地址无法直接用于其云端请求，请先确认网络和数据策略。',
    noteEn: 'Compatibility depends on the client and model; BYOK does not cover Tab completion. Requests still pass through Cursor services, so private-only URLs cannot serve those cloud requests. Check network and data policies first.' },
] as const
const client = computed(() => clients[selected.value])
const guideUrl = computed(() => `https://github.com/huzz-open/aster-team/blob/main/docs/${props.locale === 'zh' ? 'zh-CN/' : ''}${client.value.guide}`)
function move(event: KeyboardEvent) {
  const next = event.key === 'ArrowRight' ? (selected.value + 1) % clients.length : event.key === 'ArrowLeft' ? (selected.value + clients.length - 1) % clients.length : event.key === 'Home' ? 0 : event.key === 'End' ? clients.length - 1 : null
  if (next === null) return
  event.preventDefault()
  selected.value = next
  document.getElementById(`client-tab-${next}`)?.focus()
}
</script>
<template>
  <section class="client-connections" aria-labelledby="client-connections-title">
    <header><span class="section-kicker">{{ locale === 'zh' ? '接入你的工作流' : 'CONNECT YOUR WORKFLOW' }}</span><h2 id="client-connections-title">{{ locale === 'zh' ? '自己的 Key，熟悉的客户端。' : 'Your key. Your familiar tools.' }}</h2><p>{{ locale === 'zh' ? '成员创建 API Key、获取接入地址后，即可按客户端协议配置使用；额度与用量由 Aster 统一管理。' : 'Create a member API key, get your endpoint, and configure a compatible client. Aster keeps quotas and usage in one place.' }}</p></header>
    <div class="client-tabs" role="tablist" :aria-label="locale === 'zh' ? '客户端接入指南' : 'Client setup guides'" @keydown="move"><button v-for="(item,index) in clients" :id="`client-tab-${index}`" :key="item.name" type="button" role="tab" :aria-selected="selected === index" :tabindex="selected === index ? 0 : -1" aria-controls="client-guide" @click="selected=index">{{ item.name }}</button></div>
    <div id="client-guide" class="client-guide" role="tabpanel" :aria-labelledby="`client-tab-${selected}`" tabindex="0">
      <div class="client-endpoint"><SiteIcon name="key" :size="28" /><h3>{{ client.name }}</h3><p>{{ client.protocol }}</p><dl><dt>{{ locale === 'zh' ? '接入地址示例' : 'Example base URL' }}</dt><dd>{{ client.endpoint }}</dd></dl><p>{{ locale === 'zh' ? '请替换为管理员提供的实际 HTTPS 地址，使用成员 Key，而不是订阅账号的凭据。' : 'Replace this with the HTTPS endpoint provided by your administrator. Use a member key, not subscription credentials.' }}</p></div>
      <div>
        <div v-if="client.name === 'Codex'" class="codex-workflow">
          <h3>{{ locale === 'zh' ? '换一种接入方式，不换你的 Codex 工作流。' : 'A different way to connect. The same Codex workflow.' }}</h3>
          <p>{{ locale === 'zh' ? '用成员 Key 完成一次配置后，像平常一样打开 Codex、开始任务并继续工作。无需反复输入 Key，也不必改用第三方客户端；接入、额度与用量记录交给 Aster。' : 'Set up your member key once, then open Codex and get to work as usual. No repeated key entry or third-party client to switch to. Aster handles the connection, quotas and usage records.' }}</p>
        </div>
        <ol><li v-for="step in (locale === 'zh' ? client.zh : client.en)" :key="step">{{ step }}</li></ol>
        <p class="client-caveat">{{ locale === 'zh' ? client.noteZh : client.noteEn }}</p>
        <div class="client-guide-links">
          <a class="aster-client-guide" :href="guideUrl" target="_blank" rel="noopener noreferrer">{{ client.name === 'Cursor' ? (locale === 'zh' ? 'Aster 成员指南' : 'Aster member guide') : (locale === 'zh' ? `Aster ${client.name} 接入指南` : `Aster ${client.name} setup guide`) }}<SiteIcon name="chevron-right" :size="18" /></a>
          <a class="official-client-guide" :href="client.docs" target="_blank" rel="noopener noreferrer">{{ locale === 'zh' ? '客户端官方文档' : 'Official client documentation' }}<SiteIcon name="chevron-right" :size="18" /></a>
        </div>
      </div>
    </div>
  </section>
</template>
<style scoped>
.codex-workflow{margin-bottom:28px;padding-bottom:24px;border-bottom:1px solid var(--line)}.codex-workflow h3{margin:0;font-size:var(--font-size-title);line-height:1.4;letter-spacing:-.025em}.codex-workflow p{margin:12px 0 0}.client-guide-links{display:flex;flex-wrap:wrap;gap:16px 28px;align-items:center}.aster-client-guide{font-weight:700}.client-guide-links a{min-height:44px}
.client-connections{margin-top:88px}.client-connections header{max-width:840px}.client-connections h2{font-size:var(--font-size-display);line-height:1.2;letter-spacing:-.035em;margin:18px 0}.client-connections p{font-size:var(--font-size-body);line-height:1.75;color:var(--muted)}.client-tabs{display:flex;gap:12px;flex-wrap:wrap;margin:32px 0 0;border-bottom:1px solid var(--line)}.client-tabs button{padding:14px 24px;border:0;border-bottom:3px solid transparent;background:transparent;font-size:var(--font-size-body);cursor:pointer}.client-tabs button[aria-selected=true]{color:var(--accent-dark);border-color:var(--accent);font-weight:700}.client-guide{display:grid;grid-template-columns:minmax(260px,.8fr) minmax(0,1.5fr);gap:48px;padding:32px;border:1px solid var(--line);border-top:0;border-radius:0 0 16px 16px;background:var(--paper-strong)}.client-endpoint{min-width:0}.client-endpoint>.site-icon{color:var(--accent)}.client-endpoint h3{font-size:var(--font-size-title);margin:16px 0 8px}.client-endpoint dl{margin:24px 0}.client-endpoint dt{font-size:var(--font-size-body);color:var(--muted)}.client-endpoint dd{margin:8px 0;font-size:var(--font-size-body);overflow-wrap:anywhere}.client-guide ol{margin:0;padding-left:24px;display:grid;gap:20px;font-size:var(--font-size-body);line-height:1.75}.client-guide li{padding-left:10px}.client-guide li::marker{color:var(--accent);font-weight:700}.client-guide .client-caveat{font-size:var(--font-size-body);padding-top:20px;border-top:1px solid var(--line);margin-top:24px}.client-guide a{display:inline-flex;gap:8px;align-items:center;color:var(--accent-dark);font-size:var(--font-size-body)}@media(max-width:760px){.client-guide{grid-template-columns:1fr;gap:20px;padding:24px 18px}.client-tabs{gap:0}.client-tabs button{padding:12px 15px}.client-connections{margin-top:56px}}
</style>
