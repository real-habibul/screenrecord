<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import type { AudioBus, Project, VolumeKeyframe } from '../types'
import { keptClips, outputToSource, sourceToOutput } from '../video-edits'
import { insertVolumeKey, moveVolumeKey, projectVolumeKeys, volumeAt } from '../volume-keyframes'

const props = defineProps<{ project: Project; readPlayhead: () => number; disabled?: boolean }>()
const emit = defineEmits<{ beginEdit: []; endEdit: [commit?: boolean]; seek: [number]; error: [string] }>()
const names: Record<AudioBus, string> = { microphone: 'Microphone', system: 'Computer', music: 'Music' }
const tracks = computed<AudioBus[]>(() => {
  const result: AudioBus[] = []
  if (props.project.microphonePath) result.push('microphone')
  if (props.project.systemAudioPath) result.push('system')
  if (props.project.edits.music.length) result.push('music')
  return result
})
const track = ref<AudioBus>('microphone')
const selectedId = ref<string | null>(null)
const lane = ref<HTMLElement>()
const keys = computed(() => props.project.edits.audioProcessing.volumeKeyframes?.[track.value] ?? [])
const clips = computed(() => keptClips(props.project.edits))
const duration = computed(() => clips.value.at(-1)?.outputEnd ?? 0)
const graph = computed(() => projectVolumeKeys(keys.value, clips.value, duration.value))
const selected = computed(() => keys.value.find(key => key.id === selectedId.value))
const visiblePoint = computed(() => graph.value.visible.find(point => point.key.id === selectedId.value))
const buttons = new Map<string, HTMLButtonElement>()
function remember(id: string, element: unknown) {
  if (element instanceof HTMLButtonElement) buttons.set(id, element)
  else buttons.delete(id)
}
function store(points: VolumeKeyframe[]) {
  const processing = props.project.edits.audioProcessing
  processing.volumeKeyframes = { ...processing.volumeKeyframes, [track.value]: points }
  if (!Object.values(processing.volumeKeyframes).some(row => row.length)) delete processing.volumeKeyframes
}
function commit(points: VolumeKeyframe[]) {
  if (props.disabled || drag) return
  emit('beginEdit')
  store(points)
  emit('endEdit', true)
}
function addPoint() {
  if (props.disabled || !tracks.value.length) return
  const time = Math.min(props.project.duration, Math.max(0, props.readPlayhead()))
  try {
    const result = insertVolumeKey(keys.value, time, volumeAt(keys.value, time), props.project.duration)
    commit(result.keys)
    selectedId.value = result.selected
    void nextTick(() => buttons.get(result.selected)?.focus({ preventScroll: true }))
  } catch (error) { emit('error', String(error instanceof Error ? error.message : error)) }
}
function removePoint() {
  if (!selected.value) return
  commit(keys.value.filter(key => key.id !== selectedId.value))
  selectedId.value = null
}
function changeNumber(event: Event, field: 'time' | 'volume') {
  const input = event.target as HTMLInputElement
  const point = selected.value
  if (!point) return
  if (Number.isFinite(input.valueAsNumber)) {
    const time = field === 'time' ? outputToSource(props.project.edits, input.valueAsNumber) : point.time
    const level = field === 'volume' ? input.valueAsNumber / 100 : point.volume
    commit(moveVolumeKey(keys.value, point.id, time, level, props.project.duration))
  }
  const updated = selected.value
  if (updated) input.value = String(field === 'time' ? Number(sourceToOutput(props.project.edits, updated.time).toFixed(3)) : Math.round(updated.volume * 100))
}

let drag: { pointer: number; target: HTMLElement; x: number; y: number; bounds: DOMRect; points: VolumeKeyframe[]; point: VolumeKeyframe; output: number; moved: boolean } | null = null
function start(event: PointerEvent, point: VolumeKeyframe, output: number) {
  if (props.disabled || !lane.value || drag || !event.isPrimary || event.button !== 0) return
  selectedId.value = point.id
  const target = lane.value
  const handle = event.currentTarget as HTMLElement
  handle.focus({ preventScroll: true })
  drag = { pointer: event.pointerId, target, x: event.clientX, y: event.clientY, bounds: target.getBoundingClientRect(), points: keys.value, point, output, moved: false }
  emit('beginEdit')
  target.setPointerCapture(event.pointerId)
  window.addEventListener('keydown', dragKey, true)
  window.addEventListener('blur', cancelDrag)
  event.preventDefault()
}
function move(event: PointerEvent) {
  if (!drag || drag.pointer !== event.pointerId) return
  const dx = event.clientX - drag.x
  const dy = event.clientY - drag.y
  if (!drag.moved && Math.hypot(dx, dy) < 3) return
  drag.moved = true
  const output = drag.output + dx / Math.max(1, drag.bounds.width) * duration.value
  const time = dx === 0 ? drag.point.time : outputToSource(props.project.edits, output)
  const level = drag.point.volume - dy / Math.max(1, drag.bounds.height)
  store(moveVolumeKey(drag.points, drag.point.id, time, level, props.project.duration))
}
function finish(commitChanges: boolean) {
  const active = drag
  if (!active) return
  drag = null
  window.removeEventListener('keydown', dragKey, true)
  window.removeEventListener('blur', cancelDrag)
  if (active.target.hasPointerCapture(active.pointer)) active.target.releasePointerCapture(active.pointer)
  emit('endEdit', commitChanges)
}
function cancelDrag() { finish(false) }
function dragKey(event: KeyboardEvent) {
  if (event.key === 'Escape') {
    event.preventDefault(); event.stopPropagation(); finish(false)
  } else if (event.key === 'Tab') {
    finish(false)
  }
}
function release(event: PointerEvent) {
  if (drag?.pointer !== event.pointerId) return
  move(event)
  finish(true)
}
function keydown(event: KeyboardEvent, point: VolumeKeyframe, output: number) {
  if (props.disabled) return
  if (event.key === 'Escape' && drag) { event.preventDefault(); event.stopPropagation(); finish(false); return }
  if (drag) return
  selectedId.value = point.id
  if (event.key === 'Delete' || event.key === 'Backspace') {
    event.preventDefault(); event.stopPropagation(); removePoint(); return
  }
  if (!['ArrowUp', 'ArrowDown', 'ArrowLeft', 'ArrowRight'].includes(event.key)) return
  event.preventDefault(); event.stopPropagation()
  const horizontal = event.key === 'ArrowLeft' || event.key === 'ArrowRight'
  const direction = event.key === 'ArrowLeft' || event.key === 'ArrowDown' ? -1 : 1
  const time = horizontal ? outputToSource(props.project.edits, output + direction * (event.shiftKey ? 1 : 1 / props.project.fps)) : point.time
  const volume = horizontal ? point.volume : point.volume + direction * (event.shiftKey ? .1 : .01)
  commit(moveVolumeKey(keys.value, point.id, time, volume, props.project.duration))
}
watch(tracks, available => {
  if (!available.includes(track.value)) track.value = available[0] ?? 'microphone'
}, { immediate: true })
watch(track, () => { finish(false); selectedId.value = null }, { flush: 'sync' })
watch(() => props.disabled, value => { if (value) finish(false) }, { flush: 'sync' })
onBeforeUnmount(() => finish(false))
</script>

<template>
  <div class="volume-automation">
    <fieldset class="volume-tools" :disabled="disabled">
      <legend class="sr-only">Volume keyframes</legend>
      <select v-model="track" aria-label="Volume keyframe track"><option v-for="bus in tracks" :key="bus" :value="bus">{{ names[bus] }}</option></select>
      <button type="button" class="small-button" @click="addPoint">Add at playhead</button>
      <button v-if="keys.length" type="button" class="small-button" @click="commit([]); selectedId = null">Reset curve</button>
      <template v-if="selected && visiblePoint">
        <label>Time <input type="number" aria-label="Keyframe time (seconds)" min="0" :max="duration" step="any" :value="Number(visiblePoint.output.toFixed(3))" @change="changeNumber($event, 'time')" /></label>
        <label title="100% uses the track fader level">Level <input type="number" aria-label="Keyframe level (%)" min="0" max="100" step="1" :value="Math.round(selected.volume * 100)" @change="changeNumber($event, 'volume')" />%</label>
        <button type="button" class="small-button" @click="removePoint">Delete point</button>
      </template>
    </fieldset>
    <div class="timeline-row">
      <span class="track-header">Volume</span>
      <div class="volume-lane" :aria-label="`${names[track]} volume envelope`">
        <div ref="lane" class="volume-plot" @pointermove="move" @pointerup="release" @pointercancel="cancelDrag" @lostpointercapture="cancelDrag">
          <svg viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true"><polyline v-for="(path, i) in graph.paths" :key="i" :points="path" fill="none" stroke="currentColor" stroke-width="2" vector-effect="non-scaling-stroke" /></svg>
          <button v-for="{ key, output } in graph.visible" :key="key.id" :ref="el => remember(key.id, el)" class="volume-point" :class="{ selected: key.id === selectedId }"
            type="button" :disabled="disabled" :style="{ left: `clamp(10px, ${output / duration * 100}%, 100% - 10px)`, top: `${(1 - key.volume) * 100}%` }"
            :aria-label="`${names[track]} volume point at ${Number(output.toFixed(3))} seconds, ${Math.round(key.volume * 100)}%`" :aria-pressed="key.id === selectedId"
            @focus="selectedId = key.id" @blur="cancelDrag" @click="selectedId = key.id" @dblclick="emit('seek', key.time)"
            @pointerdown="start($event, key, output)" @keydown="keydown($event, key, output)" />
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.volume-tools { position: sticky; left: 0; display: flex; align-items: center; flex-wrap: wrap; gap: 5px 8px; width: fit-content; max-width: 100cqw; min-width: 0; margin: 0; padding: 4px 10px; border: 0; }
.volume-tools label { display: inline-flex; align-items: center; gap: 4px; margin: 0; font-size: 12px; }
.volume-tools input, .volume-tools select { width: auto; min-height: 28px; padding: 2px 4px; }
.volume-tools input { width: 5.5em; font-variant-numeric: tabular-nums; }
.volume-lane { height: 58px; padding-block: 11px; margin: 1px 0; background: var(--bg); }
.volume-plot { position: relative; height: 100%; color: var(--focus); }
.volume-plot svg { display: block; width: 100%; height: 100%; overflow: visible; }
.volume-point { position: absolute; width: 20px; height: 20px; min-height: 0; min-width: 0; padding: 0; border: 0; transform: translate(-50%, -50%); background: transparent; touch-action: none; cursor: grab; }
.volume-point::after { content: ''; display: block; width: 10px; height: 10px; margin: auto; transform: rotate(45deg); border: 1px solid var(--focus); background: var(--panel); }
.volume-point.selected::after, .volume-point:hover::after { background: var(--focus); }
.volume-point:active { cursor: grabbing; }
</style>
