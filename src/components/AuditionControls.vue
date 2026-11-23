<script setup lang="ts">
import { auditionFromPlayhead, fitAuditionRange, type AudioMonitor, type AuditionRange } from '../audition'

const props = defineProps<{
  monitor: AudioMonitor
  range: AuditionRange
  loop: boolean
  duration: number
  time: number
  microphone: boolean
  computer: boolean
  disabled?: boolean
}>()
const emit = defineEmits<{ monitor: [AudioMonitor]; range: [AuditionRange]; loop: [boolean] }>()

function setBoundary(field: 'start' | 'end', event: Event) {
  const input = event.target as HTMLInputElement
  const value = input.valueAsNumber
  if (Number.isFinite(value)) {
    const minimum = Math.min(0.1, props.duration)
    const next = field === 'start' ? Math.min(value, props.range.end - minimum) : Math.max(value, props.range.start + minimum)
    const range = fitAuditionRange({ ...props.range, [field]: next }, props.duration)
    input.value = String(Number(range[field].toFixed(3)))
    emit('range', range)
  } else {
    input.value = String(Number(props.range[field].toFixed(3)))
  }
}
</script>

<template>
  <fieldset class="audition-controls" :disabled="disabled">
    <legend class="sr-only">Preview audio and loop</legend>
    <label>Preview audio
      <select aria-label="Preview audio" :value="monitor" @change="emit('monitor', ($event.target as HTMLSelectElement).value as AudioMonitor)">
        <option value="mix">Full mix</option>
        <option value="microphone" :disabled="!microphone">Microphone only</option>
        <option value="system" :disabled="!computer">Computer only</option>
      </select>
    </label>
    <label class="audition-loop"><input type="checkbox" :checked="loop" @change="emit('loop', ($event.target as HTMLInputElement).checked)" />Loop selection</label>
    <label>In <input type="number" aria-label="Loop start (seconds)" min="0" :max="range.end" step="any" :value="Number(range.start.toFixed(3))" @change="setBoundary('start', $event)" /></label>
    <label>Out <input type="number" aria-label="Loop end (seconds)" :min="range.start" :max="duration" step="any" :value="Number(range.end.toFixed(3))" @change="setBoundary('end', $event)" /></label>
    <button type="button" class="small-button" @click="emit('range', auditionFromPlayhead(time, duration, range.end - range.start))">From playhead</button>
  </fieldset>
</template>

<style scoped>
.audition-controls { display: flex; flex-wrap: wrap; align-items: center; gap: 6px 12px; min-width: 0; margin: 0; padding: 5px 12px; border: 0; }
.audition-controls label { display: inline-flex; align-items: center; gap: 5px; margin: 0; font-size: 12px; }
.audition-controls select, .audition-controls input[type=number] { width: auto; min-height: 28px; margin: 0; padding: 2px 5px; }
.audition-controls input[type=number] { width: 6.3em; font-variant-numeric: tabular-nums; }
.audition-loop input { margin: 0; }
</style>
