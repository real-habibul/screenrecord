<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { waveformColumns, type WaveformSpan } from '../waveform'
import type { AudioAnalysis } from '../types'
const props = defineProps<{ analysis: AudioAnalysis; spans: WaveformSpan[]; duration: number; label: string }>()
const element = ref<SVGSVGElement>()
const width = ref(800); const height = ref(28)
let resize: ResizeObserver | undefined
const drawing = computed(() => {
  const columns = waveformColumns(props.analysis, props.spans, width.value, props.duration)
  return columns.map((column, index) => { const x = ((index + .5) / columns.length * width.value).toFixed(2); return `M${x},${(height.value / 2 - Math.min(1, column.max) * (height.value / 2 - 1)).toFixed(2)}V${(height.value / 2 - Math.max(-1, column.min) * (height.value / 2 - 1)).toFixed(2)}` }).join('')
})
onMounted(() => { resize = new ResizeObserver(() => { if (element.value) { width.value = Math.max(1, element.value.clientWidth); height.value = Math.max(2, element.value.clientHeight) } }); if (element.value) resize.observe(element.value) })
onBeforeUnmount(() => resize?.disconnect())
</script>
<template><svg ref="element" class="audio-waveform" role="img" :aria-label="label" :viewBox="`0 0 ${width} ${height}`" preserveAspectRatio="none" width="800" height="28"><path :d="drawing" stroke="currentColor" stroke-width="1" fill="none" /></svg></template>
<style scoped>.audio-waveform { display: block; width: 100%; height: 28px; color: var(--text); pointer-events: none; }</style>
