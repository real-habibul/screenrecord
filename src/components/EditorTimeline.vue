<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { timeLabel } from '../editing'
import { dragTrack, dragZoom, trackLanes, type DragMode } from '../tracks'
import { changeSourceTrim, changeVideoBoundary, keptClips, outputDuration, outputToSource, projectedRange, sourceToOutput } from '../video-edits'
import { projectedSubtitle } from '../subtitle-projection'
import { cueRanges } from '../subtitle-editing'
import type { MusicClip, Project, Redaction, SubtitleCue, TrackKind, TrackSelection, ZoomSegment } from '../types'
import type { AudioAnalysisState } from '../audio-analysis'
import { fitAuditionRange, type AuditionRange, type AudioMonitor } from '../audition'
import AuditionControls from './AuditionControls.vue'
import VolumeAutomation from './VolumeAutomation.vue'
import AudioWaveform from './AudioWaveform.vue'
import type { WaveformSpan } from '../waveform'

const props = defineProps<{ project: Project; disabled?: boolean; audioAnalysis: Record<string, AudioAnalysisState>; time: number; selectedVideoClip: string | null; selectedZoom: string | null; selection: TrackSelection | null; auditionOpen: boolean; monitor: AudioMonitor; auditionRange: AuditionRange; loopEnabled: boolean }>()
const emit = defineEmits<{ seek: [number]; selectVideo: [string]; split: []; deleteVideo: []; cutRange: []; error: [string]; selectZoom: [string]; select: [TrackSelection]; beginEdit: []; endEdit: [commit?: boolean]; importMusic: []; subtitles: []; masks: []; tool: [string]; toggleAudition: []; monitor: [AudioMonitor]; auditionRange: [AuditionRange]; loop: [boolean] }>()
const scale = ref(1)
const scales = [1, 2, 4, 8] as const
const volumeOpen = ref(false)
const plane = ref<HTMLElement>()
const scroll = ref<HTMLElement>()
let zoomRevision = 0
let volumeEditing = false
let zoomWheel = { pixels: 0, direction: 0, mode: 0, at: 0 }
type ItemKind = TrackKind | 'screen' | 'zoom'
type Item = MusicClip | SubtitleCue | Redaction | ZoomSegment | { start: number; end: number }
let drag: { kind: ItemKind; id: string; mode: DragMode; x: number; width: number; duration: number; original: Item; timeline: Project['edits']; pointerOutput: number; target: HTMLElement; pointerId: number } | null = null
const duration = computed(() => outputDuration(props.project.edits))
const videoClips = computed(() => keptClips(props.project.edits))
const outputTime = computed(() => sourceToOutput(props.project.edits, props.time))
const readPlayhead = () => props.time
const audioRows = computed(() => [{ source: 'microphone', label: 'Microphone', path: props.project.microphonePath }, { source: 'system', label: 'Computer', path: props.project.systemAudioPath }].filter(row => row.path))
const hasAudio = computed(() => !!(audioRows.value.length || props.project.edits.music.length))
watch(() => props.project, () => { volumeOpen.value = false }, { flush: 'sync' })
const recordedSpans = computed(() => projectedRange(props.project.edits, 0, props.project.duration))
const musicWaveforms = computed(() => {
  const spans: Record<string, WaveformSpan[]> = {}
  for (const clip of props.project.edits.music) for (const part of projectedRange(props.project.edits, clip.start, clip.start + clip.duration)) {
    const sourceStart = clip.sourceStart + part.sourceStart - clip.start
    spans[`${clip.id}:${part.start}:${part.end}`] = [{ sourceStart, sourceEnd: sourceStart + part.end - part.start, start: 0, end: part.end - part.start }]
  }
  return spans
})
const ticks = computed(() => Array.from({ length: 6 * scale.value }, (_, index) => duration.value * index / (6 * scale.value - 1)))
const position = (start: number, end: number) => ({ left: `${start / duration.value * 100}%`, width: `${(end - start) / duration.value * 100}%` })
const selectionDraft = ref<AuditionRange | null>(null)
const displayedRange = computed(() => selectionDraft.value ?? props.auditionRange)
let selecting: { pointer: number; origin: number; target: HTMLElement } | null = null
async function setScale(value: number, pointerX?: number) {
  if (props.disabled || drag || selecting || volumeEditing || !scales.includes(value as typeof scales[number]) || value === scale.value) return
  zoomWheel.pixels = 0
  const viewport = scroll.value
  const bounds = plane.value?.getBoundingClientRect()
  const viewportBounds = viewport?.getBoundingClientRect()
  const x = pointerX ?? (viewportBounds ? viewportBounds.left + 96 + (viewport!.clientWidth - 96) / 2 : 0)
  const fraction = bounds ? Math.min(1, Math.max(0, (x - bounds.left) / Math.max(1, bounds.width))) : 0
  const revision = ++zoomRevision
  scale.value = value
  await nextTick()
  if (revision !== zoomRevision || !viewport || viewport !== scroll.value || !plane.value) return
  const next = plane.value.getBoundingClientRect()
  viewport.scrollLeft += next.left + fraction * next.width - x
}
function zoomBy(direction: number, pointerX?: number) {
  const index = scales.indexOf(scale.value as typeof scales[number])
  void setScale(scales[Math.min(scales.length - 1, Math.max(0, index + direction))], pointerX)
}
function wheel(event: WheelEvent) {
  const viewport = scroll.value
  if (!viewport || event.defaultPrevented) return
  if (drag || selecting || volumeEditing) { zoomWheel.pixels = 0; event.preventDefault(); return }
  if (event.ctrlKey) {
    event.preventDefault()
    const direction = Math.sign(event.deltaY)
    if (!direction) return
    const now = performance.now()
    if (direction !== zoomWheel.direction || event.deltaMode !== zoomWheel.mode || now - zoomWheel.at > 200) zoomWheel.pixels = 0
    const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewport.clientHeight : 1
    zoomWheel = { pixels: zoomWheel.pixels + Math.abs(event.deltaY * unit), direction, mode: event.deltaMode, at: now }
    if (zoomWheel.pixels >= 40) {
      zoomWheel.pixels = 0
      zoomBy(-direction, event.clientX)
    }
    return
  }
  zoomWheel.pixels = 0
  const unit = event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? viewport.clientHeight : 1
  if (event.shiftKey) {
    event.preventDefault()
    viewport.scrollLeft += (event.deltaX || event.deltaY) * (event.deltaMode === 2 ? viewport.clientWidth : unit)
    return
  }
  const target = event.target as HTMLElement
  if (!viewport.contains(target) && !target.closest('input, select, textarea')) {
    event.preventDefault()
    viewport.scrollTop += event.deltaY * unit
    viewport.scrollLeft += event.deltaX * unit
  }
}
function beginVolumeEdit() {
  volumeEditing = true
  zoomWheel.pixels = 0
  emit('beginEdit')
}
function endVolumeEdit(commit = true) {
  volumeEditing = false
  emit('endEdit', commit)
}
function scrollWithKeyboard(event: KeyboardEvent) {
  if (event.target !== event.currentTarget || !scroll.value || event.altKey || event.ctrlKey || event.metaKey) return
  const viewport = scroll.value
  const page = Math.max(32, viewport.clientHeight - 36)
  if (event.key === 'ArrowLeft') viewport.scrollLeft -= 48
  else if (event.key === 'ArrowRight') viewport.scrollLeft += 48
  else if (event.key === 'ArrowUp') viewport.scrollTop -= 34
  else if (event.key === 'ArrowDown') viewport.scrollTop += 34
  else if (event.key === 'PageUp') viewport.scrollTop -= page
  else if (event.key === 'PageDown') viewport.scrollTop += page
  else if (event.key === 'Home') viewport.scrollTop = 0
  else if (event.key === 'End') viewport.scrollTop = viewport.scrollHeight
  else return
  event.preventDefault()
  event.stopPropagation()
}
watch(() => props.project.id, () => {
  zoomRevision++
  zoomWheel.pixels = 0
  scale.value = 1
  scroll.value?.scrollTo({ left: 0, top: 0 })
}, { flush: 'sync' })
onBeforeUnmount(() => { zoomRevision++ })
function pointAt(target: HTMLElement, x: number) {
  const rect = target.getBoundingClientRect()
  return Math.min(duration.value, Math.max(0, (x - rect.left) / Math.max(1, rect.width) * duration.value))
}
function startSelection(event: PointerEvent) {
  if (props.disabled || selecting || !event.isPrimary || event.button !== 0) return
  const target = event.currentTarget as HTMLElement
  selecting = { pointer: event.pointerId, origin: pointAt(target, event.clientX), target }
  target.setPointerCapture(event.pointerId)
  moveSelection(event)
}
function moveSelection(event: PointerEvent) {
  if (selecting?.pointer !== event.pointerId) return
  const current = pointAt(selecting.target, event.clientX)
  selectionDraft.value = fitAuditionRange({ start: Math.min(current, selecting.origin), end: Math.max(current, selecting.origin) }, duration.value)
}
function cancelSelection() {
  const active = selecting
  selecting = null
  selectionDraft.value = null
  if (active?.target.hasPointerCapture(active.pointer)) active.target.releasePointerCapture(active.pointer)
}
function finishSelection(event: PointerEvent) {
  if (selecting?.pointer !== event.pointerId) return
  moveSelection(event)
  if (selectionDraft.value) emit('auditionRange', selectionDraft.value)
  cancelSelection()
}
watch([() => props.disabled, () => props.auditionOpen, () => props.project.id, duration], cancelSelection)
onBeforeUnmount(cancelSelection)
const zooms = computed(() => props.project.edits.zooms.flatMap((zoom, index) => projectedRange(props.project.edits, zoom.start, zoom.end).map(fragment => ({ ...zoom, outputStart: fragment.start, outputEnd: fragment.end, hasStart: fragment.sourceStart === zoom.start, hasEnd: fragment.sourceEnd === zoom.end, fragment: fragment.clipId, index }))))
const tracks = computed(() => [
  { kind: 'music' as const, name: 'Music', clips: props.project.edits.music.map(c => ({ id: c.id, start: c.start, end: c.start + c.duration, label: c.name })) },
  { kind: 'subtitles' as const, name: 'Subtitles', clips: props.project.edits.subtitles.flatMap(cue => { const safe = projectedSubtitle(props.project.edits, cue); return safe ? [{ ...safe, label: safe.text, rawStart: cue.start, rawEnd: cue.end }] : [] }) },
  { kind: 'redactions' as const, name: 'Masks', clips: props.project.edits.redactions.map((c, i) => ({ ...c, label: `${c.mode === 'solid' ? 'Mask' : 'Blur'} ${i + 1}` })) },
].map(track => ({ ...track, clips: trackLanes(track.clips.flatMap((clip, index) => {
  const ranges = track.kind === 'subtitles' ? cueRanges(clip as SubtitleCue) : [{ start: clip.start, end: clip.end }]
  const sourceStart = 'rawStart' in clip ? clip.rawStart : clip.start; const sourceEnd = 'rawEnd' in clip ? clip.rawEnd : clip.end
  return ranges.flatMap((range, rangeIndex) => projectedRange(props.project.edits, range.start, range.end).map(fragment => ({ ...clip, start: fragment.start, end: fragment.end, sourceStart, sourceEnd, hasStart: fragment.sourceStart === sourceStart, hasEnd: fragment.sourceEnd === sourceEnd, fragment: `${rangeIndex}:${fragment.clipId}`, index })))
})) })).filter(track => track.clips.length))

function itemFor(kind: ItemKind, id: string): Item | undefined {
  if (kind === 'screen') return videoClips.value.find(clip => clip.id === id)
  if (kind === 'zoom') return props.project.edits.zooms.find(item => item.id === id)
  return props.project.edits[kind].find(item => item.id === id)
}
function musicWaveSpan(id: string, start: number, end: number) { return musicWaveforms.value[`${id}:${start}:${end}`] }
function select(kind: ItemKind, id: string) {
  if (kind === 'screen') emit('selectVideo', id)
  else if (kind === 'zoom') emit('selectZoom', id)
  else emit('select', { kind, id })
}
function apply(kind: ItemKind, id: string, original: Item, mode: DragMode, delta: number) {
  if (kind === 'screen') {
    if (mode !== 'move') {
      const raw = props.project.edits.videoClips.find(clip => clip.id === id)
      const value = (original as { start: number; end: number })[mode] + delta
      const clipped = raw && (mode === 'start' ? raw.start < props.project.edits.trimStart : raw.end > props.project.edits.trimEnd)
      const message = clipped ? changeSourceTrim(props.project.edits, mode === 'start' ? 'trimStart' : 'trimEnd', value, props.project.duration) : changeVideoBoundary(props.project.edits, id, mode, value, props.project.duration)
      if (message) emit('error', message)
    }
  }
  else if (kind === 'zoom') dragZoom(props.project.edits, original as ZoomSegment, mode, delta, props.project.duration)
  else dragTrack(props.project.edits, { kind, id }, original as MusicClip | SubtitleCue | Redaction, mode, delta, props.project.duration)
}
function startDrag(event: PointerEvent, kind: ItemKind, id: string, mode: DragMode) {
  if (props.disabled || event.button !== 0 || !plane.value) return
  const original = itemFor(kind, id); if (!original) return
  select(kind, id); emit('beginEdit')
  const bounds = plane.value.getBoundingClientRect()
  drag = { kind, id, mode, x: event.clientX, width: bounds.width, duration: duration.value, timeline: JSON.parse(JSON.stringify(props.project.edits)), pointerOutput: (event.clientX - bounds.left) / bounds.width * duration.value, original: JSON.parse(JSON.stringify(original)), target: event.currentTarget as HTMLElement, pointerId: event.pointerId }
  drag.target.focus({ preventScroll: true }); drag.target.setPointerCapture(event.pointerId); event.preventDefault()
}
function moveDrag(event: PointerEvent) {
  if (!drag || props.disabled) return
  const outputDelta = (event.clientX - drag.x) / drag.width * drag.duration
  const endpoint = drag.mode === 'end' ? ('end' in drag.original ? drag.original.end : drag.original.start + drag.original.duration) : drag.original.start
  const sourceDelta = drag.kind === 'screen' ? outputDelta : drag.mode !== 'move' ? outputDelta === 0 ? 0 : outputToSource(drag.timeline, sourceToOutput(drag.timeline, endpoint) + outputDelta) - endpoint : outputToSource(drag.timeline, drag.pointerOutput + outputDelta) - outputToSource(drag.timeline, drag.pointerOutput)
  const delta = Math.round(sourceDelta * props.project.fps) / props.project.fps
  apply(drag.kind, drag.id, drag.original, drag.mode, delta)
}
function endDrag(event: PointerEvent, cancel = false) {
  if (!drag) return
  if (cancel || props.disabled) { cancelDrag(); return }
  moveDrag(event)
  drag = null; const target = event.currentTarget as HTMLElement
  if (target.hasPointerCapture(event.pointerId)) target.releasePointerCapture(event.pointerId)
  emit('endEdit', true)
}
function cancelDrag() { const current = drag; if (!current) return; drag = null; if (current.target.hasPointerCapture(current.pointerId)) current.target.releasePointerCapture(current.pointerId); emit('endEdit', false) }
watch(() => props.disabled, value => { if (value) cancelDrag() }, { flush: 'sync' })
watch(() => props.project.id, cancelDrag, { flush: 'sync' })
onBeforeUnmount(cancelDrag)
function step(event: KeyboardEvent, kind: ItemKind, id: string, mode: DragMode) {
  if (event.key === 'Escape' && drag) { event.preventDefault(); event.stopPropagation(); cancelDrag(); return }
  if (drag) return
  if (props.disabled) return
  if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
  event.preventDefault(); event.stopPropagation()
  const original = itemFor(kind, id); if (!original) return
  select(kind, id); emit('beginEdit'); apply(kind, id, { ...original }, mode, (event.key === 'ArrowRight' ? 1 : -1) * (event.shiftKey ? 1 : 1 / props.project.fps)); emit('endEdit')
}
</script>

<template>
  <section class="timeline multi-timeline" :class="{ 'audio-editing': auditionOpen || volumeOpen }" aria-label="Video timeline" @wheel="wheel">
    <div class="timeline-heading">
      <h2>Timeline</h2>
      <div class="timeline-add"><button type="button" class="small-button" aria-label="Add music at playhead" @click="emit('importMusic')">+ Music</button><button type="button" class="small-button" aria-label="Open subtitle tools" @click="emit('subtitles')">+ Subtitles</button><button type="button" class="small-button" aria-label="Open mask tools" @click="emit('masks')">+ Mask</button></div>
      <div class="timeline-zoom" title="Ctrl+wheel zooms at the pointer. Shift+wheel scrolls horizontally.">
        <label for="timeline-scale">Zoom</label>
        <button type="button" class="small-button timeline-zoom-button" aria-label="Zoom out on timeline" title="Zoom out on timeline" :disabled="disabled || scale === 1" @click="zoomBy(-1)">−</button>
        <select id="timeline-scale" :value="scale" :disabled="disabled" @change="setScale(Number(($event.target as HTMLSelectElement).value))"><option :value="1">Fit</option><option :value="2">2x</option><option :value="4">4x</option><option :value="8">8x</option></select>
        <button type="button" class="small-button timeline-zoom-button" aria-label="Zoom in on timeline" title="Zoom in on timeline" :disabled="disabled || scale === 8" @click="zoomBy(1)">+</button>
      </div>
    </div>
    <div class="video-edit-actions"><button type="button" class="small-button" title="Split at playhead (S)" @click="emit('split')">Split</button><button type="button" class="small-button" @click="emit('cutRange')">Cut range...</button><button type="button" class="small-button" :disabled="!selectedVideoClip" title="Remove selected video clip and close the gap (Delete)" @click="emit('deleteVideo')">Ripple delete</button><button type="button" class="small-button" :aria-expanded="auditionOpen" :disabled="disabled" @click="emit('toggleAudition')">Audition</button><button v-if="hasAudio" type="button" class="small-button" :aria-expanded="volumeOpen" :disabled="disabled" @click="volumeOpen = !volumeOpen">Volume keyframes</button><span>{{ videoClips.length }} {{ videoClips.length === 1 ? 'clip' : 'clips' }} / {{ timeLabel(duration, true) }}</span></div>
    <div ref="scroll" class="timeline-scroll" role="region" aria-label="Timeline tracks" aria-description="Scroll to see more tracks. Shift+wheel scrolls horizontally. Ctrl+wheel zooms at the pointer. Arrow keys scroll. Home and End reach the first and last track." title="Scroll to see more tracks. Shift+wheel scrolls horizontally. Ctrl+wheel zooms at the pointer." tabindex="0" @keydown="scrollWithKeyboard">
      <AuditionControls v-if="auditionOpen" :monitor="monitor" :range="auditionRange" :loop="loopEnabled" :time="outputTime" :duration="duration" :microphone="!!project.microphonePath" :computer="!!project.systemAudioPath" :disabled="disabled" @monitor="emit('monitor', $event)" @range="emit('auditionRange', $event)" @loop="emit('loop', $event)" />
      <div class="timeline-content" :style="{ width: `calc(${scale * 100}% - ${(scale - 1) * 96}px)` }">
      <div class="timeline-ruler-row"><span class="track-header ruler-label">Output time</span><div ref="plane" class="timeline-plane"><div class="timeline-ruler"><span v-for="(tick, i) in ticks" :key="i">{{ timeLabel(tick) }}</span></div><label class="sr-only" for="playhead">Playhead position in seconds</label><input id="playhead" class="playhead" type="range" min="0" :max="duration" :step="1 / project.fps" :value="outputTime" :aria-valuetext="timeLabel(outputTime, true)" @input="emit('seek', outputToSource(project.edits, Number(($event.target as HTMLInputElement).value)))" /></div></div>
      <div class="timeline-row"><button type="button" class="track-header" @click="emit('tool', 'canvas')">Video</button><div class="timeline-lane screen-lane"><div v-for="(clip, index) in videoClips" :key="clip.id" class="timeline-clip screen-clip" :class="{ selected: selectedVideoClip === clip.id }" :style="position(clip.outputStart, clip.outputEnd)"><button v-for="mode in (['start', 'move', 'end'] as const)" :key="mode" type="button" :class="mode === 'move' ? 'clip-body' : 'clip-handle'" :aria-label="mode === 'move' ? `Select video clip ${index + 1}, source ${timeLabel(clip.start)} to ${timeLabel(clip.end)}` : !project.edits.videoClips.length ? `Trim ${mode} of screen recording` : `Trim ${mode} of video clip ${index + 1}`" @click="select('screen', clip.id)" @pointerdown="mode !== 'move' && startDrag($event, 'screen', clip.id, mode)" @pointermove="moveDrag" @pointerup="endDrag($event)" @pointercancel="endDrag($event, true)" @lostpointercapture="cancelDrag" @blur="cancelDrag" @keydown="mode !== 'move' && step($event, 'screen', clip.id, mode)">{{ mode === 'move' ? `${index + 1} / ${timeLabel(clip.end - clip.start, true)}` : '' }}</button></div></div></div>
      <div v-if="zooms.length" class="timeline-row"><button type="button" class="track-header" @click="emit('tool', 'zoom')">Zoom</button><div class="timeline-lane zoom-track"><div v-for="zoom in zooms" :key="`${zoom.id}:${zoom.fragment}`" class="timeline-clip" :data-has-start="zoom.hasStart" :data-has-end="zoom.hasEnd" :class="{ selected: selectedZoom === zoom.id }" :style="position(zoom.outputStart, zoom.outputEnd)"><button v-for="mode in (['start', 'move', 'end'] as const)" :key="mode" type="button" :class="mode === 'move' ? 'clip-body' : 'clip-handle'" :aria-label="mode === 'move' ? `Edit zoom ${zoom.index + 1}, ${timeLabel(zoom.start)} to ${timeLabel(zoom.end)}` : `Trim ${mode} of zoom ${zoom.index + 1}`" :disabled="mode === 'start' && !zoom.hasStart || mode === 'end' && !zoom.hasEnd" :tabindex="mode === 'start' && !zoom.hasStart || mode === 'end' && !zoom.hasEnd ? -1 : 0" @click="select('zoom', zoom.id)" @pointerdown="startDrag($event, 'zoom', zoom.id, mode)" @pointermove="moveDrag" @pointerup="endDrag($event)" @pointercancel="endDrag($event, true)" @lostpointercapture="cancelDrag" @blur="cancelDrag" @keydown="step($event, 'zoom', zoom.id, mode)">{{ mode === 'move' ? `${zoom.scale.toFixed(1)}x` : '' }}</button></div></div></div>
      <VolumeAutomation v-if="volumeOpen && hasAudio" :key="project.id" :project="project" :read-playhead="readPlayhead" :disabled="disabled" @begin-edit="beginVolumeEdit" @end-edit="endVolumeEdit" @seek="emit('seek', $event)" @error="emit('error', $event)" />
      <div v-if="auditionOpen" class="timeline-row"><span class="track-header">Loop range</span><div class="timeline-lane audition-lane" aria-label="Drag to select a loop range" title="Drag to select a loop range" @pointerdown="startSelection" @pointermove="moveSelection" @pointerup="finishSelection" @pointercancel="cancelSelection" @lostpointercapture="cancelSelection"><div class="audition-band" :style="position(displayedRange.start, displayedRange.end)"></div></div></div>
      <div v-for="row in audioRows" :key="row.source" class="timeline-row"><button type="button" class="track-header" @click="emit('tool', 'audio')">{{ row.label }}</button><div class="timeline-lane audio-lane"><AudioWaveform v-if="audioAnalysis[row.source]?.data" :analysis="audioAnalysis[row.source].data!" :spans="recordedSpans" :duration="duration" :label="`${row.label} recorded waveform`" /><button v-else type="button" class="waveform-state" @click="emit('tool', 'audio')">{{ audioAnalysis[row.source]?.state === 'loading' ? `Analyzing ${Math.round(audioAnalysis[row.source].progress * 100)}%` : audioAnalysis[row.source]?.error ? 'Analysis failed. Open audio settings' : 'Open audio settings' }}</button></div></div>
      <div v-for="track in tracks" :key="track.kind" class="timeline-row"><button type="button" class="track-header" @click="emit('tool', track.kind)">{{ track.name }}</button><div class="timeline-lane" :class="`track-${track.kind}`" :style="{ height: `${Math.max(1, ...track.clips.map(clip => clip.lane + 1)) * 30}px` }"><div v-for="clip in track.clips" :key="`${clip.id}:${clip.fragment}`" class="timeline-clip" :data-has-start="clip.hasStart" :data-has-end="clip.hasEnd" :class="{ selected: selection?.kind === track.kind && selection?.id === clip.id }" :style="{ ...position(clip.start, clip.end), top: `${clip.lane * 30 + 2}px` }"><AudioWaveform v-if="track.kind === 'music' && audioAnalysis[`music:${clip.id}`]?.data" class="music-waveform" :analysis="audioAnalysis[`music:${clip.id}`].data!" :spans="musicWaveSpan(clip.id, clip.start, clip.end)" :duration="clip.end - clip.start" :label="`${clip.label} music waveform`" /><button v-for="mode in (['start', 'move', 'end'] as const)" :key="mode" type="button" :class="mode === 'move' ? 'clip-body' : 'clip-handle'" :aria-label="mode === 'move' ? `Edit ${track.name.toLowerCase()} ${clip.index + 1}: ${clip.label}, ${timeLabel(clip.sourceStart)} to ${timeLabel(clip.sourceEnd)}` : `Trim ${mode} of ${track.name.toLowerCase()} ${clip.index + 1}`" :disabled="mode === 'start' && !clip.hasStart || mode === 'end' && !clip.hasEnd" :tabindex="mode === 'start' && !clip.hasStart || mode === 'end' && !clip.hasEnd ? -1 : 0" @click="select(track.kind, clip.id)" @pointerdown="startDrag($event, track.kind, clip.id, mode)" @pointermove="moveDrag" @pointerup="endDrag($event)" @pointercancel="endDrag($event, true)" @lostpointercapture="cancelDrag" @blur="cancelDrag" @keydown="step($event, track.kind, clip.id, mode)">{{ mode === 'move' ? clip.label : '' }}</button></div></div></div>
      <div class="timeline-position" :style="{ left: `calc(96px + (100% - 96px) * ${outputTime / duration})` }" aria-hidden="true"></div>
    </div></div>
  </section>
</template>

<style scoped>
.audio-editing { height: clamp(230px, 36vh, 360px); max-height: 44vh; }
.video-edit-actions { flex-wrap: wrap; }
.timeline-scroll { container-type: inline-size; }
.audition-lane { cursor: crosshair; touch-action: none; user-select: none; }
.audition-band { position: absolute; inset-block: 3px; pointer-events: none; background: var(--accent-soft); border: 1px solid var(--focus); border-radius: 3px; }
</style>
