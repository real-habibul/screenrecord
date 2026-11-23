<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { command, errorText } from '../api'
import type { LocalTranscriptionInstallProgress, LocalTranscriptionStatus } from '../types'

const emit = defineEmits<{ ready: [boolean]; busy: [boolean] }>()
const status = ref<LocalTranscriptionStatus | null>(null); const progress = ref<LocalTranscriptionInstallProgress | null>(null)
const loading = ref(false); const starting = ref(false); const cancelling = ref(false); const error = ref('')
const running = computed(() => progress.value?.state === 'running')
let target: string | null = null; let disposed = false; let revision = 0; const initializing = ref(true)
let initialization: Promise<void> | null = null
let timer: ReturnType<typeof setTimeout> | undefined; let unlisten: UnlistenFn | undefined
let startPromise: Promise<void> | null = null; let stopPromise: Promise<void> | null = null; let progressRequest: Promise<LocalTranscriptionInstallProgress | null> | null = null
function publish() { if (!disposed) { emit('ready', !!status.value?.ready && !initializing.value && !loading.value && !running.value && !starting.value && !cancelling.value); emit('busy', running.value || starting.value || cancelling.value) } }
function readProgress() { return progressRequest ??= command<LocalTranscriptionInstallProgress | null>('get_local_transcription_install_progress').finally(() => { progressRequest = null }) }
function accept(value: LocalTranscriptionInstallProgress | null) {
  if (disposed || !value || value.jobId !== target) return
  if (progress.value?.jobId === value.jobId && progress.value.state !== 'running') return
  progress.value = value
  if (value.status) status.value = value.status
  if (value.state === 'failed') error.value = value.error ?? 'Local subtitle setup failed.'
  publish()
}
async function refreshStatus() {
  const request = ++revision; loading.value = true; error.value = ''; publish()
  try { const value = await command<LocalTranscriptionStatus>('get_local_transcription_status'); if (!disposed && request === revision) { status.value = value; error.value = value.error ?? (progress.value?.state === 'failed' ? progress.value.error : null) ?? '' } }
  catch (failure) { if (!disposed && request === revision) error.value = errorText(failure) }
  finally { if (!disposed && request === revision) { loading.value = false; publish() } }
}
async function poll() {
  clearTimeout(timer)
  if (disposed || !target || starting.value || cancelling.value) return
  try {
    const value = await readProgress(); accept(value)
    if (value?.jobId === target && value.state !== 'running') { if (!value.status) await refreshStatus(); return }
  } catch (failure) { if (!disposed) error.value = errorText(failure) }
  if (!disposed && running.value) timer = setTimeout(() => void poll(), 250)
}
async function install() {
  if (initializing.value || running.value || starting.value || cancelling.value || loading.value) return
  const jobId = crypto.randomUUID(); target = jobId; starting.value = true; error.value = ''; progress.value = { jobId, state: 'running', progress: 0, error: null, status: null }; publish()
  const task = command<void>('start_local_transcription_install', { jobId }); startPromise = task
  try { await task } catch (failure) { progress.value = { jobId, state: 'failed', progress: 0, error: errorText(failure), status: null }; if (!disposed) error.value = errorText(failure) }
  finally { if (startPromise === task) startPromise = null; starting.value = false; publish(); if (!disposed) void poll() }
}
async function cancelAndWait(): Promise<void> {
  if (stopPromise) return stopPromise
  const task = (async () => {
    cancelling.value = true; clearTimeout(timer); publish()
    try {
      await initialization
      await startPromise?.catch(() => undefined)
      const jobId = target
      if (!jobId || progress.value?.state !== 'running') return
      await command('cancel_local_transcription_install', { jobId })
      for (;;) {
        const value = await readProgress()
        if (value && value.jobId !== jobId) throw new Error('Local setup session changed. Refresh its status before continuing.')
        accept(value)
        if (value?.state && value.state !== 'running') { if (!disposed && !value.status) await refreshStatus(); return }
        await new Promise(resolve => setTimeout(resolve, 100))
      }
    } catch (failure) { if (!disposed) error.value = errorText(failure); throw failure }
    finally { cancelling.value = false; publish(); if (!disposed && running.value) void poll() }
  })()
  stopPromise = task
  try { await task } finally { if (stopPromise === task) stopPromise = null }
}
async function cancel() { try { await cancelAndWait() } catch { /* Keep the setup error and Cancel action available. */ } }
onMounted(() => { initialization = (async () => {
  try { const stop = await listen<LocalTranscriptionInstallProgress>('local-transcription-install-progress', event => accept(event.payload)); if (disposed) stop(); else unlisten = stop } catch (failure) { error.value = errorText(failure) }
  await refreshStatus()
  try { const value = await readProgress(); if (!disposed && value?.state === 'running') { target = value.jobId; accept(value); void poll() } } catch (failure) { if (!disposed) error.value = errorText(failure) }
  initializing.value = false; publish()
})() })
onBeforeUnmount(() => { disposed = true; revision++; clearTimeout(timer); unlisten?.(); void cancelAndWait().catch(() => undefined) })
defineExpose({ cancelAndWait, refreshStatus })
</script>

<template>
  <section class="local-subtitle-setup" aria-label="Local subtitle setup">
    <p class="field-help">Recognition runs on this computer. Audio is not uploaded.</p>
    <p v-if="loading" role="status">Checking local subtitle setup...</p>
    <template v-if="status"><p><strong>{{ status.ready ? 'Local transcription ready' : 'Local transcription needs setup' }}</strong></p><p class="field-help">{{ status.modelName }} / {{ status.runtimeVersion }}. {{ status.runtimeReady ? 'Runtime installed.' : 'Runtime missing.' }} {{ status.modelReady ? 'Model installed.' : 'Model missing.' }}</p><p v-if="!status.ready" class="field-help">Download {{ Math.ceil(status.downloadBytes / 1048576) }} MiB from the official runtime and model sources. Setup does not generate subtitles.</p></template>
    <template v-if="running || starting || cancelling"><p role="status">{{ cancelling ? 'Stopping local setup...' : starting ? 'Starting local setup...' : `Downloading local setup: ${Math.round((progress?.progress ?? 0) * 100)}%` }}</p><progress :value="progress?.progress ?? 0" max="1" aria-label="Local setup progress"></progress><button type="button" :disabled="starting || cancelling" @click="cancel">Cancel download</button></template>
    <template v-else><button v-if="status && !status.ready" type="button" :disabled="loading || initializing" @click="install">{{ progress?.state === 'failed' ? 'Retry local setup' : 'Download local setup' }}</button><button v-if="error || !status" type="button" :disabled="loading" @click="refreshStatus">Retry setup status</button><p v-if="progress?.state === 'cancelled'" role="status">Local setup cancelled.</p></template>
    <p v-if="error" class="dialog-error" role="alert">{{ error }}</p>
  </section>
</template>

<style scoped>
.local-subtitle-setup { display: grid; gap: 10px; padding-block: 14px; margin-block: 12px; border-block: 1px solid var(--line); }.local-subtitle-setup p { margin: 0; }.local-subtitle-setup progress { width: 100%; }
</style>
