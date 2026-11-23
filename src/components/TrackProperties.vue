<script setup lang="ts">
import { computed } from 'vue'
import { timeLabel } from '../editing'
import { updateMusic } from '../tracks'
import type { MusicClip, Project, Redaction, TrackKind, TrackSelection } from '../types'
import type { AudioAnalysisState } from '../audio-analysis'
import SubtitleProperties from './SubtitleProperties.vue'
import MaskProperties from './MaskProperties.vue'

const props = defineProps<{ project: Project; analyses: Record<string, AudioAnalysisState>; kind: TrackKind; selection: TrackSelection | null; time: number; drawing: boolean; correcting: boolean }>()
const emit = defineEmits<{ select: [TrackSelection | null]; importMusic: []; transcribe: []; exportSrt: []; importSrt: []; draw: []; error: [string]; analyze: [string]; cancelAnalysis: []; audioTools: []; updateMask: [Redaction]; seek: [number]; trackMask: [{ regionId: string; start?: number }]; correctMask: [string]; stopCorrection: [] }>()
const music = computed(() => props.project.edits.music.find(item => item.id === props.selection?.id))
function number(event: Event) { return Number((event.target as HTMLInputElement).value) }
function musicChange(field: keyof MusicClip, event: Event) { if (music.value) updateMusic(music.value, field, number(event), props.project.duration) }
function deleteSelected() {
  if (props.kind === 'music') props.project.edits.music = props.project.edits.music.filter(item => item.id !== props.selection?.id)
  if (props.kind === 'subtitles') props.project.edits.subtitles = props.project.edits.subtitles.filter(item => item.id !== props.selection?.id)
  if (props.kind === 'redactions') props.project.edits.redactions = props.project.edits.redactions.filter(item => item.id !== props.selection?.id)
  emit('select', null)
}
</script>

<template>
  <section v-if="kind === 'music'">
    <div class="section-heading"><h3>Music</h3><button type="button" class="small-button" :disabled="project.edits.music.length >= 32" @click="emit('importMusic')">Import music</button></div>
    <p v-if="!project.edits.music.length" class="field-help">Import an audio file, then choose when it plays in the recording.</p>
    <template v-else>
      <label for="music-selection">Music clip</label><select id="music-selection" :value="music?.id ?? ''" @change="emit('select', { kind, id: ($event.target as HTMLSelectElement).value })"><option disabled value="">Choose a clip</option><option v-for="item in project.edits.music" :key="item.id" :value="item.id">{{ item.name }} ({{ timeLabel(item.start, true) }})</option></select>
      <template v-if="music">
        <p v-if="analyses[`music:${music.id}`]?.state === 'loading'" class="field-help" role="status">Analyzing music: {{ Math.round(analyses[`music:${music.id}`].progress * 100) }}% <button type="button" class="text-button" @click="emit('cancelAnalysis')">Cancel analysis</button></p>
        <p v-else-if="!analyses[`music:${music.id}`]?.data" class="field-help">{{ analyses[`music:${music.id}`]?.error ?? 'Music waveform is not available yet.' }} <button type="button" class="text-button" @click="emit('analyze', `music:${music!.id}`)">Analyze music</button></p>
        <label for="music-name">Clip name</label><input id="music-name" v-model="music.name" maxlength="120" />
        <div class="field-grid"><div><label for="music-start">Timeline start (s)</label><input id="music-start" type="number" step=".01" min="0" :value="music.start.toFixed(3)" @change="musicChange('start', $event)" /></div><div><label for="music-source">Audio in point (s)</label><input id="music-source" type="number" step=".01" min="0" :value="music.sourceStart.toFixed(3)" @change="musicChange('sourceStart', $event)" /></div></div>
        <label for="music-duration">Duration (s)</label><input id="music-duration" type="number" step=".01" min=".1" :value="music.duration.toFixed(3)" @change="musicChange('duration', $event)" />
        <label for="music-volume">Music volume <output>{{ Math.round(music.volume * 100) }}%</output></label><input id="music-volume" type="range" min="0" max="2" step=".01" :value="music.volume" @input="musicChange('volume', $event)" />
        <div class="field-grid"><div><label for="music-fade-in">Fade in (s)</label><input id="music-fade-in" type="number" min="0" step=".1" :value="music.fadeIn" @change="musicChange('fadeIn', $event)" /></div><div><label for="music-fade-out">Fade out (s)</label><input id="music-fade-out" type="number" min="0" step=".1" :value="music.fadeOut" @change="musicChange('fadeOut', $event)" /></div></div>
        <button type="button" class="danger-text full-width" @click="deleteSelected">Delete music clip</button>
        <button type="button" class="full-width" @click="emit('audioTools')">Music ducking settings</button>
      </template>
    </template>
  </section>
  <SubtitleProperties v-else-if="kind === 'subtitles'" :project="project" :selection="selection" :time="time" @select="emit('select', $event)" @transcribe="emit('transcribe')" @import-srt="emit('importSrt')" @export-srt="emit('exportSrt')" @error="emit('error', $event)" />
  <MaskProperties v-else :project="project" :selection="selection" :time="time" :drawing="drawing" :correcting="correcting" @select="emit('select', $event)" @draw="emit('draw')" @error="emit('error', $event)" @update="emit('updateMask', $event)" @seek="emit('seek', $event)" @track="emit('trackMask', $event)" @correct="emit('correctMask', $event)" @stop-correction="emit('stopCorrection')" />
</template>
