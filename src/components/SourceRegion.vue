<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, ref, watch } from 'vue'
import { command, errorText } from '../api'
import { drawCaptureRegion, fitCaptureRegion } from '../capture-region'
import type { CaptureRegion, CaptureSource, SourceThumbnail } from '../types'

const props = defineProps<{
  source?: CaptureSource
  modelValue?: CaptureRegion | null
  disabled: boolean
}>()
const emit = defineEmits<{ 'update:modelValue': [value: CaptureRegion | null] }>()
const dialog = ref<HTMLDialogElement>()
const trigger = ref<HTMLButtonElement>()
const surface = ref<HTMLElement>()
const thumbnail = ref<SourceThumbnail | null>(null)
const draft = ref<CaptureRegion | null>(null)
const loading = ref(false)
const message = ref('')
const opened = ref(false)
let revision = 0
let pending: Promise<void> | null = null
let gesture: { id: number; kind: string; x: number; y: number; original: CaptureRegion } | null =
  null
const style = computed(() => {
  const r = draft.value
  const t = thumbnail.value
  return r && t
    ? {
        left: `${(100 * r.x) / t.width}%`,
        top: `${(100 * r.y) / t.height}%`,
        width: `${(100 * r.width) / t.width}%`,
        height: `${(100 * r.height) / t.height}%`,
      }
    : {}
})
const summary = computed(() =>
  props.modelValue
    ? `${props.modelValue.width} x ${props.modelValue.height} at ${props.modelValue.x}, ${props.modelValue.y}`
    : 'Full source',
)

async function load() {
  const source = props.source
  if (!source || props.disabled || pending) return
  const request = ++revision
  loading.value = true
  message.value = ''
  thumbnail.value = null
  const work = (async () => {
    try {
      const result = await command<SourceThumbnail>('get_source_thumbnail', { sourceId: source.id })
      if (request !== revision || !opened.value || props.source?.id !== source.id) return
      if (
        !Number.isInteger(result.width) ||
        !Number.isInteger(result.height) ||
        result.width < 2 ||
        result.height < 2 ||
        !result.data
      )
        throw new Error(
          'This source did not return a usable preview. Refresh sources and try again.',
        )
      thumbnail.value = result
      draft.value = fitCaptureRegion(
        props.modelValue ?? { x: 0, y: 0, width: result.width, height: result.height },
        result.width,
        result.height,
      )
    } catch (error) {
      if (request === revision && opened.value) message.value = errorText(error)
    } finally {
      if (request === revision) loading.value = false
    }
  })()
  pending = work
  try {
    await work
  } finally {
    if (pending === work) pending = null
  }
}
async function open() {
  if (!props.source || props.disabled) return
  opened.value = true
  dialog.value?.showModal()
  if (pending) {
    loading.value = true
    await pending
    if (!opened.value) return
  }
  await load()
}
function close() {
  const restoreFocus = opened.value
  revision++
  opened.value = false
  loading.value = false
  gesture = null
  thumbnail.value = null
  draft.value = null
  dialog.value?.close()
  if (restoreFocus)
    void nextTick(() => {
      if (!opened.value) trigger.value?.focus()
    })
}
async function closeAndWait() {
  close()
  await pending
}
function apply(full = false) {
  if (!thumbnail.value || !draft.value) return
  emit('update:modelValue', full ? null : { ...draft.value })
  close()
}
function numeric(key: keyof CaptureRegion, event: Event) {
  if (!draft.value || !thumbnail.value) return
  draft.value = fitCaptureRegion(
    { ...draft.value, [key]: Number((event.target as HTMLInputElement).value) },
    thumbnail.value.width,
    thumbnail.value.height,
  )
}
function point(event: PointerEvent) {
  const box = surface.value!.getBoundingClientRect()
  const t = thumbnail.value!
  return {
    x: Math.max(0, Math.min(t.width, ((event.clientX - box.left) / box.width) * t.width)),
    y: Math.max(0, Math.min(t.height, ((event.clientY - box.top) / box.height) * t.height)),
  }
}
function down(event: PointerEvent) {
  if (gesture || event.button !== 0 || !thumbnail.value || !draft.value) return
  event.preventDefault()
  const p = point(event)
  let kind =
    (event.target as HTMLElement).closest<HTMLElement>('[data-frame-handle]')?.dataset
      .frameHandle ?? 'draw'
  const full =
    draft.value.x === 0 &&
    draft.value.y === 0 &&
    draft.value.width === Math.floor(thumbnail.value.width / 2) * 2 &&
    draft.value.height === Math.floor(thumbnail.value.height / 2) * 2
  if (kind === 'move' && full) kind = 'draw'
  gesture = { id: event.pointerId, kind, ...p, original: { ...draft.value } }
  surface.value?.setPointerCapture(event.pointerId)
  surface.value?.focus()
  if (kind === 'draw')
    draft.value = drawCaptureRegion(p, p, thumbnail.value.width, thumbnail.value.height)
}
function move(event: PointerEvent) {
  const g = gesture
  const t = thumbnail.value
  if (!g || event.pointerId !== g.id || !t) return
  const p = point(event)
  const r = g.original
  if (g.kind === 'draw') draft.value = drawCaptureRegion(g, p, t.width, t.height)
  else if (g.kind === 'move')
    draft.value = fitCaptureRegion(
      { ...r, x: r.x + p.x - g.x, y: r.y + p.y - g.y },
      t.width,
      t.height,
    )
  else {
    const anchor = {
      x: g.kind.includes('w') ? r.x + r.width : r.x,
      y: g.kind.includes('n') ? r.y + r.height : r.y,
    }
    draft.value = drawCaptureRegion(anchor, p, t.width, t.height)
  }
}
function finish(cancel = false, pointerId?: number) {
  if (!gesture || (pointerId !== undefined && gesture.id !== pointerId)) return
  const g = gesture
  gesture = null
  if (cancel) draft.value = g.original
  if (surface.value?.hasPointerCapture(g.id)) surface.value.releasePointerCapture(g.id)
}
function keyboard(event: KeyboardEvent) {
  if (event.key === 'Escape' && gesture) {
    event.preventDefault()
    event.stopPropagation()
    finish(true)
    return
  }
  if (
    !['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown'].includes(event.key) ||
    !draft.value ||
    !thumbnail.value
  )
    return
  event.preventDefault()
  const r = { ...draft.value }
  const step = event.ctrlKey || event.metaKey ? 10 : event.shiftKey ? 2 : 1
  const horizontal = event.key === 'ArrowLeft' || event.key === 'ArrowRight'
  const direction = event.key === 'ArrowLeft' || event.key === 'ArrowUp' ? -step : step
  r[event.shiftKey ? (horizontal ? 'width' : 'height') : horizontal ? 'x' : 'y'] += direction
  draft.value = fitCaptureRegion(r, thumbnail.value.width, thumbnail.value.height)
}
watch(
  () => [props.source?.id, props.source?.width, props.source?.height],
  (value, previous) => {
    if (value.every((item, index) => item === previous[index])) return
    close()
    if (value[0] !== previous[0] && props.modelValue) emit('update:modelValue', null)
  },
)
onBeforeUnmount(close)
defineExpose({ closeAndWait })
</script>

<template>
  <div class="capture-frame-choice">
    <div>
      <strong>Capture area</strong>
      <small>{{ summary }}</small>
    </div>
    <button ref="trigger" type="button" :disabled="disabled || !source" @click="open">
      Choose area
    </button>
    <button
      v-if="modelValue"
      type="button"
      class="text-button"
      :disabled="disabled"
      @click="emit('update:modelValue', null)"
    >
      Full source
    </button>
  </div>
  <dialog
    ref="dialog"
    class="capture-frame-dialog"
    aria-labelledby="capture-frame-title"
    @cancel.prevent="close"
    @close="opened && close()"
  >
    <div class="dialog-heading">
      <h2 id="capture-frame-title">Capture area</h2>
      <button type="button" @click="close">Cancel</button>
    </div>
    <div class="capture-frame-body">
      <p class="field-help">
        {{ source?.title }}. This is a still preview. Only the selected area will be recorded.
      </p>
      <p v-if="loading" role="status">Loading source preview...</p>
      <div v-else-if="message" role="alert" class="dialog-error">
        {{ message }}
        <button type="button" @click="load">Retry preview</button>
      </div>
      <template v-else-if="thumbnail && draft">
        <div
          ref="surface"
          class="capture-frame-surface"
          tabindex="0"
          role="group"
          aria-label="Selected capture frame"
          aria-describedby="capture-frame-help"
          :style="{
            aspectRatio: `${thumbnail.width}/${thumbnail.height}`,
            width: `min(100%, ${(45 * thumbnail.width) / thumbnail.height}dvh)`,
          }"
          @pointerdown="down"
          @pointermove="move"
          @pointerup="finish(false, $event.pointerId)"
          @pointercancel="finish(true, $event.pointerId)"
          @lostpointercapture="finish(true, $event.pointerId)"
          @keydown="keyboard"
        >
          <img
            :src="`data:image/jpeg;base64,${thumbnail.data}`"
            alt="Selected recording source"
            draggable="false"
          />
          <div class="capture-frame-box" data-frame-handle="move" :style="style">
            <span
              v-for="corner in ['nw', 'ne', 'sw', 'se']"
              :key="corner"
              :class="['frame-handle', corner]"
              :data-frame-handle="corner"
            ></span>
          </div>
        </div>
        <p id="capture-frame-help" class="field-help">
          Drag to select an area. Drag inside a smaller frame to move it, or drag a corner to
          resize. Arrow keys move. Hold Shift to resize or Ctrl for 10-pixel steps.
        </p>
        <div class="capture-frame-fields">
          <label v-for="key in ['x', 'y', 'width', 'height'] as const" :key="key">
            {{ key === 'x' ? 'Left' : key === 'y' ? 'Top' : key === 'width' ? 'Width' : 'Height' }}
            <input
              type="number"
              :aria-label="`Frame ${key}`"
              :min="key === 'width' || key === 'height' ? 2 : 0"
              :step="key === 'width' || key === 'height' ? 2 : 1"
              :value="draft[key]"
              @change="numeric(key, $event)"
              @keydown.enter.prevent="numeric(key, $event)"
            />
          </label>
        </div>
      </template>
    </div>
    <footer class="capture-frame-footer">
      <button type="button" :disabled="!thumbnail" @click="apply(true)">Use full source</button>
      <button
        type="button"
        class="primary-button"
        :disabled="!thumbnail || loading"
        @click="apply()"
      >
        Use this area
      </button>
    </footer>
  </dialog>
</template>

<style scoped>
.capture-frame-choice {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 12px 0;
  border-block: 1px solid var(--line);
  margin: 12px 0;
}
.capture-frame-choice > div {
  margin-right: auto;
  min-width: 0;
}
.capture-frame-choice small {
  display: block;
  color: var(--muted);
  margin-top: 4px;
  font-size: 12px;
}
.capture-frame-dialog {
  width: min(760px, calc(100vw - 48px));
  max-height: calc(100dvh - 48px);
  padding: 0;
  overflow: hidden;
}
.capture-frame-dialog[open] {
  display: flex;
  flex-direction: column;
}
.capture-frame-dialog .dialog-heading {
  padding: 16px 20px;
  margin: 0;
}
.capture-frame-body {
  overflow: auto;
  min-height: 0;
  padding: 0 20px 16px;
}
.capture-frame-surface {
  position: relative;
  max-height: 45dvh;
  max-width: 100%;
  margin: 12px auto;
  background: #101114;
  touch-action: none;
  overflow: hidden;
  outline-offset: 3px;
}
.capture-frame-surface:focus-visible {
  outline: 2px solid var(--focus);
}
.capture-frame-surface img {
  display: block;
  width: 100%;
  height: 100%;
  object-fit: fill;
  pointer-events: none;
}
.capture-frame-box {
  position: absolute;
  border: 2px solid var(--accent);
  box-shadow: 0 0 0 3000px #0009;
  cursor: move;
}
.frame-handle {
  position: absolute;
  width: 12px;
  height: 12px;
  background: var(--accent);
  border: 1px solid #20242b;
}
.nw {
  left: -6px;
  top: -6px;
  cursor: nwse-resize;
}
.ne {
  right: -6px;
  top: -6px;
  cursor: nesw-resize;
}
.sw {
  left: -6px;
  bottom: -6px;
  cursor: nesw-resize;
}
.se {
  right: -6px;
  bottom: -6px;
  cursor: nwse-resize;
}
.capture-frame-fields {
  display: grid;
  grid-template-columns: repeat(4, minmax(0, 1fr));
  gap: 12px;
}
.capture-frame-footer {
  display: flex;
  justify-content: flex-end;
  gap: 10px;
  padding: 14px 20px;
  border-top: 1px solid var(--line);
}
</style>
