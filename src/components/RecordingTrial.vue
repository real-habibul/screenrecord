<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { command, errorText, mediaUrl } from '../api'
import type { RecordingOptions, RecordingTestResult } from '../types'

const props = defineProps<{
  options: RecordingOptions
  disabled: boolean
  active: boolean
  prepare: () => Promise<void>
}>()
const emit = defineEmits<{ busy: [value: boolean] }>()
const phase = ref<
  'idle' | 'preparing' | 'recording' | 'finalizing' | 'ready' | 'stopping' | 'error'
>('idle')
const result = ref<RecordingTestResult | null>(null)
const message = ref('')
const feedback = ref('')
const channel = ref('none')
const screen = ref<HTMLVideoElement>()
const camera = ref<HTMLVideoElement>()
const microphone = ref<HTMLAudioElement>()
const system = ref<HTMLAudioElement>()
const cameraVisible = ref(false)
const running = computed(() =>
  ['preparing', 'recording', 'finalizing', 'stopping'].includes(phase.value),
)
let job: { id: string; cancelled: boolean; signature: string } | null = null
let task: Promise<void> | null = null
let stopping: Promise<void> | null = null
let unlisten: UnlistenFn | undefined
let disposed = false
let desired = new Set<HTMLMediaElement>()
let playbackRevision = 0
const playingRequests = new Map<HTMLMediaElement, Promise<void>>()
const tracks = () =>
  [screen.value, camera.value, microphone.value, system.value].filter(
    (track): track is HTMLMediaElement => !!track,
  )
function pause() {
  playbackRevision++
  desired.clear()
  for (const track of tracks()) track.pause()
}
function play() {
  playbackRevision++
  sync(true)
}
async function release() {
  pause()
  cameraVisible.value = false
  channel.value = 'none'
  const media = tracks()
  for (const track of media) {
    track.removeAttribute('src')
    track.load()
  }
  await Promise.all(
    media.map(
      (track) =>
        new Promise<void>((resolve, reject) => {
          const ready = () => {
            if (track.readyState === 0 && (track.networkState === 0 || track.networkState === 3)) {
              cleanup()
              resolve()
            }
          }
          const timer = setTimeout(() => {
            cleanup()
            reject(
              new Error(
                'The test player is still releasing its files. Retry discard before recording.',
              ),
            )
          }, 3000)
          const cleanup = () => {
            clearTimeout(timer)
            track.removeEventListener('emptied', ready)
            track.removeEventListener('error', ready)
          }
          track.addEventListener('emptied', ready)
          track.addEventListener('error', ready)
          ready()
        }),
    ),
  )
}
async function discardAndWait() {
  if (stopping) return stopping
  const current = job
  if (!current) {
    await release()
    return
  }
  current.cancelled = true
  phase.value = 'stopping'
  emit('busy', true)
  const work = (async () => {
    await release()
    let cancelFailure: unknown
    if (task) {
      try {
        await command('cancel_recording_test', { id: current.id })
      } catch (error) {
        cancelFailure = error
      }
      await task
    }
    if (cancelFailure) throw cancelFailure
    await command('discard_recording_test', { id: current.id })
    if (job === current) {
      job = null
      result.value = null
      phase.value = 'idle'
      message.value = ''
    }
  })()
  stopping = work
  try {
    await work
  } catch (error) {
    message.value = errorText(error)
    phase.value = 'error'
    throw error
  } finally {
    if (stopping === work) stopping = null
    emit('busy', false)
  }
}
async function start() {
  if (props.disabled || !props.active || running.value || stopping) return
  try {
    if (job) await discardAndWait()
  } catch {
    return
  }
  if (disposed || !props.active || props.disabled) return
  const current = {
    id: crypto.randomUUID(),
    cancelled: false,
    signature: JSON.stringify(props.options),
  }
  job = current
  const options = JSON.parse(current.signature) as RecordingOptions
  phase.value = 'preparing'
  message.value = ''
  feedback.value = ''
  result.value = null
  channel.value = 'none'
  emit('busy', true)
  let externallyCancelled = false
  const work = (async () => {
    try {
      await props.prepare()
      if (job !== current || current.cancelled || disposed || !props.active) return
      const value = await command<RecordingTestResult>('run_recording_test', {
        id: current.id,
        options,
      })
      if (
        job !== current ||
        current.cancelled ||
        disposed ||
        !props.active ||
        current.signature !== JSON.stringify(props.options)
      )
        return
      if (
        value.id !== current.id ||
        !value.screenPath ||
        !Number.isFinite(value.duration) ||
        value.duration <= 0
      )
        throw new Error(
          'The recorded test did not return a valid result. Discard it and try again.',
        )
      result.value = value
      phase.value = 'ready'
    } catch (error) {
      if (job === current && !current.cancelled && !disposed) {
        if (errorText(error) === 'Recording test cancelled.') externallyCancelled = true
        else {
          message.value = errorText(error)
          phase.value = 'error'
        }
      }
    }
  })()
  task = work
  try {
    await work
  } finally {
    if (task === work) task = null
    if (!current.cancelled) emit('busy', false)
  }
  if (externallyCancelled && job === current && !current.cancelled) {
    try {
      await discardAndWait()
      if (!disposed && props.active) feedback.value = 'Test cancelled.'
    } catch {
      /* Cleanup remains available through Retry discard. */
    }
  }
}
function mediaError(name: string) {
  pause()
  message.value = `${name} could not be played. Discard the test and try again.`
}
function sync(play = !!screen.value && !screen.value.paused, force = false) {
  const main = screen.value
  const value = result.value
  if (!main || !value) return
  const time = main.currentTime
  const revision = playbackRevision
  const cam = camera.value
  const cameraTime = time + (value.cameraOffsetSeconds ?? 0)
  cameraVisible.value = !!cam && cam.readyState >= 2 && cameraTime >= 0 && cameraTime < cam.duration
  const auxiliaries = [
    [cam, cameraTime, cameraVisible.value],
    [microphone.value, time, channel.value === 'microphone'],
    [system.value, time, channel.value === 'system'],
  ] as const
  const next = new Set<HTMLMediaElement>()
  for (const [track, target, enabled] of auxiliaries) {
    if (!track) continue
    if (!enabled || !Number.isFinite(track.duration) || target >= track.duration) {
      track.pause()
      continue
    }
    if (force || Math.abs(track.currentTime - target) > 0.08)
      track.currentTime = Math.min(target, track.duration)
    if (play && !main.paused) next.add(track)
  }
  desired = next
  for (const track of desired)
    if (track.paused && !playingRequests.has(track)) {
      const request = track.play()
      playingRequests.set(track, request)
      void request
        .then(() => {
          if (!desired.has(track)) track.pause()
        })
        .catch((error) => {
          if (
            revision === playbackRevision &&
            desired.has(track) &&
            result.value === value &&
            !disposed
          ) {
            pause()
            message.value = `Test playback failed. ${errorText(error)}`
          }
        })
        .finally(() => {
          if (playingRequests.get(track) === request) playingRequests.delete(track)
        })
    }
}
watch(channel, () => {
  playbackRevision++
  desired.clear()
  for (const track of [microphone.value, system.value]) track?.pause()
  sync(!!screen.value && !screen.value.paused)
})
watch(
  () => props.options,
  () => {
    if (job && job.signature !== JSON.stringify(props.options))
      void discardAndWait().catch(() => undefined)
  },
  { deep: true, flush: 'sync' },
)
watch(
  () => props.active,
  (active) => {
    if (!active) void discardAndWait().catch(() => undefined)
  },
)
onMounted(async () => {
  const stop = await listen<{ id: string; phase: 'recording' | 'finalizing' }>(
    'recording-test-phase',
    (event) => {
      if (
        job?.id === event.payload.id &&
        !job.cancelled &&
        task &&
        (event.payload.phase === 'recording' || event.payload.phase === 'finalizing')
      )
        phase.value = event.payload.phase
    },
  )
  if (disposed) stop()
  else unlisten = stop
})
onBeforeUnmount(() => {
  disposed = true
  unlisten?.()
  void discardAndWait().catch(() => undefined)
})
defineExpose({ discardAndWait })
</script>

<template>
  <section class="recording-trial" aria-label="Recording test">
    <p v-if="feedback && phase === 'idle'" role="status">{{ feedback }}</p>
    <div class="trial-heading">
      <div>
        <h3>Recording test</h3>
        <p class="field-help">
          The editor minimizes to record five seconds and returns after processing for video and
          audio review.
        </p>
      </div>
      <button type="button" :disabled="disabled || running || !active" @click="start">
        {{ phase === 'error' ? 'Retry test' : 'Test 5 seconds' }}
      </button>
    </div>
    <p v-if="running" role="status">
      {{
        phase === 'recording'
          ? 'Recording five seconds...'
          : phase === 'finalizing'
            ? 'Finalizing test. This can take longer than five seconds.'
            : phase === 'stopping'
              ? 'Releasing and discarding test...'
              : 'Preparing test...'
      }}
      <button
        v-if="phase !== 'stopping'"
        type="button"
        @click="discardAndWait().catch(() => undefined)"
      >
        Cancel test
      </button>
    </p>
    <div v-if="message" class="dialog-error" role="alert">
      {{ message }}
      <button v-if="job && !running" type="button" @click="discardAndWait().catch(() => undefined)">
        Retry discard
      </button>
    </div>
    <div v-if="result && phase === 'ready'" class="trial-result">
      <div class="trial-videos">
        <figure>
          <figcaption>
            Screen / {{ result.width }} x {{ result.height }} / {{ result.duration.toFixed(1) }} s
          </figcaption>
          <video
            ref="screen"
            :src="mediaUrl(result.screenPath)"
            controls
            muted
            playsinline
            preload="auto"
            crossorigin="anonymous"
            aria-label="Recorded test screen"
            @play="play"
            @pause="pause"
            @seeking="pause"
            @seeked="sync(false, true)"
            @timeupdate="sync(!screen?.paused)"
            @error="mediaError('Screen recording')"
          />
        </figure>
        <figure v-if="result.cameraPath">
          <figcaption>Recorded webcam</figcaption>
          <video
            ref="camera"
            :src="mediaUrl(result.cameraPath)"
            :style="{ visibility: cameraVisible ? 'visible' : 'hidden' }"
            muted
            playsinline
            preload="auto"
            crossorigin="anonymous"
            aria-label="Recorded test webcam"
            @loadeddata="sync()"
            @error="mediaError('Webcam recording')"
          />
          <span v-if="!cameraVisible" class="field-help">No webcam frame at this time.</span>
        </figure>
      </div>
      <label for="trial-audio">Listen to</label>
      <select id="trial-audio" v-model="channel">
        <option value="none">Muted</option>
        <option v-if="result.microphonePath" value="microphone">Microphone only</option>
        <option v-if="result.systemAudioPath" value="system">Computer audio only</option>
      </select>
      <p v-if="!result.microphonePath && !result.systemAudioPath" class="field-help">
        No audio track was recorded.
      </p>
      <audio
        v-if="result.microphonePath"
        ref="microphone"
        :src="mediaUrl(result.microphonePath)"
        :muted="channel !== 'microphone'"
        preload="auto"
        crossorigin="anonymous"
        @error="mediaError('Microphone recording')"
      />
      <audio
        v-if="result.systemAudioPath"
        ref="system"
        :src="mediaUrl(result.systemAudioPath)"
        :muted="channel !== 'system'"
        preload="auto"
        crossorigin="anonymous"
        @error="mediaError('Computer audio recording')"
      />
      <button type="button" class="text-button" @click="discardAndWait().catch(() => undefined)">
        Discard test
      </button>
    </div>
  </section>
</template>

<style scoped>
.recording-trial {
  margin-top: 20px;
  padding-top: 16px;
  border-top: 1px solid var(--line);
}
.trial-heading {
  display: flex;
  align-items: center;
  gap: 16px;
}
.trial-heading > div {
  flex: 1;
  min-width: 0;
}
.trial-heading .field-help {
  margin: 6px 0 0;
}
.trial-heading button {
  flex: none;
}
.trial-videos {
  display: grid;
  grid-template-columns: minmax(0, 2fr) minmax(0, 1fr);
  gap: 16px;
  margin-top: 16px;
}
.trial-videos figure {
  margin: 0;
  min-width: 0;
}
.trial-videos figcaption {
  margin-bottom: 8px;
  font-size: 12px;
}
.trial-videos video {
  display: block;
  width: 100%;
  max-height: 220px;
  background: #101114;
}
.trial-result > select {
  max-width: 280px;
}
.trial-result > .text-button {
  display: block;
  margin-top: 12px;
}
</style>
