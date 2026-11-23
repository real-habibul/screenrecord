<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { PhysicalPosition } from '@tauri-apps/api/dpi'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { command, desktop, errorText } from '../api'
import { timeLabel } from '../editing'
import type { RecordingCameraFrame, RecordingError, RecordingState } from '../types'

const state = ref<RecordingState | null>(null)
const busy = ref(''); const error = ref(''); const connectionError = ref(''); const cameraError = ref(''); const image = ref('')
const finalization = ref<number | null>(null)
const phase = computed(() => busy.value === 'Finalizing' || state.value?.phase === 'finalizing' ? `Finalizing${finalization.value === null ? '' : ` ${Math.round(finalization.value * 100)}%`}` : busy.value || (state.value?.phase === 'paused' ? 'Paused' : state.value?.active ? 'Recording' : state.value ? 'Recording stopped' : 'Connecting...'))
const canControl = computed(() => desktop && !busy.value && state.value?.phase !== 'finalizing' && state.value?.active === true && !!state.value.projectId)
let timer: ReturnType<typeof setTimeout> | undefined
let polling = false; let disposed = false; let epoch = 0
let frameGeneration = -1; let sequence = -1
let positionChange = Promise.resolve()
let unlisteners: UnlistenFn[] = []

function resetCamera() { image.value = ''; cameraError.value = ''; frameGeneration = -1; sequence = -1 }
function applyState(value: RecordingState) {
  if (value.projectId !== state.value?.projectId) { resetCamera(); finalization.value = null; if (value.active) error.value = '' }
  state.value = value
  if (value.phase !== 'recording' || !value.cameraAvailable) image.value = ''
}
async function poll() {
  if (polling || disposed || !desktop) return
  polling = true; const request = epoch
  try {
    if (busy.value) return
    const value = await command<RecordingState>('get_recording_state')
    if (disposed || request !== epoch) return
    applyState(value); connectionError.value = ''
    if (value.phase !== 'recording' || !value.cameraAvailable || !value.projectId) return
    try {
      const frame = await command<RecordingCameraFrame>('get_recording_camera_frame', { projectId: value.projectId })
      if (disposed || request !== epoch || value.projectId !== state.value?.projectId || state.value.phase !== 'recording') return
      if (frame.generation < frameGeneration) return
      if (frame.generation !== frameGeneration) { resetCamera(); frameGeneration = frame.generation }
      if (frame.error) { image.value = ''; cameraError.value = frame.error; return }
      if (!frame.active) { image.value = ''; return }
      if (frame.sequence > sequence && frame.data) {
        if (frame.mimeType !== 'image/jpeg' || frame.data.length > 700_000) throw new Error('The recording camera returned an invalid preview frame.')
        sequence = frame.sequence; image.value = `data:image/jpeg;base64,${frame.data}`; cameraError.value = ''
      }
    } catch (failure) { if (!disposed && request === epoch) { image.value = ''; cameraError.value = errorText(failure) } }
  } catch (failure) { if (!disposed && request === epoch) connectionError.value = errorText(failure) }
  finally { polling = false; if (!disposed) timer = setTimeout(() => void poll(), 250) }
}
async function action(kind: 'pause' | 'resume' | 'stop') {
  if (!canControl.value) return
  const projectId = state.value?.projectId ?? null
  epoch++; busy.value = kind === 'pause' ? 'Pausing...' : kind === 'resume' ? 'Resuming...' : 'Finalizing'; error.value = ''; image.value = ''
  try {
    if (kind === 'stop') { await command('stop_recording', { projectId }); applyState({ active: false, paused: false, phase: 'idle', cameraAvailable: false, elapsed: 0, projectId: null }) }
    else applyState(await command<RecordingState>(kind === 'pause' ? 'pause_recording' : 'resume_recording', { projectId }))
  } catch (failure) { error.value = errorText(failure) }
  finally { busy.value = ''; clearTimeout(timer); void poll() }
}
async function showApp() { try { await command('restore_main_window') } catch (failure) { error.value = errorText(failure) } }
async function drag(event: PointerEvent) { if (event.button !== 0 || !desktop) return; try { await getCurrentWindow().startDragging() } catch (failure) { error.value = errorText(failure) } }
function move(event: KeyboardEvent) {
  if (!desktop || !event.key.startsWith('Arrow')) return
  const delta: Record<string, [number, number]> = { ArrowLeft: [-1, 0], ArrowRight: [1, 0], ArrowUp: [0, -1], ArrowDown: [0, 1] }
  const step = delta[event.key]; if (!step) return
  event.preventDefault(); const amount = event.shiftKey ? 10 : 1
  positionChange = positionChange.then(async () => { if (disposed) return; const window = getCurrentWindow(); const position = await window.outerPosition(); await window.setPosition(new PhysicalPosition(position.x + step[0] * amount, position.y + step[1] * amount)) }).catch(failure => { error.value = errorText(failure) })
}
function cameraImageError() { image.value = ''; cameraError.value = 'The camera preview frame could not be displayed.' }
function syncTheme() { document.documentElement.dataset.theme = localStorage.getItem('screenrecord-theme') ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light') }
onMounted(async () => {
  document.title = 'Screenrecord recording controls'; syncTheme(); window.addEventListener('storage', syncTheme)
  if (desktop) try {
    const stops = await Promise.all([listen<RecordingError>('recording-error', event => { if (typeof event.payload === 'string') error.value = event.payload; else if (event.payload.projectId === state.value?.projectId) error.value = event.payload.error }), listen<{ projectId: string; progress: number }>('recording-finalization-progress', event => { if (event.payload.projectId === state.value?.projectId) finalization.value = Math.max(0, Math.min(1, event.payload.progress)) })])
    if (disposed) stops.forEach(stop => stop()); else unlisteners = stops
  } catch (failure) { error.value = errorText(failure) }
  void poll()
})
onBeforeUnmount(() => { disposed = true; epoch++; clearTimeout(timer); unlisteners.forEach(stop => stop()); window.removeEventListener('storage', syncTheme); resetCamera() })
</script>

<template>
  <main class="floating-recorder" aria-label="Recording controls">
    <header class="floating-heading"><button type="button" class="move-controls" aria-label="Move recording controls" aria-describedby="move-help" :disabled="!desktop" @pointerdown="drag" @keydown="move"><svg width="18" height="18" viewBox="0 0 18 18" fill="none" aria-hidden="true"><path d="M4 6h10M4 9h10M4 12h10" stroke="currentColor" stroke-width="1.5" /></svg></button><div><strong role="status">{{ desktop ? phase : 'Desktop app required' }}</strong><time v-if="state" aria-label="Recorded duration">{{ timeLabel(state.elapsed) }}</time></div><button type="button" class="small-button" :disabled="!desktop" @click="showApp">Show app</button></header>
    <p id="move-help" class="sr-only">Drag to move. With this control focused, use arrow keys. Hold Shift for larger steps.</p>
    <div v-if="state?.cameraAvailable" class="floating-camera"><img v-if="image" :src="image" width="480" height="270" alt="Camera view from the active recording" @error="cameraImageError" /><p v-else>{{ cameraError || (state.paused ? 'Camera paused' : state.phase === 'finalizing' || busy === 'Finalizing' ? 'Finishing recording' : 'Waiting for camera frame...') }}</p></div>
    <p v-if="error || connectionError" class="floating-error" role="alert">{{ error || connectionError }}</p>
    <div class="floating-actions"><button type="button" :disabled="!canControl || !state?.active" @click="action(state?.paused ? 'resume' : 'pause')">{{ state?.paused ? 'Resume' : 'Pause' }}</button><button type="button" class="stop-button" :disabled="!canControl" @click="action('stop')">Stop recording</button></div>
  </main>
</template>

<style scoped>
.floating-recorder { height: 100dvh; padding: 8px; display: flex; flex-direction: column; gap: 8px; overflow: auto; background: var(--panel); }
.floating-heading { display: flex; align-items: center; gap: 8px; flex-shrink: 0; }
.floating-heading>div { display: flex; gap: 4px; flex: 1; min-width: 0; flex-direction: column; }
.floating-heading strong { font-size: 12px; }.floating-heading time { font-size: 15px; font-weight: 600; font-variant-numeric: tabular-nums; }
.move-controls { width: 30px; padding: 6px; min-height: 32px; cursor: move; }
.floating-camera { aspect-ratio: 16/9; width: 100%; min-height: 70px; overflow: hidden; border: 1px solid var(--line); border-radius: 6px; background: var(--preview); display: grid; place-items: center; }
.floating-camera img { width: 100%; height: 100%; object-fit: contain; }
.floating-camera p { padding: 12px; color: var(--muted); font-size: 12px; text-align: center; line-height: 1.5; overflow-wrap: anywhere; }
.floating-error { color: var(--danger); line-height: 1.4; font-size: 12px; overflow-wrap: anywhere; }
.floating-actions { display: flex; gap: 8px; position: sticky; bottom: 0; background: var(--panel); margin-top: auto; padding-top: 2px; flex-shrink: 0; }.floating-actions>button { flex: 1; }
</style>
