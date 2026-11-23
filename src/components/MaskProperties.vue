<script setup lang="ts">
import { computed, ref } from 'vue'
import { clamp, timeLabel } from '../editing'
import { deleteMaskKeyframe, fullMask, manualCorrection, maskAt, retimeMask } from '../mask-tracking'
import { redactionsFit, updateInterval } from '../tracks'
import type { Project, Redaction, TrackSelection } from '../types'
const props = defineProps<{ project: Project; selection: TrackSelection | null; time: number; drawing: boolean; correcting: boolean }>()
const emit = defineEmits<{ select: [TrackSelection | null]; draw: []; error: [string]; update: [Redaction]; seek: [number]; track: [{ regionId: string; start?: number }]; correct: [string]; stopCorrection: [] }>()
const region = computed(() => props.project.edits.redactions.find(item => item.id === props.selection?.id))
const active = computed(() => !!region.value && props.time >= region.value.start && props.time < region.value.end)
const current = computed(() => region.value ? maskAt(region.value, clamp(props.time, region.value.start, region.value.end)) : { ...fullMask(), lost: false })
const keyTime = ref<string>('')
const keyframes = computed(() => region.value?.tracking?.keyframes ?? [])
const selectedKey = computed(() => keyframes.value.find(key => String(key.time) === keyTime.value))
const keyIndex = computed(() => keyframes.value.findIndex(key => key === selectedKey.value))
const manualHere = computed(() => keyframes.value.find(key => key.kind === 'manual' && Math.abs(key.time - props.time) < 1 / props.project.fps / 4))
function apply(value: Redaction | string) { if (typeof value === 'string') emit('error', value); else emit('update', value) }
function select(id: string) { keyTime.value = ''; emit('select', { kind: 'redactions', id }) }
function mode(event: Event) { if (region.value) apply({ ...region.value, mode: (event.target as HTMLSelectElement).value as Redaction['mode'], tracking: region.value.tracking ? { ...region.value.tracking, reviewed: false } : null }) }
function strength(event: Event) { if (region.value) apply({ ...region.value, strength: Number((event.target as HTMLInputElement).value), tracking: region.value.tracking ? { ...region.value.tracking, reviewed: false } : null }) }
function geometry(field: 'x' | 'y' | 'width' | 'height', event: Event) {
  const item = region.value; if (!item || item.tracking && !active.value) return
  const value = Number((event.target as HTMLInputElement).value) / 100; if (!Number.isFinite(value)) return
  const rectangle = { ...current.value }
  if (field === 'x' || field === 'y') { rectangle[field] = clamp(value, 0, .999); const dimension = field === 'x' ? 'width' : 'height'; rectangle[dimension] = Math.min(rectangle[dimension], 1 - rectangle[field]) }
  else rectangle[field] = clamp(value, .001, 1 - (field === 'width' ? rectangle.x : rectangle.y))
  apply(item.tracking ? manualCorrection(item, props.time, rectangle) : { ...item, x: rectangle.x, y: rectangle.y, width: rectangle.width, height: rectangle.height })
}
function interval(field: 'start' | 'end', event: Event) {
  const item = region.value; if (!item) return
  const proposed = { ...item }; updateInterval(proposed, field, Number((event.target as HTMLInputElement).value), props.project.duration)
  if (!redactionsFit(props.project.edits.redactions.map(value => value.id === item.id ? proposed : value))) { emit('error', 'At most 16 redactions can overlap.'); return }
  apply(retimeMask(item, proposed.start, proposed.end))
}
function add() {
  if (props.project.edits.redactions.length >= 100) return
  const start = clamp(props.time, props.project.edits.trimStart, props.project.edits.trimEnd - .1)
  const item: Redaction = { id: crypto.randomUUID(), start, end: Math.min(start + 3, props.project.edits.trimEnd), x: .25, y: .25, width: .5, height: .2, mode: 'solid', strength: 16 }
  if (!redactionsFit([...props.project.edits.redactions, item])) { emit('error', 'At most 16 redactions can overlap. Adjust their timing first.'); return }
  props.project.edits.redactions.push(item); select(item.id)
}
function remove() { props.project.edits.redactions = props.project.edits.redactions.filter(item => item.id !== region.value?.id); emit('select', null) }
function chooseKey(value: string) { keyTime.value = value; const key = keyframes.value.find(key => String(key.time) === value); if (key) emit('seek', Math.min(key.time, (region.value?.end ?? key.time) - 1 / props.project.fps / 2)) }
function stepKey(delta: number) { const key = keyframes.value[keyIndex.value + delta]; if (key) chooseKey(String(key.time)) }
function deleteKey() { if (region.value && selectedKey.value) { apply(deleteMaskKeyframe(region.value, selectedKey.value.time)); keyTime.value = '' } }
function clear() { const item = region.value; if (!item) return; const value = current.value; apply({ ...item, x: value.x, y: value.y, width: value.width, height: value.height, mode: value.lost ? 'solid' : item.mode, tracking: null }); keyTime.value = '' }
function reviewed() { const item = region.value; if (item?.tracking) apply({ ...item, tracking: { ...item.tracking, reviewed: true } }) }
</script>
<template>
  <section><h3>Blur and masks</h3><p v-if="project.edits.camera.layout === 'fullscreen'" class="field-help">Screen masks are hidden in Camera only. <button type="button" class="text-button" @click="project.edits.camera.layout = 'bubble'">Switch to bubble layout</button> to edit them on the preview.</p>
    <div class="tool-actions"><button type="button" class="small-button" :disabled="project.edits.redactions.length >= 100 || project.edits.camera.layout === 'fullscreen'" @click="add">Add mask</button><button type="button" class="small-button" :aria-pressed="drawing" :disabled="project.edits.redactions.length >= 100 || project.edits.camera.layout === 'fullscreen'" @click="emit('draw')">{{ drawing ? 'Cancel drawing' : 'Draw on preview' }}</button></div>
    <p class="field-help">Use a solid mask to hide sensitive fields. Blur softens content. Original recordings stay unchanged.</p>
    <template v-if="project.edits.redactions.length">
      <label for="redaction-selection">Region</label><select id="redaction-selection" :value="region?.id ?? ''" @change="select(($event.target as HTMLSelectElement).value)"><option disabled value="">Choose a region</option><option v-for="(item, i) in project.edits.redactions" :key="item.id" :value="item.id">Region {{ i + 1 }} ({{ timeLabel(item.start, true) }})</option></select>
      <template v-if="region">
        <label for="redaction-mode">Effect</label><select id="redaction-mode" :value="region.mode" @change="mode"><option value="solid">Solid black mask</option><option value="blur">Blur</option></select><template v-if="region.mode === 'blur'"><label for="blur-strength">Blur strength <output>{{ region.strength }} px</output></label><input id="blur-strength" :value="region.strength" type="range" min="1" max="50" step="1" @change="strength" /></template>
        <p v-if="region.tracking && !active" class="field-help">Move the playhead inside this region to correct its position. <button type="button" class="text-button" @click="emit('seek', region.start)">Go to region start</button></p>
        <div class="field-grid"><div v-for="field in (['x', 'y', 'width', 'height'] as const)" :key="field"><label :for="`region-${field}`">{{ { x: 'Region left', y: 'Region top', width: 'Region width', height: 'Region height' }[field] }} %</label><input :id="`region-${field}`" type="number" min="0" max="100" step=".1" :disabled="!!region.tracking && !active" :value="(current[field] * 100).toFixed(2)" @change="geometry(field, $event)" /></div></div>
        <div class="field-grid"><div><label for="redaction-start">Region start (s)</label><input id="redaction-start" type="number" min="0" step=".01" :value="region.start.toFixed(3)" @change="interval('start', $event)" /></div><div><label for="redaction-end">Region end (s)</label><input id="redaction-end" type="number" min=".1" step=".01" :value="region.end.toFixed(3)" @change="interval('end', $event)" /></div></div>
        <div class="section-divider"></div><h3>Motion tracking</h3><p class="field-help">Tracks the source screen for this region, up to 300 seconds. Review the mask before export.</p>
        <button type="button" class="full-width" :disabled="region.end - region.start > 300 + 1e-9" @click="emit('track', { regionId: region.id })">{{ region.tracking ? 'Track region again' : 'Track region' }}</button><p v-if="region.end - region.start > 300 + 1e-9" class="field-help">Shorten the region to 300 seconds or less to track it.</p>
        <template v-if="region.tracking"><p role="status"><strong>{{ region.tracking.reviewed ? 'Tracking reviewed' : 'Tracking needs review' }}</strong></p><p v-if="current.lost && active" class="field-help">Tracking is lost here. The full screen stays masked until a manual correction.</p>
          <div class="tool-actions"><button type="button" class="small-button" :disabled="!active || project.edits.camera.layout === 'fullscreen'" @click="correcting ? emit('stopCorrection') : emit('correct', region.id)">{{ correcting ? 'Cancel correction' : 'Draw correction on original' }}</button><button type="button" class="small-button" :disabled="!manualHere || manualHere.time + .1 > region.end + 1e-9" @click="emit('track', { regionId: region.id, start: manualHere!.time })">Retrack from correction</button></div><p class="field-help">Dragging a tracked mask or changing its position adds a manual keyframe at the playhead.</p>
          <label for="mask-keyframes">Tracking keyframe <output>{{ keyframes.length }} frames</output></label><select id="mask-keyframes" :value="keyTime" :size="Math.min(5, Math.max(2, keyframes.length + 1))" @change="chooseKey(($event.target as HTMLSelectElement).value)"><option disabled value="">Choose a keyframe</option><option v-for="key in keyframes" :key="key.time" :value="String(key.time)">{{ timeLabel(key.time, true) }} {{ key.kind }}{{ key.kind === 'auto' ? ` ${Math.round(key.confidence * 100)}%` : '' }}</option></select>
          <div class="tool-actions"><button type="button" class="small-button" :disabled="keyIndex <= 0" @click="stepKey(-1)">Previous keyframe</button><button type="button" class="small-button" :disabled="keyIndex >= keyframes.length - 1" @click="stepKey(1)">Next keyframe</button><button type="button" class="small-button" :disabled="!selectedKey || keyIndex === 0 || selectedKey.kind === 'lost'" @click="deleteKey">Delete keyframe</button></div>
          <p v-if="selectedKey?.kind === 'lost'" class="field-help">Lost markers remain until corrected. Draw a correction where the target becomes visible.</p>
          <button type="button" class="full-width" :disabled="region.tracking.reviewed" @click="reviewed">Mark tracking reviewed</button><button type="button" class="text-button full-width" @click="clear">Clear tracking, keep current mask</button>
        </template>
        <button type="button" class="danger-text full-width" @click="remove">Delete region</button>
      </template>
    </template>
  </section>
</template>
