<script setup lang="ts">
const props = withDefaults(defineProps<{ steps: string[]; current: number; maxReached?: number; interactive?: boolean; vertical?: boolean }>(), { interactive: false, vertical: false })
const emit = defineEmits<{ select: [step: number] }>()
</script>

<template>
  <ol class="workflow-stepper" :class="{ 'is-vertical': vertical }" :style="{ '--workflow-columns': steps.length }" aria-label="办理进度">
    <li v-for="(label, index) in steps" :key="label" :class="{ active: current === index + 1, complete: (props.maxReached ?? current) > index + 1 }">
      <button v-if="interactive" type="button" :disabled="!!props.maxReached && index + 1 > props.maxReached" :aria-current="current === index + 1 ? 'step' : undefined" @click="emit('select', index + 1)">
        <span class="workflow-node">{{ (props.maxReached ?? current) > index + 1 && current !== index + 1 ? '✓' : index + 1 }}</span>
        <strong>{{ label }}</strong>
      </button>
      <template v-else>
        <span class="workflow-node">{{ (props.maxReached ?? current) > index + 1 && current !== index + 1 ? '✓' : index + 1 }}</span>
        <strong>{{ label }}</strong>
      </template>
    </li>
  </ol>
</template>

<style scoped>
.workflow-stepper{display:grid;grid-template-columns:repeat(var(--workflow-columns),minmax(0,1fr));gap:0;margin:0;padding:0;list-style:none}.workflow-stepper li{position:relative;display:grid;justify-items:center;gap:8px;min-width:0;color:var(--muted);font-size:var(--font-size-body);text-align:center}.workflow-stepper li>button{display:grid;justify-items:center;gap:8px;border:0;padding:0;background:transparent;color:inherit;font:inherit}.workflow-stepper li:not(:last-child)::after{content:"";position:absolute;top:16px;left:calc(50% + 25px);right:calc(-50% + 25px);height:2px;background:var(--line)}.workflow-node{position:relative;z-index:1;display:grid;place-items:center;width:34px;height:34px;border:1px solid var(--line);border-radius:999px;background:var(--surface);font-weight:700;font-variant-numeric:tabular-nums}.workflow-stepper li.active{color:var(--accent)}.workflow-stepper li.active .workflow-node{border-color:var(--accent);background:var(--accent);color:#fff}.workflow-stepper li.complete{color:var(--positive)}.workflow-stepper li.complete .workflow-node{border-color:var(--positive);background:var(--positive);color:#fff}.workflow-stepper li.complete::after{background:var(--positive)}
.workflow-stepper.is-vertical{--workflow-node-size:34px;grid-template-columns:minmax(0,1fr);align-content:start;gap:0;padding:14px;border:1px solid var(--line);border-radius:14px;background:var(--surface)}.workflow-stepper.is-vertical li{grid-template-columns:var(--workflow-node-size) minmax(0,1fr);align-items:center;justify-items:start;gap:14px;min-height:64px;padding:8px 12px;text-align:left}.workflow-stepper.is-vertical li>button{grid-column:1/-1;width:100%;grid-template-columns:var(--workflow-node-size) minmax(0,1fr);align-items:center;justify-items:start;gap:14px;text-align:left}.workflow-stepper.is-vertical li:not(:last-child)::after{top:calc(8px + var(--workflow-node-size));bottom:-8px;left:calc(12px + var(--workflow-node-size) / 2);right:auto;width:2px;height:auto}.workflow-stepper.is-vertical strong{font-weight:650}
.workflow-stepper.is-vertical li>button:not(:disabled){cursor:pointer}.workflow-stepper.is-vertical li>button:not(:disabled):hover strong{text-decoration:underline;text-underline-offset:3px}.workflow-stepper.is-vertical li>button:disabled{cursor:default}
@media(max-width:900px){.workflow-stepper{overflow-x:auto;grid-template-columns:repeat(var(--workflow-columns),110px);padding-bottom:6px}.workflow-stepper li:not(:last-child)::after{left:72px;right:-38px}}
</style>
