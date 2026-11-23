<script setup lang="ts">
import { computed, onBeforeUnmount, ref, watch } from 'vue'
import { command, errorText } from '../api'

const props = defineProps<{ microphoneId: string | null; disabled: boolean; active: boolean }>()
const emit = defineEmits<{ error: [string] }>()
type Reading = { sequence: number; peak: number; rms: number; active: boolean; ready: boolean; error: string | null }
const state = ref<'off' | 'starting' | 'live' | 'stopping' | 'stop-error' | 'ended' | 'error'>('off')
const peak = ref(0); const rms = ref(0); const message = ref('')
const db = (value: number) => value > 0 ? Math.max(-60, 20 * Math.log10(value)) : -60
const peakDb = computed(() => db(peak.value))
const rmsDb = computed(() => db(rms.value))
const level = computed(() => (peakDb.value + 60) / 60 * 100)
let sessionId: string | null = null
let starting: Promise<void> | null = null
let stopping: Promise<void> | null = null
let timer: ReturnType<typeof setTimeout> | undefined
let disposed = false
let sequence = -1

async function stop(): Promise<void> {
  if (stopping) return stopping
  clearTimeout(timer); const id = sessionId; sessionId = null; peak.value = 0; rms.value = 0
  if (!id) return
  state.value = 'stopping'; const pending = starting
  const work = (async () => {
    let failure: unknown
    try { await command('stop_microphone_meter', { sessionId: id }) } catch (error) { failure = error }
    if (pending) { await pending.catch(() => undefined); try { await command('stop_microphone_meter', { sessionId: id }); failure = undefined } catch (error) { failure = error } }
    if (failure) throw failure
  })()
  stopping = work
  try { await work; state.value = 'off' }
  catch (error) { sessionId = id; message.value = errorText(error); state.value = 'stop-error'; emit('error', message.value); throw error }
  finally { stopping = null }
}
async function fail(id: string, error: unknown) {
  if (id !== sessionId || disposed) return
  const text = errorText(error); await stop().catch(() => undefined)
  if (!sessionId && !disposed) { state.value = 'error'; message.value = text; emit('error', text) }
}
async function poll(id: string) {
  if (id !== sessionId || disposed) return
  try {
    const value = await command<Reading>('get_microphone_meter', { sessionId: id })
    if (id !== sessionId || disposed) return
    if (value.error) throw new Error(value.error)
    if (!value.active) { await stop(); state.value = 'ended'; return }
    if (value.ready && value.sequence > sequence) {
      if (![value.peak, value.rms].every(Number.isFinite)) throw new Error('The microphone returned an invalid level.')
      sequence = value.sequence; peak.value = Math.max(0, Math.min(1, value.peak)); rms.value = Math.max(0, Math.min(1, value.rms)); state.value = 'live'
    }
    timer = setTimeout(() => void poll(id), 100)
  } catch (error) { await fail(id, error) }
}
async function start() {
  if (!props.microphoneId || props.disabled || !props.active || disposed || sessionId || stopping) return
  const id = crypto.randomUUID(); sessionId = id; sequence = -1; state.value = 'starting'; message.value = ''
  const pending = command<void>('start_microphone_meter', { microphoneId: props.microphoneId, sessionId: id }); starting = pending
  try { await pending; if (sessionId === id && !disposed) void poll(id) }
  catch (error) { await fail(id, error) }
  finally { if (starting === pending) starting = null }
}
watch(() => [props.microphoneId, props.disabled, props.active], () => { void stop().catch(() => undefined) })
onBeforeUnmount(() => { disposed = true; void stop().catch(() => undefined) })
defineExpose({ stop })
</script>

<template>
  <section class="microphone-meter" aria-label="Microphone level test">
    <div class="meter-heading"><span role="status">{{ state === 'starting' ? 'Opening microphone...' : state === 'stopping' ? 'Releasing microphone...' : state === 'stop-error' ? 'Microphone still needs cleanup' : state === 'live' ? 'Microphone test' : state === 'ended' ? '30-second test ended' : 'Microphone test is off' }}</span><button v-if="['starting', 'live', 'stopping', 'stop-error'].includes(state)" type="button" class="small-button" :disabled="state === 'stopping'" @click="stop().catch(() => undefined)">{{ state === 'stop-error' ? 'Retry stop test' : 'Stop test' }}</button><button v-else type="button" class="small-button" :disabled="!microphoneId || disabled || !active" @click="start">{{ state === 'error' ? 'Retry microphone test' : 'Test microphone' }}</button></div>
    <div class="meter-bar" role="meter" aria-label="Microphone peak level" aria-valuemin="-60" aria-valuemax="0" :aria-valuenow="peakDb" :aria-valuetext="state === 'live' ? `${peakDb.toFixed(1)} dBFS` : 'Test stopped'"><span :style="{ width: `${level}%` }" :class="{ clipping: peakDb > -1 }"></span></div>
    <div v-if="state === 'live'" class="meter-readout"><span>Peak {{ peakDb.toFixed(1) }} dBFS</span><span>Average {{ rmsDb.toFixed(1) }} dBFS</span><strong v-if="peakDb > -1">Clipping risk</strong></div>
    <p v-if="state === 'error' || state === 'stop-error'" class="meter-error" role="alert">{{ message }}</p><p v-else class="field-help">Test for up to 30 seconds. Audio is not saved or played back.</p>
  </section>
</template>

<style scoped>
.microphone-meter { margin-top: 12px; }
.meter-heading,.meter-readout { display: flex; align-items: center; justify-content: space-between; flex-wrap: wrap; gap: 8px; }
.meter-heading { font-size: 12px; color: var(--muted); }
.meter-bar { height: 10px; margin-top: 10px; background: var(--bg); border: 1px solid var(--line); border-radius: 3px; overflow: hidden; }
.meter-bar span { display: block; height: 100%; background: var(--accent); }
.meter-bar .clipping { background: var(--danger); }
.meter-readout { margin-top: 6px; font-size: 12px; font-variant-numeric: tabular-nums; }
.meter-readout strong,.meter-error { color: var(--danger); }
.meter-error { line-height: 1.5; overflow-wrap: anywhere; }
</style>
