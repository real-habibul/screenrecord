<script setup lang="ts">
import { onBeforeUnmount, ref, watch } from 'vue'
import { command, errorText } from '../api'

const props = withDefaults(defineProps<{ cameraId: string | null; disabled: boolean; active?: boolean }>(), { active: true })
const emit = defineEmits<{ previewing: [value: boolean]; error: [message: string] }>()
type Frame = { sequence: number; mimeType: string; data: string | null; error: string | null; active: boolean }
const state = ref<'off' | 'loading' | 'live' | 'stopping' | 'stop-error' | 'error'>('off')
const message = ref('')
const image = ref('')
let sessionId: string | null = null
let sequence = 0
let timer: ReturnType<typeof setTimeout> | undefined
let starting: Promise<void> | null = null
let stopping: Promise<void> | null = null
let disposed = false

function clearFrame() {
  clearTimeout(timer)
  timer = undefined
  image.value = ''
  sequence = 0
  emit('previewing', false)
}

async function stop(): Promise<void> {
  if (stopping) return stopping
  const id = sessionId
  sessionId = null
  clearFrame()
  if (!id) return
  const pendingStart = starting
  state.value = 'stopping'
  const work = (async () => {
    let failure: unknown
    try { await command('stop_camera_preview', { sessionId: id }) } catch (error) { failure = error }
    if (pendingStart) {
      await pendingStart.catch(() => undefined)
      try { await command('stop_camera_preview', { sessionId: id }); failure = undefined } catch (error) { failure = error }
    }
    if (failure) throw failure
  })()
  stopping = work
  try {
    await work
    state.value = 'off'
  } catch (error) {
    sessionId = id
    message.value = errorText(error)
    state.value = 'stop-error'
    emit('error', message.value)
    throw error
  } finally {
    stopping = null
  }
}

async function fail(id: string, error: unknown) {
  if (sessionId !== id || disposed) return
  const text = errorText(error)
  await stop().catch(() => undefined)
  if (!sessionId && !disposed) {
    message.value = text
    state.value = 'error'
    emit('error', text)
  }
}

async function poll(id: string) {
  if (sessionId !== id || disposed) return
  try {
    const frame = await command<Frame>('get_camera_preview_frame', { sessionId: id })
    if (sessionId !== id || disposed) return
    if (frame.error) throw new Error(frame.error)
    if (!frame.active) throw new Error('Camera preview ended. Check the camera connection and try again.')
    if (frame.sequence > sequence && frame.data) {
      if (frame.mimeType !== 'image/jpeg' || frame.data.length > 700_000) throw new Error('The camera returned an invalid preview frame.')
      image.value = `data:image/jpeg;base64,${frame.data}`
      sequence = frame.sequence
      state.value = 'live'
      emit('previewing', true)
    }
    timer = setTimeout(() => void poll(id), 250)
  } catch (error) { await fail(id, error) }
}

async function start() {
  if (!props.cameraId || props.disabled || !props.active || disposed || sessionId || stopping) return
  const id = crypto.randomUUID()
  sessionId = id
  state.value = 'loading'
  message.value = ''
  const pending = command<void>('start_camera_preview', { cameraId: props.cameraId, sessionId: id })
  starting = pending
  try {
    await pending
    if (sessionId === id && !disposed) void poll(id)
  } catch (error) { await fail(id, error) }
  finally { if (starting === pending) starting = null }
}

watch(() => [props.cameraId, props.disabled, props.active], () => { void stop().catch(() => undefined) })
onBeforeUnmount(() => { disposed = true; void stop().catch(() => undefined) })
defineExpose({ stop })
</script>

<template>
  <section class="camera-preview" aria-label="Camera preview" :aria-busy="state === 'loading' || state === 'stopping'">
    <div class="camera-preview-image">
      <img v-if="image" :src="image" width="480" height="270" alt="Live preview of the selected camera" @error="sessionId && fail(sessionId, new Error('The camera preview frame could not be displayed.'))" />
      <p v-else>{{ state === 'stop-error' ? 'Camera still needs cleanup' : !cameraId ? 'Camera is off' : state === 'loading' ? 'Opening camera...' : state === 'stopping' ? 'Releasing camera...' : 'Camera preview is off' }}</p>
    </div>
    <div class="camera-preview-controls">
      <span role="status" aria-live="polite">{{ state === 'live' ? 'Preview only' : state === 'loading' ? 'Waiting for camera' : state === 'stopping' ? 'Stopping preview' : 'Camera preview' }}</span>
      <button v-if="['live', 'loading', 'stopping', 'stop-error'].includes(state)" type="button" class="small-button" :disabled="state === 'stopping'" @click="stop().catch(() => undefined)">{{ state === 'stop-error' ? 'Retry stop preview' : 'Stop preview' }}</button>
      <button v-else type="button" class="small-button" :disabled="!cameraId || disabled || !active" @click="start">{{ state === 'error' ? 'Retry preview' : 'Enable preview' }}</button>
    </div>
    <p v-if="state === 'error' || state === 'stop-error'" class="camera-preview-error" role="alert">{{ message }}</p>
  </section>
</template>

<style scoped>
.camera-preview { min-width: 0; margin-top: 14px; }
.camera-preview-image { aspect-ratio: 16 / 9; display: grid; place-items: center; background: var(--bg); border: 1px solid var(--line); border-radius: 4px; overflow: hidden; }
.camera-preview-image img { width: 100%; height: 100%; object-fit: contain; }
.camera-preview-image p { color: var(--muted); padding: 12px; text-align: center; }
.camera-preview-controls { display: flex; flex-wrap: wrap; align-items: center; justify-content: space-between; gap: 8px; margin-top: 8px; }
.camera-preview-controls span { color: var(--muted); font-size: 12px; }
.camera-preview-error { color: var(--danger); margin-top: 8px; overflow-wrap: anywhere; line-height: 1.5; }
</style>
