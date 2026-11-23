<script setup lang="ts">
import { computed, nextTick } from 'vue'
import { audioSignature, MAX_RECORDED_GAIN, type AudioAnalysisState } from '../audio-analysis'
import type { Project, RecordedAudioSource } from '../types'
const props = defineProps<{ project: Project; analyses: Record<string, AudioAnalysisState> }>()
const emit = defineEmits<{
  analyze: [RecordedAudioSource]
  normalize: [RecordedAudioSource]
  cancel: []
}>()
const tracks = computed(() =>
  [
    {
      source: 'microphone' as const,
      label: 'Microphone',
      path: props.project.microphonePath,
      volume: 'microphoneVolume' as const,
      gain: 'microphoneGain' as const,
      input: 'mic-volume',
    },
    {
      source: 'system' as const,
      label: 'Computer audio',
      path: props.project.systemAudioPath,
      volume: 'systemVolume' as const,
      gain: 'systemGain' as const,
      input: 'system-volume',
    },
  ].filter((track) => track.path),
)
const canReduce = computed(() =>
  !!props.project.microphonePath &&
  ((!!props.project.systemAudioPath && !!props.project.edits.audioProcessing.ducking.systemEnabled) ||
    (!!props.project.edits.music.length && props.project.edits.audioProcessing.ducking.enabled)),
)
const decibels = (value: number) =>
  value > 0 ? `${(20 * Math.log10(value)).toFixed(1)} dBFS` : 'Silent'
const current = (source: string) =>
  props.analyses[source]?.signature === audioSignature(props.project.edits, source)
const silent = (source: string) => current(source) && props.analyses[source]?.data?.peak === 0
const gainText = (value: number) =>
  value > MAX_RECORDED_GAIN && Number(value.toFixed(2)) !== value ? String(value) : value.toFixed(2)
function setGain(gain: 'microphoneGain' | 'systemGain', value: number, input: string) {
  const wasLegacy = props.project.edits.audioProcessing[gain] > MAX_RECORDED_GAIN
  props.project.edits.audioProcessing[gain] = value
  if (wasLegacy) void nextTick(() => document.getElementById(`${input}-gain`)?.focus())
}
</script>
<template>
  <h3>Recorded audio</h3>
  <p v-if="!project.microphonePath" class="field-help">
    No microphone recorded. Choose a microphone in recording setup to capture your voice.
  </p>
  <p v-if="!project.systemAudioPath" class="field-help">No computer audio recorded.</p>
  <section
    v-for="track in tracks"
    :key="track.source"
    class="audio-source-properties"
    :aria-label="`${track.label} settings`"
  >
    <label :for="track.input">
      {{ track.label }} volume
      <output>{{ Math.round(project.edits[track.volume] * 100) }}%</output>
    </label>
    <input
      :id="track.input"
      v-model.number="project.edits[track.volume]"
      :aria-label="`${track.label} volume`"
      type="range"
      min="0"
      max="1"
      step=".01"
    />
    <div class="gain-readout">
      <label
        :for="
          project.edits.audioProcessing[track.gain] <= MAX_RECORDED_GAIN
            ? `${track.input}-gain`
            : undefined
        "
      >
        Source gain
        <output>{{ gainText(project.edits.audioProcessing[track.gain]) }}x</output>
      </label>
      <button
        type="button"
        class="small-button"
        :aria-label="`Reset ${track.label.toLowerCase()} gain`"
        :disabled="project.edits.audioProcessing[track.gain] === 1"
        @click="setGain(track.gain, 1, track.input)"
      >
        Reset
      </button>
    </div>
    <template v-if="project.edits.audioProcessing[track.gain] <= MAX_RECORDED_GAIN">
      <input
        :id="`${track.input}-gain`"
        v-model.number="project.edits.audioProcessing[track.gain]"
        type="range"
        min=".01"
        :max="MAX_RECORDED_GAIN"
        step=".01"
        :aria-label="`${track.label} source gain`"
        :aria-valuetext="`${project.edits.audioProcessing[track.gain].toFixed(2)} times`"
      />
      <div class="gain-limits" aria-hidden="true">
        <span>0.01x</span>
        <span>Max {{ MAX_RECORDED_GAIN.toFixed(2) }}x</span>
      </div>
    </template>
    <div v-else class="legacy-gain">
      <p class="field-help">This gain was saved by an earlier version.</p>
      <button
        type="button"
        class="small-button"
        :aria-label="`Set ${track.label.toLowerCase()} gain to ${MAX_RECORDED_GAIN.toFixed(2)}x`"
        @click="setGain(track.gain, MAX_RECORDED_GAIN, track.input)"
      >
        Set to {{ MAX_RECORDED_GAIN.toFixed(2) }}x
      </button>
    </div>
    <template v-if="analyses[track.source]?.data && current(track.source)">
      <dl class="audio-stats">
        <div>
          <dt>Raw peak</dt>
          <dd>{{ decibels(analyses[track.source].data!.peak) }}</dd>
        </div>
        <div>
          <dt>Raw RMS</dt>
          <dd>{{ decibels(analyses[track.source].data!.rms) }}</dd>
        </div>
      </dl>
      <p v-if="silent(track.source)" class="field-help">No signal in the kept video.</p>
    </template>
    <p v-else-if="analyses[track.source]?.data" class="field-help">
      Updating levels for the current cut.
    </p>
    <p v-if="analyses[track.source]?.error" class="audio-analysis-error" role="alert">
      {{ analyses[track.source].error }}
    </p>
    <div class="tool-actions">
      <button
        type="button"
        class="small-button"
        :aria-label="`Normalize ${track.label.toLowerCase()} level`"
        :disabled="silent(track.source)"
        @click="emit('normalize', track.source)"
      >
        Normalize level
      </button>
      <button
        v-if="analyses[track.source]?.state === 'loading'"
        type="button"
        class="small-button"
        @click="emit('cancel')"
      >
        Cancel analysis
      </button>
      <button
        v-else
        type="button"
        class="small-button"
        :aria-label="`Analyze ${track.label.toLowerCase()} level`"
        @click="emit('analyze', track.source)"
      >
        {{ analyses[track.source]?.error ? 'Retry analysis' : 'Analyze' }}
      </button>
    </div>
    <p v-if="analyses[track.source]?.state === 'loading'" class="field-help" role="status">
      Analyzing {{ track.label.toLowerCase() }}:
      {{ Math.round(analyses[track.source].progress * 100) }}%
    </p>
  </section>
  <p v-if="tracks.length" class="field-help">
    Normalize the kept audio toward -18 dBFS RMS, with peaks below -1 dBFS. Volume remains separate.
  </p>
  <div class="section-divider"></div>
  <section class="audio-ducking-properties" aria-labelledby="audio-ducking-heading">
    <h3 id="audio-ducking-heading">Audio ducking</h3>
    <label class="check-label">
      <input
        v-model="project.edits.audioProcessing.ducking.systemEnabled"
        type="checkbox"
        :disabled="!project.microphonePath || !project.systemAudioPath"
      />
      Lower computer audio while microphone speaks
    </label>
    <label class="check-label">
      <input
        v-model="project.edits.audioProcessing.ducking.enabled"
        type="checkbox"
        :disabled="!project.microphonePath || !project.edits.music.length"
      />
      Lower music while microphone speaks
    </label>
    <p v-if="!project.microphonePath" class="field-help">A recorded microphone track is required.</p>
    <p v-else-if="!project.edits.music.length" class="field-help">Import music to enable music ducking.</p>
    <label for="ducking-amount">
      Reduction
      <output>{{ project.edits.audioProcessing.ducking.amountDb }} dB</output>
    </label>
    <input
      id="ducking-amount"
      v-model.number="project.edits.audioProcessing.ducking.amountDb"
      type="range"
      min="0"
      max="24"
      step="1"
      :disabled="!canReduce"
    />
    <p class="field-help">
      Microphone volume and gain control the speech detector. Reduction applies to the selected tracks.
    </p>
  </section>
</template>
<style scoped>
.audio-source-properties + .audio-source-properties {
  margin-top: 20px;
  padding-top: 12px;
  border-top: 1px solid var(--line);
}
.gain-readout,
.audio-stats > div {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 8px;
}
.gain-readout,
.audio-stats {
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.gain-readout {
  margin-top: 14px;
}
.gain-readout label {
  flex: 1;
  min-width: 0;
  margin: 0;
}
.gain-readout output {
  min-width: 0;
  overflow-wrap: anywhere;
  text-align: right;
}
.gain-limits {
  display: flex;
  justify-content: space-between;
  color: var(--muted);
  font-size: 12px;
  font-variant-numeric: tabular-nums;
}
.legacy-gain .field-help {
  margin: 8px 0;
}
:root[data-theme='light'] .audio-source-properties input[type='range'],
:root[data-theme='light'] .audio-ducking-properties input {
  accent-color: var(--focus);
}
.audio-stats {
  margin: 10px 0;
}
.audio-stats > div {
  margin: 4px 0;
}
.audio-stats dt {
  color: var(--muted);
}
.audio-stats dd {
  margin: 0;
}
.audio-analysis-error {
  color: var(--danger);
  font-size: 12px;
  overflow-wrap: anywhere;
}
</style>
