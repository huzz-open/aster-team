<script setup lang="ts">
import { computed } from 'vue'

const props = defineProps<{ locale: 'zh' | 'en'; email: string }>()
const copy = computed(() => props.locale === 'zh' ? {
  title: '联系我们', intro: '欢迎在 GitHub Issues 交流想法、分享使用体验。',
  feedback: '产品问题与建议', private: '商务或私密沟通',
} : {
  title: 'Get in touch', intro: 'Share ideas and experiences in GitHub Issues.',
  feedback: 'Product questions and feedback', private: 'Business or private enquiries',
})
</script>

<template>
  <section class="contact-links" :aria-label="copy.title">
    <h3>{{ copy.title }}</h3>
    <p>{{ copy.intro }}</p>
    <dl class="contact-alternatives">
      <div><dt>{{ copy.feedback }}</dt><dd><a href="https://github.com/huzz-open/aster-team/issues" target="_blank" rel="noopener noreferrer">GitHub Issues</a></dd></div>
      <div v-if="email"><dt>{{ copy.private }}</dt><dd><a :href="`mailto:${email}`">{{ email }}</a></dd></div>
    </dl>
  </section>
</template>

<style scoped>
.contact-links{margin-top:32px;max-width:550px}
.contact-links h3{margin:0;font-size:var(--font-size-title);line-height:1.4;letter-spacing:-.02em}
.contact-links>p{margin:10px 0 18px;color:var(--muted);font-size:var(--font-size-body);line-height:1.65}
.contact-alternatives{display:grid;gap:12px;margin:20px 0 0;font-size:var(--font-size-body);line-height:1.6}
.contact-alternatives>div{display:flex;flex-wrap:wrap;gap:4px 14px;justify-content:space-between}
.contact-alternatives dt{color:var(--muted)}
.contact-alternatives dd{margin:0;min-width:0;overflow-wrap:anywhere}
.contact-alternatives a{color:var(--accent-dark);text-underline-offset:4px}
a:focus-visible{outline:3px solid var(--accent);outline-offset:4px}
</style>
