<script setup lang="ts">
import { computed } from 'vue'
import type { Project, ReframeKeyframe } from '../types'
import { keptClips } from '../video-edits'
const props = defineProps<{ project: Project; selected: string | null; adding: boolean }>()
const emit = defineEmits<{ add: []; select: [string]; remove: [string]; update: [{ id: string; field: 'x' | 'y' | 'scale'; value: number }] }>()
const keys = computed(() => props.project.edits.reframeKeyframes ?? [])
const active = computed(() => keys.value.find(key => key.id === props.selected))
const ranges = computed(() => keptClips(props.project.edits))
const kept = (key: ReframeKeyframe) => ranges.value.some(clip => key.time >= clip.start && key.time < clip.end)
function change(field: 'x' | 'y' | 'scale', event: Event) {
  if (!active.value) return
  const input = event.target as HTMLInputElement
  emit('update', { id: active.value.id, field, value: Number(input.value) / (field === 'scale' ? 1 : 100) })
  input.value = String(active.value[field] * (field === 'scale' ? 1 : 100))
}
</script>

<template>
  <div class="section-divider"></div>
  <h3>Manual framing</h3>
  <p class="field-help">Focus moves between keyframes in source time. Use Fill or a scale above 1 to move within the recording crop. Manual focus replaces automatic zoom focus; their scales multiply.</p>
  <button type="button" class="full-width" :disabled="adding || project.edits.camera.layout === 'fullscreen'" @click="emit('add')">{{ adding ? 'Waiting for frame...' : 'Set keyframe at playhead' }}</button>
  <p v-if="!keys.length" class="field-help">Pause at the frame to focus, then set a keyframe.</p>
  <template v-else>
    <label for="reframe-key">Framing keyframe ({{ keys.length }}/3000)</label>
    <select id="reframe-key" :value="selected ?? ''" @change="emit('select', ($event.target as HTMLSelectElement).value)">
      <option v-if="!active" value="" disabled>Select a keyframe</option>
      <option v-for="(key, index) in keys" :key="key.id" :value="key.id">{{ index + 1 }}: {{ key.time.toFixed(3) }} s{{ kept(key) ? '' : ' (outside kept video)' }}</option>
    </select>
    <template v-if="active">
      <p v-if="!kept(active)" class="field-help" role="status">This keyframe is outside kept video. The preview seeks to a kept frame. Setting a new keyframe uses that visible frame.</p>
      <div class="field-grid">
        <div><label for="reframe-x">Framing horizontal %</label><input id="reframe-x" type="number" min="0" max="100" step="1" :value="Number((active.x * 100).toFixed(3))" @change="change('x', $event)" /></div>
        <div><label for="reframe-y">Framing vertical %</label><input id="reframe-y" type="number" min="0" max="100" step="1" :value="Number((active.y * 100).toFixed(3))" @change="change('y', $event)" /></div>
      </div>
      <label for="reframe-scale">Framing scale</label><input id="reframe-scale" type="number" min="1" max="3" step=".1" :value="Number(active.scale.toFixed(3))" @change="change('scale', $event)" />
      <button type="button" class="danger-text full-width" @click="emit('remove', active.id)">Delete framing keyframe</button>
    </template>
  </template>
  <div class="section-divider"></div>
</template>
