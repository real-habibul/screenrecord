<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { clamp, timeLabel } from '../editing'
import type { AppearanceSettings, Project, Redaction, RecordedAudioSource, TrackKind, TrackSelection } from '../types'
import type { AudioAnalysisState } from '../audio-analysis'
import AudioProperties from './AudioProperties.vue'
import TrackProperties from './TrackProperties.vue'
import AppearancePresets from './AppearancePresets.vue'
import ReframeProperties from './ReframeProperties.vue'
import { changeSourceTrim, changeVideoBoundary, keptClips, outputDuration } from '../video-edits'

const props = defineProps<{ project: Project; audioAnalysis: Record<string, AudioAnalysisState>; selectedZoom: string | null; selectedReframe: string | null; addingReframe: boolean; selectedVideoClip?: string | null; selection: TrackSelection | null; time: number; drawing: boolean; correcting: boolean }>()
const emit = defineEmits<{ selectZoom: [string | null]; regenerate: []; removeZoom: [string]; select: [TrackSelection | null]; importMusic: []; transcribe: []; exportSrt: []; importSrt: []; draw: []; newCameraRecording: []; error: [string]; activeTool: [string]; analyzeAudio: [string]; normalizeAudio: [RecordedAudioSource]; cancelAnalysis: []; applyAppearance: [AppearanceSettings]; updateMask: [Redaction]; seek: [number]; trackMask: [{ regionId: string; start?: number }]; correctMask: [string]; stopCorrection: []; addReframe: []; selectReframe: [string]; removeReframe: [string]; updateReframe: [{ id: string; field: 'x' | 'y' | 'scale'; value: number }] }>()
const tab = ref('canvas')
watch(tab, value => emit('activeTool', value), { immediate: true, flush: 'sync' })
const tabs = ['canvas', 'zoom', 'cursor', 'camera', 'audio', 'music', 'subtitles', 'redactions']
const tabLabels: Record<string, string> = { redactions: 'Masks' }
const toolIcons: Record<string, string> = { canvas: 'M3 4h18v16H3zM8 4v16', zoom: 'M15 15l6 6M10 3a7 7 0 1 0 0 14 7 7 0 0 0 0-14M7 10h6m-3-3v6', cursor: 'M5 3v17l5-5 4 7 3-2-4-7h7Z', camera: 'M3 6h12v12H3zM15 10l6-4v12l-6-4', audio: 'M3 9h4l5-5v16l-5-5H3zM16 8a6 6 0 0 1 0 8m3-11a10 10 0 0 1 0 14', music: 'M9 18V5l11-2v13M9 8l11-2M9 18a3 2 0 1 0-6 0 3 2 0 0 0 6 0m11-2a3 2 0 1 0-6 0 3 2 0 0 0 6 0', subtitles: 'M3 5h18v14H3zM6 10h5m3 0h4M6 14h12', redactions: 'M3 5h18v14H3zM7 8v8m5-8v8m5-8v8' }
watch(() => props.selection, selection => { if (selection) tab.value = selection.kind })
const activeZoom = computed(() => props.project.edits.zooms.find(z => z.id === props.selectedZoom))
const activeVideo = computed(() => props.project.edits.videoClips.find(clip => clip.id === props.selectedVideoClip) ?? keptClips(props.project.edits).find(clip => clip.id === props.selectedVideoClip))
function trimChange(field: 'trimStart' | 'trimEnd', event: Event) {
  const message = changeSourceTrim(props.project.edits, field, Number((event.target as HTMLInputElement).value), props.project.duration)
  if (message) emit('error', message)
  ;(event.target as HTMLInputElement).value = String(Number(props.project.edits[field].toFixed(3)))
}
function videoChange(field: 'start' | 'end', event: Event) {
  if (!activeVideo.value) return
  const message = changeVideoBoundary(props.project.edits, activeVideo.value.id, field, Number((event.target as HTMLInputElement).value), props.project.duration)
  if (message) emit('error', message)
  ;(event.target as HTMLInputElement).value = String(Number((activeVideo.value?.[field] ?? 0).toFixed(3)))
}
function showZoom() { tab.value = 'zoom' }
function showTool(value: string) { if (tabs.includes(value)) tab.value = value }
function zoomChange(field: 'start' | 'end', event: Event) {
  const z = activeZoom.value; if (!z) return
  const value = Number((event.target as HTMLInputElement).value)
  const others = props.project.edits.zooms.filter(item => item.id !== z.id)
  if (field === 'start') {
    const prev = Math.max(0, ...others.filter(item => item.end <= z.end).map(item => item.end))
    z.start = clamp(value, prev, z.end - .1)
  } else {
    const next = Math.min(props.project.duration, ...others.filter(item => item.start >= z.start).map(item => item.start))
    z.end = clamp(value, z.start + .1, next)
  }
  ;(event.target as HTMLInputElement).value = z[field].toFixed(2)
}
function cropToggle(event: Event) { props.project.edits.crop = (event.target as HTMLInputElement).checked ? { x: 0, y: 0, width: 1, height: 1 } : null }
function backgroundBlurChange(event: Event) {
  const input = event.target as HTMLInputElement; const value = Number(input.value)
  if (Number.isFinite(value)) props.project.edits.backgroundBlur = clamp(value, 4, 64)
  input.value = String(props.project.edits.backgroundBlur ?? 24)
}
function setCrop(field: 'x' | 'y' | 'width' | 'height', event: Event) {
  const crop = props.project.edits.crop; if (!crop) return
  const value = Number((event.target as HTMLInputElement).value) / 100
  if (field === 'x') { crop.x = clamp(value, 0, .95); crop.width = Math.min(crop.width, 1 - crop.x) }
  else if (field === 'y') { crop.y = clamp(value, 0, .95); crop.height = Math.min(crop.height, 1 - crop.y) }
  else crop[field] = clamp(value, .05, 1 - (field === 'width' ? crop.x : crop.y))
}
function tabKey(event: KeyboardEvent) {
  const i = tabs.indexOf(tab.value)
  if (['ArrowRight', 'ArrowLeft', 'ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) {
    event.preventDefault(); tab.value = event.key === 'Home' ? tabs[0] : event.key === 'End' ? tabs[tabs.length - 1] : tabs[(i + (event.key === 'ArrowRight' || event.key === 'ArrowDown' ? 1 : -1) + tabs.length) % tabs.length]
    requestAnimationFrame(() => document.getElementById(`tab-${tab.value}`)?.focus())
  }
}
defineExpose({ showZoom, showTool })
</script>

<template>
  <aside class="inspector" aria-label="Edit settings">
    <div class="inspector-tabs" role="tablist" aria-label="Edit category" aria-orientation="vertical" @keydown="tabKey">
      <button v-for="item in tabs" :id="`tab-${item}`" :key="item" type="button" role="tab" :aria-selected="tab === item" :aria-controls="`panel-${item}`" :tabindex="tab === item ? 0 : -1" @click="tab = item"><svg width="20" height="20" viewBox="0 0 24 24" fill="none" aria-hidden="true"><path :d="toolIcons[item]" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" /></svg><span>{{ tabLabels[item] ?? item[0].toUpperCase() + item.slice(1) }}</span></button>
    </div>
    <div class="inspector-content">
      <p v-if="project.edits.camera.layout === 'fullscreen' && ['canvas', 'zoom', 'cursor'].includes(tab)" class="field-help">Screen video is hidden in Camera only. <button type="button" class="text-button" @click="project.edits.camera.layout = 'bubble'">Switch to bubble layout</button> to preview screen edits.</p>
      <div v-if="['music', 'subtitles', 'redactions'].includes(tab)" :id="`panel-${tab}`" role="tabpanel" :aria-labelledby="`tab-${tab}`"><TrackProperties :project="project" :analyses="audioAnalysis" :kind="tab as TrackKind" :selection="selection" :time="time" :drawing="drawing" :correcting="correcting" @update-mask="emit('updateMask', $event)" @seek="emit('seek', $event)" @track-mask="emit('trackMask', $event)" @correct-mask="emit('correctMask', $event)" @stop-correction="emit('stopCorrection')" @audio-tools="tab = 'audio'" @analyze="emit('analyzeAudio', $event)" @cancel-analysis="emit('cancelAnalysis')" @select="emit('select', $event)" @import-music="emit('importMusic')" @transcribe="emit('transcribe')" @export-srt="emit('exportSrt')" @import-srt="emit('importSrt')" @draw="emit('draw')" @error="emit('error', $event)" /></div>
      <section v-if="tab === 'canvas'" id="panel-canvas" role="tabpanel" aria-labelledby="tab-canvas">
        <AppearancePresets :project="project" @apply="emit('applyAppearance', $event)" @new-camera-recording="emit('newCameraRecording')" />
        <h3>Canvas</h3>
        <label for="aspect">Aspect ratio</label>
        <select id="aspect" v-model="project.edits.aspectRatio"><option value="source">Match recording</option><option value="16:9">16:9 landscape</option><option value="9:16">9:16 portrait</option><option value="1:1">1:1 square</option></select>
        <label for="screen-sizing">Screen sizing</label><select id="screen-sizing" v-model="project.edits.fitMode" :disabled="project.edits.camera.layout === 'fullscreen'"><option value="fit">Fit</option><option value="fill">Fill</option></select>
        <p class="field-help">{{ project.edits.fitMode === 'fill' ? 'Fills the screen area by cropping within your recording crop.' : 'Shows the entire recording crop inside the screen area.' }}</p>
        <ReframeProperties :project="project" :selected="selectedReframe" :adding="addingReframe" @add="emit('addReframe')" @select="emit('selectReframe', $event)" @remove="emit('removeReframe', $event)" @update="emit('updateReframe', $event)" />
        <label for="background-mode">Background style</label><select id="background-mode" v-model="project.edits.backgroundMode" :disabled="project.edits.camera.layout === 'fullscreen' || correcting"><option value="solid">Solid color</option><option value="videoBlur">Blurred video</option></select>
        <template v-if="project.edits.backgroundMode === 'videoBlur'">
          <label for="background-blur">Background blur strength</label><input id="background-blur" type="number" :value="project.edits.backgroundBlur ?? 24" min="4" max="64" step="1" :disabled="project.edits.camera.layout === 'fullscreen' || correcting" @change="backgroundBlurChange" />
          <p class="field-help">Uses only the visible screen crop after masks, framing, and zoom. Camera only and original-frame inspection use the solid color.</p>
        </template>
        <label for="background">{{ project.edits.backgroundMode === 'videoBlur' ? 'Fallback color' : 'Background' }}</label>
        <div class="color-field"><input id="background" v-model="project.edits.background" type="color" /><span>{{ project.edits.background.toUpperCase() }}</span></div>
        <label for="padding">Padding <output>{{ project.edits.padding }} px</output></label>
        <input id="padding" v-model.number="project.edits.padding" type="range" min="0" max="180" step="2" />
        <label for="radius">Corner radius <output>{{ project.edits.cornerRadius }} px</output></label>
        <input id="radius" v-model.number="project.edits.cornerRadius" type="range" min="0" max="80" step="2" />
        <div class="section-divider"></div>
        <label class="check-label"><input type="checkbox" :checked="!!project.edits.crop" @change="cropToggle" />Crop recording</label>
        <div v-if="project.edits.crop" class="field-grid">
          <div v-for="field in (['x', 'y', 'width', 'height'] as const)" :key="field"><label :for="`crop-${field}`">{{ { x: 'Left', y: 'Top', width: 'Width', height: 'Height' }[field] }} %</label><input :id="`crop-${field}`" type="number" :value="Math.round(project.edits.crop[field] * 100)" :min="field === 'width' || field === 'height' ? 5 : 0" max="100" step="1" @change="setCrop(field, $event)" /></div>
        </div>
        <div class="section-divider"></div>
        <h3>Trim</h3>
        <div class="field-grid"><div><label for="trim-start">Start (seconds)</label><input id="trim-start" :value="Number(project.edits.trimStart.toFixed(3))" type="number" min="0" :max="project.edits.trimEnd - .1" step="0.1" @change="trimChange('trimStart', $event)" /></div><div><label for="trim-end">End (seconds)</label><input id="trim-end" :value="Number(project.edits.trimEnd.toFixed(3))" type="number" :min="project.edits.trimStart + .1" :max="project.duration" step="0.1" @change="trimChange('trimEnd', $event)" /></div></div>
        <p class="field-help">Source recording times. {{ timeLabel(outputDuration(project.edits), true) }} remains after cuts.</p>
        <template v-if="activeVideo"><div class="section-divider"></div><h3>Selected video clip</h3><div class="field-grid"><div><label for="video-start">Clip source start (s)</label><input id="video-start" :value="Number(activeVideo.start.toFixed(3))" type="number" min="0" :max="project.duration" step="any" @change="videoChange('start', $event)" /></div><div><label for="video-end">Clip source end (s)</label><input id="video-end" :value="Number(activeVideo.end.toFixed(3))" type="number" min="0" :max="project.duration" step="any" @change="videoChange('end', $event)" /></div></div></template>
      </section>
      <section v-if="tab === 'zoom'" id="panel-zoom" role="tabpanel" aria-labelledby="tab-zoom">
        <div class="section-heading"><h3>Zoom segments</h3><button class="small-button" type="button" :disabled="!project.cursor.length" @click="emit('regenerate')">Auto zoom</button></div>
        <p v-if="!project.edits.zooms.length" class="field-help">Add a zoom at the playhead, or generate zooms from recorded clicks.</p>
        <div v-else class="zoom-list"><button v-for="(z, i) in project.edits.zooms" :key="z.id" type="button" :class="{ selected: selectedZoom === z.id }" @click="emit('selectZoom', z.id)"><span>Zoom {{ i + 1 }}</span><span>{{ timeLabel(z.start, true) }} - {{ timeLabel(z.end, true) }}</span></button></div>
        <div v-if="activeZoom" class="zoom-fields">
          <div class="field-grid"><div><label for="zoom-start">Start (seconds)</label><input id="zoom-start" type="number" :value="activeZoom.start.toFixed(2)" min="0" :max="activeZoom.end - .1" step=".1" @change="zoomChange('start', $event)" /></div><div><label for="zoom-end">End (seconds)</label><input id="zoom-end" type="number" :value="activeZoom.end.toFixed(2)" :min="activeZoom.start + .1" :max="project.duration" step=".1" @change="zoomChange('end', $event)" /></div></div>
          <label for="zoom-scale">Magnification <output>{{ activeZoom.scale.toFixed(1) }}x</output></label><input id="zoom-scale" v-model.number="activeZoom.scale" type="range" min="1" max="3" step=".1" />
          <label class="check-label"><input v-model="activeZoom.followCursor" type="checkbox" />Follow cursor</label>
          <template v-if="!activeZoom.followCursor"><label for="zoom-x">Horizontal focus <output>{{ Math.round(activeZoom.x * 100) }}%</output></label><input id="zoom-x" v-model.number="activeZoom.x" type="range" min="0" max="1" step=".01" /><label for="zoom-y">Vertical focus <output>{{ Math.round(activeZoom.y * 100) }}%</output></label><input id="zoom-y" v-model.number="activeZoom.y" type="range" min="0" max="1" step=".01" /></template>
          <button type="button" class="danger-text full-width" @click="emit('removeZoom', activeZoom.id)">Delete zoom</button>
        </div>
        <p v-else-if="project.edits.zooms.length" class="field-help">Select a segment to edit its timing and focus.</p>
      </section>
      <section v-if="tab === 'cursor'" id="panel-cursor" role="tabpanel" aria-labelledby="tab-cursor">
        <h3>Cursor</h3><p v-if="!project.cursor.length" class="field-help">This recording has no cursor track.</p>
        <fieldset :disabled="!project.cursor.length"><label class="check-label"><input v-model="project.edits.showCursor" type="checkbox" />Show cursor</label><label class="check-label"><input :checked="project.edits.showClicks !== false" :disabled="!project.edits.showCursor" type="checkbox" @change="project.edits.showClicks = ($event.target as HTMLInputElement).checked" />Show click animations</label><label for="cursor-size">Size <output>{{ project.edits.cursorSize.toFixed(1) }}x</output></label><input id="cursor-size" v-model.number="project.edits.cursorSize" type="range" min=".5" max="3" step=".1" /><label for="cursor-smoothing">Smoothing <output>{{ Math.round(project.edits.cursorSmoothing * 100) }}%</output></label><input id="cursor-smoothing" v-model.number="project.edits.cursorSmoothing" type="range" min="0" max="1" step=".05" /></fieldset>
      </section>
      <section v-if="tab === 'camera'" id="panel-camera" role="tabpanel" aria-labelledby="tab-camera">
        <h3>Webcam</h3><div v-if="!project.cameraPath" class="missing-track"><h4>No camera recorded</h4><p class="field-help">This project contains no webcam video. Include a camera in a new recording to use this overlay.</p><button type="button" class="full-width" @click="emit('newCameraRecording')">New recording with camera</button></div>
        <button v-if="!project.cameraPath && project.edits.camera.layout !== 'bubble'" type="button" @click="project.edits.camera.layout = 'bubble'">Show screen instead</button>
        <fieldset v-if="project.cameraPath">
          <label for="camera-layout">Camera layout</label><select id="camera-layout" v-model="project.edits.camera.layout"><option value="bubble">Bubble over screen</option><option value="fullscreen">Camera only</option><option value="split">Split screen and camera</option></select>
          <label class="check-label"><input v-model="project.edits.camera.enabled" type="checkbox" />Show webcam</label>
          <p v-if="project.edits.camera.layout === 'fullscreen'" class="field-help">Screen video stays hidden in this layout, including camera gaps.</p>
          <p v-else-if="project.edits.camera.layout === 'split'" class="field-help">Camera appears on the right in landscape, or below the screen in portrait. Gaps keep its space empty.</p>
          <template v-if="project.edits.camera.layout === 'bubble'"><label for="camera-shape">Shape</label><select id="camera-shape" v-model="project.edits.camera.shape"><option value="rectangle">Rectangle</option><option value="circle">Circle</option></select></template>
          <template v-if="project.edits.camera.layout !== 'fullscreen'"><label for="camera-size">{{ project.edits.camera.layout === 'split' ? 'Camera share' : 'Width' }} <output>{{ Math.round(project.edits.camera.width * 100) }}%</output></label><input id="camera-size" v-model.number="project.edits.camera.width" type="range" min=".08" max=".6" step=".01" /></template>
          <template v-if="project.edits.camera.layout === 'bubble'"><label for="camera-x">Horizontal position <output>{{ Math.round(project.edits.camera.x * 100) }}%</output></label><input id="camera-x" v-model.number="project.edits.camera.x" type="range" min="0" max="1" step=".01" /><label for="camera-y">Vertical position <output>{{ Math.round(project.edits.camera.y * 100) }}%</output></label><input id="camera-y" v-model.number="project.edits.camera.y" type="range" min="0" max="1" step=".01" /></template>
          <label class="check-label"><input v-model="project.edits.camera.mirror" type="checkbox" />Mirror webcam</label>
        </fieldset>
      </section>
      <section v-if="tab === 'audio'" id="panel-audio" role="tabpanel" aria-labelledby="tab-audio">
        <AudioProperties :project="project" :analyses="audioAnalysis" @analyze="emit('analyzeAudio', $event)" @normalize="emit('normalizeAudio', $event)" @cancel="emit('cancelAnalysis')" />
      </section>
    </div>
    <div class="source-info"><span>{{ project.width }} x {{ project.height }}</span><span>{{ project.fps }} fps</span></div>
  </aside>
</template>
