<script setup lang="ts">
import { computed } from 'vue'
import { demoDays, tokens, usage, type DemoLog } from './product-demo'
const props = defineProps<{ logs: DemoLog[]; locale: 'zh' | 'en' }>()
const stats = computed(() => usage(props.logs))
const points = computed(() => stats.value.daily.map((value, index) => `${30 + index * 100},${175 - value / Math.max(...stats.value.daily, 1) * 135}`).join(' '))
const shares = computed(() => [stats.value.input, stats.value.cache, stats.value.output].map(value => Math.round(value / Math.max(stats.value.raw, 1) * 100)))
const labels = computed(() => props.locale === 'zh' ? ['输入', '缓存', '输出'] : ['Input', 'Cache', 'Output'])
</script>
<template>
  <div class="demo-usage-charts">
    <section class="demo-trend"><header><strong>{{ locale === 'zh' ? 'Token 使用趋势' : 'Token usage trend' }}</strong><span>{{ tokens(stats.billed) }} Token</span></header>
      <svg viewBox="0 0 660 220" role="img" :aria-label="`${locale === 'zh' ? '每日结算 Token' : 'Daily billed tokens'}: ${stats.daily.join(', ')}`"><path d="M30 40H630M30 85H630M30 130H630M30 175H630" fill="none" stroke="#ebeaf1"/><polygon :points="`30,175 ${points} 630,175`" fill="#7161e51a"/><polyline :points="points" fill="none" stroke="#7161e5" stroke-width="3" stroke-linejoin="round"/><text v-for="(day,index) in demoDays" :key="day" :x="30+index*100" y="205" text-anchor="middle">{{ day }}</text></svg>
    </section>
    <section class="demo-breakdown"><header><strong>{{ locale === 'zh' ? 'Token 构成' : 'Token composition' }}</strong></header><div class="demo-composition-bar"><i v-for="(share,index) in shares" :key="index" :style="{ width: `${share}%` }"></i></div><dl><div v-for="(label,index) in labels" :key="label"><dt>{{ label }}</dt><dd>{{ shares[index] }}%</dd></div></dl><h4>{{ locale === 'zh' ? '模型请求占比' : 'Requests by model' }}</h4><div v-for="model in stats.models" :key="model.model" class="demo-model-share"><span>{{ model.model }}</span><strong>{{ Math.round(model.requests / Math.max(stats.requests,1)*100) }}%</strong><b><i :style="{width:`${model.requests / Math.max(stats.requests,1)*100}%`}"></i></b></div></section>
  </div>
</template>
<style scoped>
.demo-usage-charts{display:grid;grid-template-columns:minmax(0,1.65fr) minmax(220px,1fr);gap:16px;margin-top:20px}.demo-trend,.demo-breakdown{min-width:0;padding:20px;border:1px solid #e5e5ed;border-radius:12px;background:#fff}header{display:flex;flex-wrap:wrap;justify-content:space-between;gap:8px;font-size:var(--font-size-body)}header span{color:#706d7c;font-size:var(--font-size-body)}.demo-trend svg{width:100%;margin-top:28px;display:block}.demo-trend text{font-size:var(--font-size-body);fill:#797685}.demo-composition-bar{display:flex;height:10px;margin:24px 0 18px;border-radius:8px;overflow:hidden}.demo-composition-bar i{background:#7161e5}.demo-composition-bar i:nth-child(2){background:#32a99c}.demo-composition-bar i:nth-child(3){background:#d69b41}dl{margin:0;display:grid;gap:9px}dl>div{display:flex;justify-content:space-between;font-size:var(--font-size-body)}dt{color:#706d7c}dd{margin:0}h4{margin:24px 0 12px;font-size:var(--font-size-body)}.demo-model-share{display:grid;grid-template-columns:1fr auto;gap:7px;margin-top:12px;font-size:var(--font-size-body)}.demo-model-share b{grid-column:1/-1;background:#edeaf7;height:4px}.demo-model-share i{display:block;height:100%;background:#7161e5}@media(max-width:900px){.demo-usage-charts{grid-template-columns:1fr}.demo-trend,.demo-breakdown{padding:16px}}
</style>
