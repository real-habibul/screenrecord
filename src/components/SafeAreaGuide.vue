<script setup lang="ts">
import { computed } from 'vue'
import { safeAreas, type SafeAreaChoice } from '../safe-area'

const props = defineProps<{ guide: Exclude<SafeAreaChoice, 'off'> }>()
const layout = computed(() => {
  const area = safeAreas[props.guide]
  return {
    gridTemplateRows: `${area.top}% 1fr ${area.bottom}%`,
    gridTemplateColumns: `${area.left}% 1fr ${area.right}%`,
  }
})
</script>

<template>
  <div class="safe-area-guide" :data-guide="guide" :style="layout" aria-hidden="true">
    <span class="safe-edge across"></span>
    <span class="safe-edge"></span><span class="safe-interior"></span><span class="safe-edge"></span>
    <span class="safe-edge across"></span>
  </div>
</template>

<style scoped>
.safe-area-guide { position: absolute; inset: 0; display: grid; pointer-events: none; }
.safe-edge { background: rgb(0 0 0 / 40%); }
.across { grid-column: 1 / -1; }
.safe-interior { border: 1px dashed rgb(255 255 255 / 94%); }
</style>
