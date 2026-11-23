<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, useId, watch } from 'vue'
import { command, mediaUrl } from '../api'
import { aspectRatio, clamp, cursorAt, even, previewGeometry, smoothCursor } from '../editing'
import type { CameraSettings, CursorSample, MusicClip, Project, Redaction, SubtitleCue, VideoClip } from '../types'
import { PreviewAudio } from '../preview-audio'
import { sourceRectangle } from '../tracks'
import { maskAt, trackedBlurPatch } from '../mask-tracking'
import { PreviewImageCache } from '../image-cache'
import { canvasPoint, preserveViewportCenter, previewViewport, type PreviewScale } from '../preview-viewport'
import { cameraRectangle, contains, corners, manipulateCamera, manipulateRegion, type Manipulation, type Point, type Rectangle } from '../preview-manipulation'
import { playbackRanges, retainedSource } from '../video-edits'
import { activeWord, projectedSubtitle, subtitleAt, subtitleRasterKey } from '../subtitle-projection'
import { cueRanges } from '../subtitle-editing'
import { seekDecoded } from '../media-seek'
import { VideoBackground } from '../video-background'
import { PreviewScheduler } from '../preview-scheduler'
import { clicksAt, clickStyle, cursorClicks, indexClicks } from '../cursor-clicks'
import type { AudioMonitor, AuditionRange } from '../audition'
import { loopSourceRanges, loopSourceTarget } from '../preview-loop'
import SafeAreaGuide from './SafeAreaGuide.vue'
import { safeAreaChoice, supportsSafeArea } from '../safe-area'

const props = defineProps<{ project: Project; drawing?: boolean; selectedRegion?: string | null; selectedCamera?: boolean; disabled?: boolean; originalFrame?: boolean; monitor?: AudioMonitor; loopRange?: AuditionRange | null }>()
const emit = defineEmits<{ time: [number]; playing: [boolean]; error: [string]; region: [Rectangle]; selectCamera: []; selectRegion: [string]; beginEdit: []; endEdit: [boolean]; updateRegion: [Rectangle & { id:string }]; updateCamera: [Pick<CameraSettings, 'x' | 'y' | 'width'>]; exitOriginal: [] }>()
const canvas = ref<HTMLCanvasElement>()
const viewport = ref<HTMLDivElement>()
const previewScale = ref<PreviewScale>('fit')
const viewportSize = ref({ width: 1, height: 1 })
const pixelRatio = ref(window.devicePixelRatio || 1)
const viewportHint = useId()
const scaleId = useId()
const guideId = useId()
const guide = ref(safeAreaChoice(localStorage.getItem('screenrecord-safe-area')))
watch(guide, value => localStorage.setItem('screenrecord-safe-area', value))
const dimensions = computed(() => ({ width: even(1080 * Math.max(1, ratio.value)), height: even(1080 * Math.max(1, 1 / ratio.value)) }))
const portraitGuide = computed(() => supportsSafeArea(dimensions.value.width, dimensions.value.height))
const viewportLayout = computed(() => previewViewport(dimensions.value.width, dimensions.value.height, viewportSize.value.width, viewportSize.value.height, previewScale.value, pixelRatio.value))
const canPan = computed(() => viewportLayout.value.cssWidth > viewportLayout.value.viewportWidth + 1 || viewportLayout.value.cssHeight > viewportLayout.value.viewportHeight + 1)
let resizeObserver: ResizeObserver | undefined
let viewportRevision = 0
const video = ref<HTMLVideoElement>()
const camera = ref<HTMLVideoElement>()
const mic = ref<HTMLAudioElement>()
const system = ref<HTMLAudioElement>()
const ready = ref(false)
const screenLoadFailed = ref(false)
const playing = ref(false)
const seeking = ref(false)
const screenDuration = ref(Infinity)
function readScreenDuration() {
  const duration = video.value?.duration
  screenDuration.value = duration !== undefined && Number.isFinite(duration) ? duration : Infinity
}
function availableRanges(spans: VideoClip[]) {
  const end = screenDuration.value
  if (!spans.length || spans[spans.length - 1].end <= end) return spans
  return spans.filter(span => span.start < end).map(span => span.end <= end ? span : { ...span, end })
}
const ranges = computed(() => availableRanges(playbackRanges(props.project.edits)))
const loopRanges = computed(() => {
  const spans = loopSourceRanges(props.project.edits, props.loopRange)
  return spans && availableRanges(spans)
})
const playback = computed(() => playing.value && loopRanges.value ? loopRanges.value : ranges.value)
const pendingSeeks = new Map<HTMLMediaElement, { controller: AbortController; time: number }>()
const pendingStarts = new Map<HTMLMediaElement, () => void>()
const unverifiedSeeks = new WeakSet<HTMLVideoElement>()
function cancelPendingMedia() {
  for (const { controller } of pendingSeeks.values()) controller.abort()
  pendingSeeks.clear()
  for (const cancel of pendingStarts.values()) cancel()
  pendingStarts.clear()
}
let transitionTarget: number | null = null
let transitionFinished = Promise.resolve()
const seekFailures = new Set<HTMLMediaElement>()
const requestedPlayback = new Set<HTMLMediaElement>()
function retained(time: number) { return ranges.value.some(span => time >= span.start && time < span.end) }
function displayTime(time: number) {
  const last = ranges.value.at(-1)
  if (!last) return props.project.edits.trimStart
  const mapped = retainedSource(props.project.edits, time)
  return mapped >= last.end ? Math.max(last.start, last.end - Math.min(1 / props.project.fps, (last.end - last.start) / 2)) : mapped
}
function clearFrame() { videoBackground.clear(); const ctx = canvas.value?.getContext('2d'); if (ctx && canvas.value) { ctx.resetTransform(); ctx.fillStyle = props.project.edits.background; ctx.fillRect(0, 0, canvas.value.width, canvas.value.height) } }
const released = ref(false)
const originalSuppressed = ref(false)
const originalVisible = computed(() => !!props.originalFrame && !originalSuppressed.value)
const selectionGeometry = shallowRef<ReturnType<typeof previewGeometry> | null>(null)
const cameraBounds = shallowRef<Rectangle | null>(null)
const cameraStatus = ref('')
const canvasHelp = computed(() => props.project.edits.camera.layout === 'fullscreen'
  ? 'Camera only preview. Switch to Bubble layout in Camera settings to edit the screen.'
  : props.project.edits.camera.layout === 'split'
    ? 'Click a screen mask to select it. Drag or use arrow keys to move; corner handles resize. Escape cancels a drag.'
    : 'Click a mask or camera to select it. Drag to move; use corner handles to resize. Arrow keys move the selection; Escape cancels a drag.')
const selectionTime = ref(0)
const activeRegion = computed(() => props.project.edits.redactions.find(region => region.id === props.selectedRegion && selectionTime.value >= region.start && selectionTime.value < region.end))
const regionBounds = computed(() => {
  const region = activeRegion.value; const g = selectionGeometry.value
  if (!region || !g || !g.screenVisible || props.project.edits.camera.layout === 'fullscreen') return null
  const box = sourceRectangle(maskAt(region, selectionTime.value), props.project.width, props.project.height)
  return { x:g.fx+(box.x-g.sx)/g.sw*g.fw, y:g.fy+(box.y-g.sy)/g.sh*g.fh, width:box.width/g.sw*g.fw, height:box.height/g.sh*g.fh }
})
const controlTarget = computed<'camera' | 'region' | null>(() => props.selectedCamera && (props.project.edits.camera.layout ?? 'bubble') === 'bubble' && cameraBounds.value ? 'camera' : activeRegion.value && regionBounds.value ? 'region' : null)
const controlBounds = computed(() => controlTarget.value === 'camera' ? cameraBounds.value : controlTarget.value === 'region' ? regionBounds.value : null)
const cornerName = { nw:'top left', ne:'top right', sw:'bottom left', se:'bottom right' }
const editingGesture = ref(false)
type Gesture = { target:'camera' | 'region'; id?:string; original:Rectangle | Pick<CameraSettings, 'x' | 'y' | 'width'>; start:Point; delta:Point; handle:Manipulation; geometry:ReturnType<typeof previewGeometry>; ratio:number; pointerId:number | null; changed:boolean }
let gesture: Gesture | null = null
const loadingImages = new Set<HTMLImageElement>()
const ratio = computed(() => aspectRatio(props.project))
const cursors = shallowRef<CursorSample[]>([])
const clickIndex = computed(() => indexClicks(cursorClicks(props.project.cursor)))
const musicElements = new Map<string, HTMLAudioElement>()
let startingMusic = new Map<HTMLMediaElement, number>()
const joiningMusic = new WeakMap<HTMLMediaElement, symbol>()
const redactedFrame = document.createElement('canvas')
const videoBackground = new VideoBackground()
const blurPatch = document.createElement('canvas')
const subtitleImages = new PreviewImageCache<HTMLImageElement>()
const skippedPrefetch = new Set<string>()
const subtitlePending = new Map<string, Promise<void>>()
const subtitleFailed = new Set<string>()
const subtitleLoading = ref(false)
let subtitleTimer: ReturnType<typeof setTimeout> | undefined
let subtitleRetryTimer: ReturnType<typeof setTimeout> | undefined
let subtitleRunning = false
let subtitleKey = ''
let subtitleBaseKey = ''
let subtitleDirty = false
let draft: Pick<Redaction, 'x' | 'y' | 'width' | 'height'> | null = null
let anchor: { x: number; y: number } | null = null
watch([() => props.project.cursor, () => props.project.edits.cursorSmoothing], ([samples, smoothing]) => { cursors.value = smoothCursor(samples, smoothing) }, { immediate: true })
let lastEmission = 0
let playbackRequest = 0
let audio: PreviewAudio | null = null
let audioUnavailable = false
let disposed = false
type SceneStamp = Array<number | string | boolean>
let paintedDiscrete: SceneStamp = []
let paintedContinuous: SceneStamp = []
let lastContinuousCheck = -Infinity
const presentedTimes = new Map<HTMLVideoElement, number>()
const verifiedPausedTimes = new Map<HTMLVideoElement, { time: number; src: string }>()
const pausedFrameVerified = (source: HTMLVideoElement, time: number) => source.paused && !source.seeking && verifiedPausedTimes.get(source)?.time === time && verifiedPausedTimes.get(source)?.src === source.currentSrc
const scheduler = new PreviewScheduler({
  requestFrame: callback => requestAnimationFrame(callback), cancelFrame: handle => cancelAnimationFrame(handle),
  sources: () => [video.value, props.project.edits.camera.enabled && !originalVisible.value ? camera.value : undefined].filter((value): value is HTMLVideoElement => !!value),
  visible: () => document.visibilityState !== 'hidden', clock: loop, paint: draw,
  presented: (source, metadata) => { if (source instanceof HTMLVideoElement && Number.isFinite(metadata.mediaTime)) presentedTimes.set(source, metadata.mediaTime) },
})
const requestDraw = scheduler.invalidate
const visibilityChanged = () => { clearFrame(); presentedTimes.clear(); scheduler.visibilityChanged() }
const cameraTime = (time: number) => time + (props.project.cameraOffsetSeconds ?? 0)
const cameraMediaAvailable = (time: number) => !!camera.value && cameraTime(time) >= 0 && (!Number.isFinite(camera.value.duration) || cameraTime(time) < camera.value.duration)
const cameraAvailable = (time: number) => cameraMediaAvailable(time) && (props.project.cameraIntervals == null || props.project.cameraIntervals.some(interval => time >= interval.start && time < interval.end))
function presentedInsideSpan(source: HTMLVideoElement, screenTime: number, offset = 0) {
  const span = playback.value.find(range => screenTime >= range.start && screenTime < range.end)
  if (!span) return false
  const presented = presentedTimes.get(source)
  return presented === undefined || presented - offset < span.end
}
function frameBoundary(source: HTMLVideoElement, screenTime: number) {
  const end = playback.value.find(range => screenTime >= range.start && screenTime < range.end)?.end ?? screenTime
  const offset = source === camera.value ? props.project.cameraOffsetSeconds ?? 0 : 0
  const intervalEnd = source === camera.value ? props.project.cameraIntervals?.find(range => screenTime >= range.start && screenTime < range.end)?.end ?? end : end
  return { end: Math.min(end, intervalEnd, Number.isFinite(source.duration) ? source.duration - offset : end), offset }
}
function nearFrameBoundary(source: HTMLVideoElement, screenTime: number, mediaTime = source.currentTime) {
  const boundary = frameBoundary(source, screenTime)
  return Math.max(screenTime, mediaTime - boundary.offset) >= boundary.end - 1 / Math.min(30, Math.max(1, props.project.fps))
}
function boundaryFrame(source: HTMLVideoElement, screenTime: number, mediaTime = source.currentTime): { image: CanvasImageSource; close: () => void } | null {
  const boundary = frameBoundary(source, screenTime)
  if (nearFrameBoundary(source, screenTime, mediaTime)) {
    if (typeof VideoFrame === 'function') {
      try {
        const frame = new VideoFrame(source)
        if (Number.isFinite(frame.timestamp) && frame.timestamp / 1e6 - boundary.offset < boundary.end) return { image: frame, close: () => frame.close() }
        frame.close(); return null
      } catch { /* Older media runtimes use the conservative decoded-seek fence below. */ }
    }
    if (!pausedFrameVerified(source, mediaTime)) return null
  }
  return { image: source, close: () => {} }
}
function cameraVisibleAt(screenTime: number, mediaTime = camera.value?.currentTime ?? 0) {
  const cam = camera.value; const offset = props.project.cameraOffsetSeconds ?? 0
  const frameTime = cam && presentedTimes.get(cam)
  const interval = props.project.cameraIntervals?.find(range => screenTime >= range.start && screenTime < range.end)
  return !!(!originalVisible.value && props.project.edits.camera.enabled && cam && !cam.seeking && !unverifiedSeeks.has(cam) && !seekFailures.has(cam)
    && playback.value.some(range => mediaTime - offset >= range.start && mediaTime - offset < range.end) && cameraAvailable(mediaTime - offset) && cameraAvailable(screenTime)
    && cam.readyState >= 2 && cam.videoWidth > 0 && presentedInsideSpan(cam, screenTime, offset)
    && (frameTime === undefined || frameTime < cam.duration && (!interval || frameTime - offset < interval.end)))
}
const projectedCues = computed(() => props.project.edits.subtitles.flatMap(cue => { const safe = projectedSubtitle(props.project.edits, cue); return safe ? [safe] : [] }))
const activeCue = (time: number) => props.project.edits.subtitleStyle?.enabled ? projectedCues.value.find(cue => subtitleAt(cue, time)) : undefined
const cueKey = (cue: SubtitleCue, wordIndex: number | null) => subtitleRasterKey(cue, wordIndex, props.project.edits.subtitleStyle, props.project.edits.aspectRatio, props.project.edits.crop)

async function loadSubtitle(cue: SubtitleCue, wordIndex: number | null): Promise<void> {
  const key = cueKey(cue, wordIndex)
  if (subtitleImages.has(key) || subtitleFailed.has(key)) return
  if (subtitlePending.has(key)) return subtitlePending.get(key)
  const request = (async () => {
    try {
      const path = await command<string>('render_subtitle_preview', { id: props.project.id, cue: JSON.parse(JSON.stringify(cue)), wordIndex, style: { ...props.project.edits.subtitleStyle }, aspectRatio: props.project.edits.aspectRatio, crop: props.project.edits.crop ? { ...props.project.edits.crop } : null })
      if (disposed) return
      const image = new Image(); image.crossOrigin = 'anonymous'
      loadingImages.add(image)
      try { await new Promise<void>((resolve, reject) => { image.onload = () => resolve(); image.onerror = () => reject(new Error('Subtitle preview image is unavailable.')); image.src = mediaUrl(path) }) }
      finally { loadingImages.delete(image) }
      if (disposed) return
      if (!subtitleImages.put(key, image, [subtitleKey, subtitleBaseKey]) || key !== subtitleKey && key !== subtitleBaseKey) { skippedPrefetch.add(key); if (skippedPrefetch.size > 64) skippedPrefetch.delete(skippedPrefetch.values().next().value!) }
      if (subtitleKey === key || subtitleBaseKey === key) { subtitleLoading.value = false; requestDraw() }
    } catch (error) {
      if (/cancel|supersed/i.test(String(error))) {
        if (!disposed) subtitleRetryTimer = setTimeout(() => { subtitleRetryTimer = undefined; prepareSubtitles(video.value?.currentTime ?? props.project.edits.trimStart) }, 250)
      } else {
        subtitleFailed.add(key)
        if (subtitleFailed.size > 32) subtitleFailed.delete(subtitleFailed.values().next().value!)
        if (!disposed && subtitleKey === key) { subtitleLoading.value = false; emit('error', `Subtitle preview failed: ${String(error)} Reopen the project to retry.`) }
      }
    }
    finally { subtitlePending.delete(key) }
  })()
  subtitlePending.set(key, request)
  return request
}
function prepareSubtitles(time: number) {
  const cue = activeCue(time)
  const index = cue && props.project.edits.subtitleStyle.highlightWords ? activeWord(cue, time) : null
  subtitleKey = cue ? cueKey(cue, index) : ''; subtitleBaseKey = cue ? cueKey(cue, null) : ''
  subtitleLoading.value = !!cue && !subtitleImages.has(subtitleKey) && !subtitleFailed.has(subtitleKey)
  if (disposed || subtitleRunning || subtitleRetryTimer || subtitleDirty || !props.project.edits.subtitleStyle?.enabled) return
  const next = projectedCues.value.find(item => item !== cue && cueRanges(item).some(range => range.start > time))
  const candidates: Array<{ cue: SubtitleCue; index: number | null; active: boolean }> = []
  if (cue) {
    if (!subtitleImages.has(subtitleKey)) candidates.push({ cue, index: null, active: true })
    candidates.push({ cue, index, active: true })
  }
  for (const item of [cue, next]) if (item) {
    if (item !== cue) candidates.push({ cue: item, index: null, active: false })
    if (props.project.edits.subtitleStyle.highlightWords) candidates.push(...(item.words ?? []).flatMap((word, index) => word.end > time ? [{ cue: item, index, active: false }] : []).slice(0, 6))
  }
  const seen = new Set<string>()
  const bounded = candidates.filter(item => { const key = cueKey(item.cue, item.index); if (seen.has(key)) return false; seen.add(key); return seen.size <= 8 })
  const wanted = bounded.find(item => { const key = cueKey(item.cue, item.index); return !subtitleImages.has(key) && !subtitleFailed.has(key) && (item.active || !skippedPrefetch.has(key)) })
  if (!wanted) return
  subtitleRunning = true
  void loadSubtitle(wanted.cue, wanted.index).finally(() => { subtitleRunning = false; if (!disposed) prepareSubtitles(video.value?.currentTime ?? props.project.edits.trimStart) })
}

function censoredSource(time: number, image: CanvasImageSource): CanvasImageSource {
  if (originalVisible.value) return image
  const regions = props.project.edits.redactions?.filter(region => time >= region.start && time < region.end) ?? []
  if (!regions.length) return image
  const width = props.project.width; const height = props.project.height
  if (redactedFrame.width !== width || redactedFrame.height !== height) { redactedFrame.width = width; redactedFrame.height = height }
  const ctx = redactedFrame.getContext('2d', { alpha: false })!
  ctx.drawImage(image, 0, 0, width, height)
  for (const region of regions) {
    const evaluated = maskAt(region, time)
    const actual = sourceRectangle(evaluated, width, height)
    const rect = region.tracking && !evaluated.lost ? trackedBlurPatch(region, time, width, height) : actual
    if (evaluated.lost || region.mode === 'solid') { ctx.fillStyle = '#000'; ctx.fillRect(actual.x, actual.y, actual.width, actual.height); continue }
    const padding = Math.ceil(region.strength * 3)
    blurPatch.width = rect.width + padding * 2; blurPatch.height = rect.height + padding * 2
    const patch = blurPatch.getContext('2d', { alpha: false })!
    for (const [sx, sy, sw, sh, dx, dy, dw, dh] of [
      [rect.x, rect.y, rect.width, rect.height, padding, padding, rect.width, rect.height],
      [rect.x, rect.y, rect.width, 1, padding, 0, rect.width, padding], [rect.x, rect.y + rect.height - 1, rect.width, 1, padding, padding + rect.height, rect.width, padding],
      [rect.x, rect.y, 1, rect.height, 0, padding, padding, rect.height], [rect.x + rect.width - 1, rect.y, 1, rect.height, padding + rect.width, padding, padding, rect.height],
      [rect.x, rect.y, 1, 1, 0, 0, padding, padding], [rect.x + rect.width - 1, rect.y, 1, 1, padding + rect.width, 0, padding, padding],
      [rect.x, rect.y + rect.height - 1, 1, 1, 0, padding + rect.height, padding, padding], [rect.x + rect.width - 1, rect.y + rect.height - 1, 1, 1, padding + rect.width, padding + rect.height, padding, padding],
    ]) patch.drawImage(redactedFrame, sx, sy, sw, sh, dx, dy, dw, dh)
    ctx.save(); ctx.beginPath(); ctx.rect(actual.x, actual.y, actual.width, actual.height); ctx.clip(); ctx.filter = `blur(${region.strength}px)`; ctx.drawImage(blurPatch, rect.x - padding, rect.y - padding); ctx.restore()
  }
  return redactedFrame
}

const sameStamp = (left: SceneStamp, right: SceneStamp) => left.length === right.length && left.every((value, index) => value === right[index])
function activeClicks(time: number) {
  const edits = props.project.edits
  return originalVisible.value || !edits.showCursor || edits.showClicks === false || edits.camera.layout === 'fullscreen' ? [] : clicksAt(clickIndex.value, time, ranges.value)
}
function visibleClicks(time: number, geometry: ReturnType<typeof previewGeometry>) {
  return activeClicks(time).filter(click => { const x = click.x * props.project.width; const y = click.y * props.project.height; return x >= geometry.sx && x < geometry.sx + geometry.sw && y >= geometry.sy && y < geometry.sy + geometry.sh })
}
function discreteScene(time: number): SceneStamp {
  const stamp: SceneStamp = [cameraVisibleAt(time)]
  for (const click of activeClicks(time)) stamp.push(click.button, click.time, click.x, click.y)
  for (const region of props.project.edits.redactions) if (time >= region.start && time < region.end) {
    const rectangle = maskAt(region, time)
    stamp.push(region.id, rectangle.x, rectangle.y, rectangle.width, rectangle.height, rectangle.lost)
  }
  const cue = activeCue(time)
  if (cue) stamp.push(cue.id, props.project.edits.subtitleStyle.highlightWords ? activeWord(cue, time) ?? -1 : -1)
  return stamp
}
function continuousScene(time: number, geometry = previewGeometry(props.project, time, cursorAt(cursors.value, time)), pointer = cursorAt(cursors.value, time)): SceneStamp {
  return [geometry.sx, geometry.sy, geometry.sw, geometry.sh, ...(props.project.edits.showCursor && pointer?.visible ? [pointer.x, pointer.y] : []), ...visibleClicks(time, geometry).flatMap(click => [click.button, time - click.time])]
}
function updateTimedVisuals(time: number) {
  if (!sameStamp(paintedDiscrete, discreteScene(time))) requestDraw()
  if (time < lastContinuousCheck || time - lastContinuousCheck >= 1 / Math.max(1, props.project.fps) - 1e-6) {
    lastContinuousCheck = time
    if (!sameStamp(paintedContinuous, continuousScene(time))) requestDraw()
  }
}

function draw() {
  const target = canvas.value; const source = video.value
  if (disposed || !ready.value || !target || !source || source.readyState < 2) return
  if (seeking.value || source.seeking || unverifiedSeeks.has(source)) return
  const time = source.currentTime
  if (!retained(time) || !presentedInsideSpan(source, time)) { clearFrame(); cameraBounds.value = null; selectionGeometry.value = null; return }
  const ctx = target.getContext('2d', { alpha: false })
  if (!ctx) return
  const screenFrame = boundaryFrame(source, time, time)
  if (!screenFrame) { clearFrame(); cameraBounds.value = null; selectionGeometry.value = null; return }
  let cameraFrame: ReturnType<typeof boundaryFrame> = null
  try {
  const e = props.project.edits
  const pointer = cursorAt(cursors.value, time)
  const geometry = previewGeometry(props.project, time, pointer)
  paintedDiscrete = discreteScene(time); paintedContinuous = continuousScene(time, geometry, pointer); lastContinuousCheck = time
  selectionGeometry.value = geometry; selectionTime.value = time
  const { width: w, height: h, fw, fh, fx, fy, sw, sh, sx, sy } = geometry
  const { bufferWidth: displayWidth, bufferHeight: displayHeight, scale: displayScale } = viewportLayout.value
  if (target.width !== displayWidth || target.height !== displayHeight) { target.width = displayWidth; target.height = displayHeight }
  ctx.setTransform(displayWidth / w, 0, 0, displayHeight / h, 0, 0)
  ctx.fillStyle = e.background; ctx.fillRect(0, 0, w, h)
  if (geometry.screenVisible) {
  const frame = censoredSource(time, screenFrame.image)
  if (!originalVisible.value && e.backgroundMode === 'videoBlur') ctx.drawImage(videoBackground.draw(frame, geometry, e.backgroundBlur ?? 24), 0, 0, w, h)
  ctx.save(); ctx.beginPath(); ctx.roundRect(fx, fy, fw, fh, Math.min(e.cornerRadius, fw / 2, fh / 2)); ctx.clip()
  videoBackground.drawForeground(ctx, frame, geometry, { x: fx, y: fy, width: fw, height: fh })
  for (const click of visibleClicks(time, geometry)) {
    const px = fx + (click.x * props.project.width - sx) / sw * fw
    const py = fy + (click.y * props.project.height - sy) / sh * fh
    const style = clickStyle(click, time, e.cursorSize)
    ctx.save(); ctx.globalAlpha = style.opacity; ctx.beginPath()
    for (const radius of style.double ? [style.radius, style.radius * .64] : [style.radius]) { ctx.moveTo(px + radius, py); ctx.arc(px, py, radius, 0, Math.PI * 2) }
    ctx.strokeStyle = '#20242B'; ctx.lineWidth = style.backingWidth; ctx.stroke()
    ctx.strokeStyle = style.color; ctx.lineWidth = style.width; ctx.stroke(); ctx.restore()
  }
  if (!originalVisible.value && e.showCursor && pointer?.visible) {
    const px = fx + (pointer.x * props.project.width - sx) / sw * fw
    const py = fy + (pointer.y * props.project.height - sy) / sh * fh
    const size = clamp(Math.round(28 * e.cursorSize), 8, 256)
    if (px >= fx && px < fx + fw && py >= fy && py < fy + fh) {
      ctx.save(); ctx.translate(px - size * .08, py - size * .05); ctx.scale(size, size)
      ctx.beginPath(); ctx.moveTo(.08, .05); ctx.lineTo(.08, .80); ctx.lineTo(.28, .62); ctx.lineTo(.43, .96); ctx.lineTo(.60, .88); ctx.lineTo(.44, .55); ctx.lineTo(.73, .55); ctx.closePath()
      ctx.fillStyle = '#f5f5f5'; ctx.strokeStyle = '#191919'; ctx.lineWidth = 1.2 / size; ctx.stroke(); ctx.fill(); ctx.restore()
    }
  }
  ctx.restore()
  }
  const cam = camera.value
  const cameraClock = cam?.currentTime ?? 0
  cameraBounds.value = null
  if (cam && cameraVisibleAt(time, cameraClock)) cameraFrame = boundaryFrame(cam, time, cameraClock)
  const cameraVisible = !!cameraFrame
  cameraStatus.value = cameraVisible || geometry.layout === 'bubble' ? '' : !props.project.cameraPath || e.camera.enabled ? 'Camera unavailable' : 'Camera hidden'
  if (cameraVisible && cam) {
    const bubble = geometry.layout === 'bubble'
    const cameraRatio = bubble && e.camera.shape === 'circle' ? 1 : cam.videoWidth / cam.videoHeight
    const box = geometry.cameraPane ?? cameraRectangle(e.camera, w, h, cameraRatio)
    if (bubble) cameraBounds.value = box
    const { x:cx, y:cy, width:cw, height:ch } = box
    ctx.save(); ctx.beginPath()
    if (bubble && e.camera.shape === 'circle') ctx.arc(cx + cw / 2, cy + ch / 2, cw / 2, 0, Math.PI * 2)
    else if (bubble) ctx.rect(cx, cy, cw, ch)
    else ctx.roundRect(cx, cy, cw, ch, Math.min(e.cornerRadius, cw / 2, ch / 2))
    ctx.clip()
    let sx = 0; let sy = 0; let scw = cam.videoWidth; let sch = cam.videoHeight
    if (bubble && e.camera.shape === 'circle') { const side = Math.min(scw, sch); sx = (scw - side) / 2; sy = (sch - side) / 2; scw = side; sch = side }
    else if (!bubble) { scw = clamp(Math.round(Math.min(cam.videoWidth, cam.videoHeight * cw / ch)), 1, cam.videoWidth); sch = clamp(Math.round(Math.min(cam.videoHeight, cam.videoWidth * ch / cw)), 1, cam.videoHeight); sx = Math.floor((cam.videoWidth - scw) / 2); sy = Math.floor((cam.videoHeight - sch) / 2) }
    if (e.camera.mirror) { ctx.translate(cx + cw, cy); ctx.scale(-1, 1); ctx.drawImage(cameraFrame!.image, sx, sy, scw, sch, 0, 0, cw, ch) }
    else ctx.drawImage(cameraFrame!.image, sx, sy, scw, sch, cx, cy, cw, ch)
    ctx.restore()
  }
  prepareSubtitles(time)
  const subtitle = !originalVisible.value ? subtitleImages.get(subtitleKey) ?? subtitleImages.get(subtitleBaseKey) : undefined
  if (subtitle) ctx.drawImage(subtitle, 0, 0, w, h)
  const selected = draft ?? e.redactions?.find(region => region.id === props.selectedRegion && time >= region.start && time < region.end)
  if (selected && geometry.screenVisible) {
    const rect = sourceRectangle(draft ?? maskAt(selected as Redaction, time), props.project.width, props.project.height)
    ctx.save(); ctx.beginPath(); ctx.rect(fx, fy, fw, fh); ctx.clip(); ctx.strokeStyle = '#e9b451'; ctx.lineWidth = 2 / displayScale; ctx.setLineDash([7 / displayScale, 4 / displayScale]); ctx.strokeRect(fx + (rect.x - sx) / sw * fw, fy + (rect.y - sy) / sh * fh, rect.width / sw * fw, rect.height / sh * fh); ctx.restore()
  }
  } finally { screenFrame.close(); cameraFrame?.close() }
}
function pointerSource(event: PointerEvent, requireInside = false) {
  if (!canvas.value || !video.value) return null
  const geometry = previewGeometry(props.project, video.value.currentTime, cursorAt(cursors.value, video.value.currentTime))
  if (!geometry.screenVisible) return null
  const { x, y } = canvasPoint(event.clientX, event.clientY, canvas.value.getBoundingClientRect(), geometry.width, geometry.height)
  if (requireInside && (x < geometry.fx || x > geometry.fx + geometry.fw || y < geometry.fy || y > geometry.fy + geometry.fh)) return null
  return { x: (geometry.sx + clamp((x - geometry.fx) / geometry.fw) * geometry.sw) / props.project.width, y: (geometry.sy + clamp((y - geometry.fy) / geometry.fh) * geometry.sh) / props.project.height }
}
function drawStart(event: PointerEvent) {
  if (disposed || props.disabled || event.button !== 0) return
  if (props.project.edits.camera.layout === 'fullscreen') return
  if (!props.drawing) {
    const g = selectionGeometry.value; if (!g || !canvas.value) return
    const point = canvasPoint(event.clientX,event.clientY,canvas.value.getBoundingClientRect(),g.width,g.height)
    const box = cameraBounds.value
    if (box && contains(point,box) && (props.project.edits.camera.shape !== 'circle' || Math.hypot((point.x-box.x-box.width/2)/(box.width/2),(point.y-box.y-box.height/2)/(box.height/2)) <= 1)) { startPointer(event,'camera','move'); return }
    if (!contains(point,{ x:g.fx,y:g.fy,width:g.fw,height:g.fh })) return
    const source = { x:(g.sx+(point.x-g.fx)/g.fw*g.sw)/props.project.width,y:(g.sy+(point.y-g.fy)/g.fh*g.sh)/props.project.height }
    const region = [...props.project.edits.redactions].reverse().find(item => selectionTime.value >= item.start && selectionTime.value < item.end && contains(source,maskAt(item, selectionTime.value)))
    if (region) startPointer(event,'region','move',region.id)
    return
  }
  anchor = pointerSource(event, true)
  if (anchor) { pause(); canvas.value?.setPointerCapture(event.pointerId); event.preventDefault() }
}
function drawMove(event: PointerEvent) {
  if (gesture?.pointerId === event.pointerId) { movePointer(event); return }
  if (!anchor || props.disabled) return
  const point = pointerSource(event); if (!point) return
  draft = { x: Math.min(anchor.x, point.x), y: Math.min(anchor.y, point.y), width: Math.abs(point.x - anchor.x), height: Math.abs(point.y - anchor.y) }; requestDraw()
}
function drawEnd(event: PointerEvent) {
  if (gesture?.pointerId === event.pointerId) { movePointer(event); finishGesture(true); return }
  if (!anchor) return
  drawMove(event)
  if (!props.disabled && draft && draft.width >= .001 && draft.height >= .001 && draft.width * props.project.width >= 2 && draft.height * props.project.height >= 2) emit('region', draft)
  cancelDrawing(event.pointerId)
  if (originalVisible.value) { hideOriginal(); emit('exitOriginal') }
}
function cancelDrawing(pointerId?:number) { anchor = null; draft = null; if (pointerId !== undefined && canvas.value?.hasPointerCapture(pointerId)) canvas.value.releasePointerCapture(pointerId); requestDraw() }
function abortDrawing(pointerId: number) { finishGesture(false); cancelDrawing(pointerId); if (originalVisible.value) { hideOriginal(); emit('exitOriginal') } }
function startGesture(target:'camera' | 'region', handle:Manipulation, point:Point, pointerId:number | null, id = props.selectedRegion ?? undefined) {
  if (disposed || props.disabled || seeking.value || video.value?.seeking || gesture || props.drawing || !selectionGeometry.value) return false
  if (target === 'camera' && (props.project.edits.camera.layout ?? 'bubble') !== 'bubble' || target === 'region' && !selectionGeometry.value.screenVisible) return false
  const region = props.project.edits.redactions.find(item => item.id === id)
  if (target === 'camera' ? !cameraBounds.value : !region) return false
  pause()
  if (target === 'camera') emit('selectCamera'); else emit('selectRegion', id!)
  const tracked = region ? maskAt(region, selectionTime.value) : null
  const original = target === 'camera' ? { x:props.project.edits.camera.x,y:props.project.edits.camera.y,width:props.project.edits.camera.width } : { x:tracked!.x,y:tracked!.y,width:tracked!.width,height:tracked!.height }
  gesture = { target,id,original,start:point,delta:{ x:0,y:0 },handle,geometry:selectionGeometry.value,ratio:target === 'camera' && props.project.edits.camera.shape !== 'circle' ? camera.value!.videoWidth/camera.value!.videoHeight : 1,pointerId,changed:false }
  editingGesture.value = true; emit('beginEdit'); return true
}
function startPointer(event:PointerEvent, target:'camera' | 'region', handle:Manipulation, id?:string) {
  if (event.button !== 0 || !canvas.value || !selectionGeometry.value) return
  const g = selectionGeometry.value
  const point = canvasPoint(event.clientX,event.clientY,canvas.value.getBoundingClientRect(),g.width,g.height)
  const box = cameraBounds.value
  if (target === 'region' && handle === 'move' && box && contains(point,box) && (props.project.edits.camera.shape !== 'circle' || Math.hypot((point.x-box.x-box.width/2)/(box.width/2),(point.y-box.y-box.height/2)/(box.height/2)) <= 1)) { target = 'camera'; id = undefined }
  if (!startGesture(target,handle,point,event.pointerId,id)) return
  canvas.value.focus({ preventScroll:true }); canvas.value.setPointerCapture(event.pointerId); event.preventDefault(); event.stopPropagation()
}
function applyGesture() {
  const g = gesture; if (!g) return
  if (g.target === 'camera') emit('updateCamera',manipulateCamera(g.original,g.delta,g.handle,g.geometry.width,g.geometry.height,g.ratio))
  else emit('updateRegion',{ id:g.id!,...manipulateRegion(g.original as Rectangle,g.delta,g.handle,props.project.width,props.project.height) })
}
function movePointer(event:PointerEvent) {
  const g = gesture; if (!g || !canvas.value || props.disabled) return
  const point = canvasPoint(event.clientX,event.clientY,canvas.value.getBoundingClientRect(),g.geometry.width,g.geometry.height)
  g.delta = { x:point.x-g.start.x,y:point.y-g.start.y }
  if (g.target === 'region') g.delta = { x:g.delta.x/g.geometry.fw*g.geometry.sw/props.project.width,y:g.delta.y/g.geometry.fh*g.geometry.sh/props.project.height }
  if (g.delta.x || g.delta.y || g.changed) { g.changed = true; applyGesture() }
}
function finishGesture(commit:boolean) {
  const g = gesture; if (!g) return
  gesture = null; editingGesture.value = false
  if (!commit) { if (g.target === 'camera') emit('updateCamera',g.original); else emit('updateRegion',{ id:g.id!,...g.original as Rectangle }) }
  if (g.pointerId !== null && canvas.value?.hasPointerCapture(g.pointerId)) canvas.value.releasePointerCapture(g.pointerId)
  emit('endEdit',commit); void nextTick(requestDraw)
}
function editKeys(event:KeyboardEvent, target = controlTarget.value, handle:Manipulation = 'move') {
  if (event.key === 'Escape' && originalVisible.value) { hideOriginal(); emit('exitOriginal'); event.preventDefault(); event.stopPropagation(); return }
  if (event.key === 'Escape') { if (gesture || anchor) { finishGesture(false); cancelDrawing(); event.preventDefault(); event.stopPropagation() }; return }
  if (!target || props.disabled || disposed || event.ctrlKey || event.metaKey || event.altKey) return
  const directions:Record<string,Point> = { ArrowLeft:{ x:-1,y:0 },ArrowRight:{ x:1,y:0 },ArrowUp:{ x:0,y:-1 },ArrowDown:{ x:0,y:1 } }
  const direction = directions[event.key]; if (!direction) return
  if (!gesture && !startGesture(target,handle,{ x:0,y:0 },null)) return
  if (gesture?.pointerId !== null) return
  const step = event.shiftKey ? 10 : 1
  gesture.delta.x += direction.x*step/(target === 'region' ? props.project.width : 1)
  gesture.delta.y += direction.y*step/(target === 'region' ? props.project.height : 1)
  applyGesture(); event.preventDefault(); event.stopPropagation()
}
function finishKeys(event:KeyboardEvent) { if (gesture?.pointerId === null && event.key.startsWith('Arrow')) { finishGesture(true); event.preventDefault(); event.stopPropagation() } }
function selectionStyle(box:Rectangle) { const scale = viewportLayout.value.scale; return { left:`${box.x*scale}px`,top:`${box.y*scale}px`,width:`${box.width*scale}px`,height:`${box.height*scale}px` } }
function retireTrack(element: HTMLMediaElement) {
  pendingSeeks.get(element)?.controller.abort()
  pendingStarts.get(element)?.()
  requestedPlayback.delete(element)
  joiningMusic.delete(element)
  if (!element.paused) element.pause()
}
function registerMusic(id: string, value: unknown) {
  const previous = musicElements.get(id)
  if (value === previous) return
  if (previous) retireTrack(previous)
  if (value instanceof HTMLAudioElement) musicElements.set(id, value)
  else musicElements.delete(id)
  syncMusicTracks()
}
function syncMusicTracks() {
  audio?.setMusicTracks((props.project.edits.music ?? []).flatMap(clip => { const element = musicElements.get(clip.id); return element ? [{ clip, element }] : [] }))
}
function musicPosition(clip: MusicClip, element: HTMLAudioElement, clock: number) {
  if (element.error || clip.volume <= 0 || clock < clip.start || clock >= clip.start + clip.duration) return null
  const target = clip.sourceStart + (clock - clip.start)
  return Number.isFinite(element.duration) && target >= element.duration ? null : target
}
function audioClock(time: number, running: boolean) {
  if (!running) return time
  const span = playback.value.find(span => time >= span.start && time < span.end)
  return Math.min(time + (audio?.latencySeconds ?? 0), span?.end ?? time)
}
function playPreparedMusic(element: HTMLMediaElement) {
  const request = playbackRequest
  const token = Symbol()
  joiningMusic.set(element, token)
  requestedPlayback.add(element)
  element.playbackRate = element.defaultPlaybackRate
  void playWithDeadline(element).catch(error => {
    if (request === playbackRequest && joiningMusic.get(element) === token) {
      requestedPlayback.delete(element)
      element.pause()
      seekFailures.add(element)
      emit('error', `${element.dataset.track} preview cannot play: ${String(error)}`)
    }
  }).finally(() => { if (joiningMusic.get(element) === token) joiningMusic.delete(element) })
}
function syncMusic(time: number) {
  if (!audio || disposed || (playing.value && audioUnavailable)) return
  const clock = audioClock(time, playing.value)
  const active = new Map<HTMLMediaElement, { clip: MusicClip; target: number }>()
  for (const clip of props.project.edits.music ?? []) {
    const element = musicElements.get(clip.id)
    if (!element) continue
    const target = musicPosition(clip, element, clock)
    if (target === null) {
      retireTrack(element)
      if (clock < clip.start && clip.volume > 0 && !element.error && element.readyState >= 1 && !element.seeking
        && !seekFailures.has(element) && clip.sourceStart < element.duration && Math.abs(element.currentTime - clip.sourceStart) > 1e-5) {
        element.currentTime = clip.sourceStart
      }
      continue
    }
    active.set(element, { clip, target })
    const pending = pendingSeeks.get(element)
    if (pending && pending.time !== target) pending.controller.abort()
  }
  if (seeking.value) return
  if (transitionTarget !== null) {
    if (active.size !== startingMusic.size || [...active].some(([element, { target }]) => startingMusic.get(element) !== target)) {
      void transition(transitionTarget, playing.value)
    }
    return
  }
  for (const [element, { clip, target }] of active) {
    if (seekFailures.has(element) || joiningMusic.has(element)) continue
    const drift = Math.abs(element.currentTime - target)
    if (playing.value && audio.running && element.paused && element.readyState >= 2 && !element.seeking
      && Math.abs(element.currentTime - clip.sourceStart) < 1e-6 && drift <= 1 / props.project.fps) {
      const span = playback.value.find(range => time >= range.start && time < range.end)
      if (span && clip.start >= span.start && clip.start < span.end) {
        playPreparedMusic(element)
        continue
      }
    }
    if (playing.value && (element.paused && !element.ended || drift > .12)) {
      void transition(time, true)
      return
    }
    if (!playing.value && element.readyState >= 1 && drift > 1e-5) element.currentTime = target
  }
  audio.setMusicTime(clock, playing.value)
}
type MediaTarget = { element: HTMLMediaElement; time: number; required: boolean }
function recordedTargets(time: number, audioTime: number): MediaTarget[] {
  const targets: MediaTarget[] = []
  if (camera.value && !camera.value.error && props.project.edits.camera.enabled && cameraMediaAvailable(time)) {
    targets.push({ element: camera.value, time: cameraTime(time), required: false })
  }
  if (playing.value && audioUnavailable) return targets
  for (const [element, volume] of [[mic.value, props.project.edits.microphoneVolume], [system.value, props.project.edits.systemVolume]] as const) {
    if (element && !element.error && volume > 0 && (!Number.isFinite(element.duration) || audioTime < element.duration)) {
      targets.push({ element, time: audioTime, required: false })
    }
  }
  return targets
}
function syncAux(time: number) {
  if (seeking.value || disposed) return
  let audioMoved = false
  const audioTime = audioClock(time, playing.value)
  for (const { element, time: target } of recordedTargets(time, audioTime)) {
    if (element.readyState < 1 || seekFailures.has(element)) continue
    if (Math.abs(element.currentTime - target) > .12 || (playing.value && element.paused && !element.ended)) {
      if (playing.value) { void transition(time, true); return }
      element.currentTime = target
      audioMoved ||= element !== camera.value
    }
  }
  if (audioMoved) audio?.reset()
  syncMusic(time)
}
function gateAudio(time: number) {
  if (audioUnavailable) return
  const span = playback.value.find(span => time >= span.start && time < span.end)
  if (span) audio?.gateSpan(time, span.end); else audio?.pause()
}
function loop(now: number) {
  const source = video.value
  if (!source || !playing.value || seeking.value) return
  const last = playback.value.at(-1)
  if (!last || source.currentTime >= last.end) { void transition(loopRanges.value?.[0]?.start ?? last?.end ?? 0, !!loopRanges.value); return }
  if (!playback.value.some(span => source.currentTime >= span.start && source.currentTime < span.end)) { void transition(source.currentTime, true, true); return }
  gateAudio(source.currentTime); updateTimedVisuals(source.currentTime); syncAux(source.currentTime)
  if (now - lastEmission > 30) { emit('time', source.currentTime); lastEmission = now }
}
function mediaTargets(time: number, running: boolean) {
  const audioTime = audioClock(time, running)
  const targets: MediaTarget[] = []
  if (video.value) targets.push({ element: video.value, time, required: true })
  targets.push(...recordedTargets(time, audioTime))
  if (running && audioUnavailable) return targets
  for (const clip of props.project.edits.music ?? []) {
    const element = musicElements.get(clip.id)
    if (!element) continue
    const position = musicPosition(clip, element, audioTime)
    if (position !== null) targets.push({ element, time: position, required: false })
  }
  return targets
}
async function playWithDeadline(element: HTMLMediaElement) {
  let cancel!: () => void
  let timer!: ReturnType<typeof setTimeout>
  const deadline = new Promise<void>((resolve, reject) => {
    cancel = resolve
    timer = setTimeout(() => reject(new Error(`Starting ${element.dataset.track ?? 'media'} took too long. Try playback again.`)), 7000)
  })
  pendingStarts.get(element)?.()
  pendingStarts.set(element, cancel)
  try {
    const started = element.play().then(() => {
      if (!requestedPlayback.has(element)) element.pause()
    })
    await Promise.race([started, deadline])
  } finally {
    clearTimeout(timer)
    if (pendingStarts.get(element) === cancel) pendingStarts.delete(element)
  }
}
function pauseMedia() {
  for (const element of [video.value, camera.value, mic.value, system.value, ...musicElements.values()]) element?.pause()
}
function transition(time: number, running: boolean, preserveDucking = false) {
  transitionFinished = runTransition(time, running, preserveDucking)
  return transitionFinished
}
async function runTransition(time: number, running: boolean, preserveDucking: boolean): Promise<void> {
  if (originalVisible.value) running = false
  if (disposed || screenLoadFailed.value || !video.value) return
  if (!ranges.value.length || running && loopRanges.value?.length === 0) {
    stopPlayback()
    if (!ranges.value.length) clearFrame()
    emit('error', ranges.value.length
      ? 'This loop contains no screen frames. Move the loop start earlier.'
      : 'This edit contains no screen frames. Move the trim or cut boundary earlier.')
    return
  }
  const request = ++playbackRequest
  cancelPendingMedia()
  requestedPlayback.clear()
  seeking.value = true; scheduler.stop(); presentedTimes.clear()
  clearFrame()
  pauseMedia()
  audio?.pause(); playing.value = running; emit('playing', running)
  const target = running && loopRanges.value ? loopSourceTarget(loopRanges.value, time) : displayTime(time); seekFailures.clear()
  transitionTarget = target
  try {
    const settledTimes = new Map<HTMLMediaElement, number>()
    await Promise.all(mediaTargets(target, running).map(async track => {
      const controller = new AbortController()
      pendingSeeks.set(track.element, { controller, time: track.time })
      const source = track.element instanceof HTMLVideoElement ? track.element : null
      try {
        const verify = !!source && (unverifiedSeeks.has(source) || !pausedFrameVerified(source, track.time) && nearFrameBoundary(source, target, track.time))
        if (source) { verifiedPausedTimes.delete(source); unverifiedSeeks.add(source) }
        await seekDecoded(track.element, track.time, controller.signal, verify)
        if (controller.signal.aborted || request !== playbackRequest) return
        if (source) {
          unverifiedSeeks.delete(source)
          verifiedPausedTimes.set(source, { time: source.currentTime, src: source.currentSrc })
        }
      } catch (error) {
        if (controller.signal.aborted || request !== playbackRequest) return
        if (track.required) throw error
        seekFailures.add(track.element); emit('error', String(error))
      } finally {
        if (pendingSeeks.get(track.element)?.controller === controller) pendingSeeks.delete(track.element)
      }
      settledTimes.set(track.element, track.time)
    }))
    if (request !== playbackRequest || disposed) return
    const currentTracks = mediaTargets(target, running)
    if (currentTracks.some(track => settledTimes.get(track.element) !== track.time)) {
      await transition(target, running && playing.value, preserveDucking)
      return
    }
    const music = new Set(musicElements.values())
    startingMusic = new Map(currentTracks.filter(track => music.has(track.element as HTMLAudioElement)).map(track => [track.element, track.time]))
    seeking.value = false; emit('time', target); scheduler.paintNow()
    if (!running || !playing.value) return
    if (audio && !audioUnavailable) {
      try { await audio.resume(); if (request !== playbackRequest || disposed) return; audio.reset(!preserveDucking) }
      catch (error) {
        if (request !== playbackRequest || disposed) return
        audioUnavailable = true
        audio.pause()
        emit('error', String(error))
      }
    }
    if (request !== playbackRequest || disposed || props.disabled) return
    const tracks = mediaTargets(target, true).filter(track => !seekFailures.has(track.element))
    if (tracks.some(track => settledTimes.get(track.element) !== track.time)) {
      await transition(target, true, preserveDucking)
      return
    }
    verifiedPausedTimes.clear()
    await Promise.all(tracks.map(async track => {
      requestedPlayback.add(track.element)
      try {
        // Keep the decoded frame and clocks at the seek target until every decoder can play.
        track.element.playbackRate = 0
        await playWithDeadline(track.element)
      }
      catch (error) {
        if (request !== playbackRequest || !requestedPlayback.has(track.element)) return
        requestedPlayback.delete(track.element)
        track.element.pause()
        if (track.required) throw error
        seekFailures.add(track.element); emit('error', `${track.element.dataset.track} preview cannot play: ${String(error)}`)
      }
    }))
    if (request !== playbackRequest || disposed) return
    if (!audioUnavailable) audio?.setMusicTime(target + audio.latencySeconds, true)
    gateAudio(video.value.currentTime)
    for (const { element } of tracks) if (requestedPlayback.has(element)) element.playbackRate = element.defaultPlaybackRate
    scheduler.start()
  } catch (error) {
    if (request !== playbackRequest || disposed) return
    stopPlayback(); emit('error', String(error))
  } finally { if (request === playbackRequest) { seeking.value = false; transitionTarget = null } }
}
async function play() {
  const source = video.value
  if (disposed || props.disabled || originalVisible.value || !source || (!ready.value && !screenLoadFailed.value)) return
  const last = ranges.value.at(-1)
  const playhead = transitionTarget ?? source.currentTime
  const target = loopRanges.value ? loopSourceTarget(loopRanges.value, playhead) : !last || playhead >= last.end - 1 / props.project.fps - 1e-6 ? ranges.value[0]?.start ?? 0 : playhead
  if (screenLoadFailed.value) {
    screenLoadFailed.value = false
    source.load()
  }
  audioUnavailable = false
  await transition(target, true)
}
function stopPlayback() {
  playbackRequest++; cancelPendingMedia(); transitionTarget = null; requestedPlayback.clear(); seeking.value = false; scheduler.stop()
  pauseMedia()
  audio?.pause(); playing.value = false; emit('playing', false)
  const source = video.value
  if (source && !source.seeking && !unverifiedSeeks.has(source) && ready.value) { emit('time', displayTime(source.currentTime)); scheduler.paintNow() }
}
function pause() {
  if (seeking.value && transitionTarget !== null && ready.value && !disposed && !video.value?.error) {
    playing.value = false
    emit('playing', false)
    return transitionFinished
  }
  stopPlayback()
}
function seek(time: number) { void transition(time, playing.value) }
function ended() {
  if (!playing.value || seeking.value || disposed || props.disabled || !video.value) return
  const start = loopRanges.value?.[0]?.start
  void transition(start ?? video.value.currentTime, start !== undefined)
}
async function pauseAtFrame(): Promise<{ time: number; x: number; y: number } | null> {
  const pendingPause = pause()
  if (pendingPause) {
    const request = playbackRequest
    await pendingPause
    if (request !== playbackRequest) return null
  }
  const source = video.value
  if (!source || !ready.value || disposed) return null
  const target = displayTime(source.currentTime)
  const unverifiedBoundary = nearFrameBoundary(source, target) && !pausedFrameVerified(source, source.currentTime)
  if (source.seeking || unverifiedSeeks.has(source) || target !== source.currentTime || unverifiedBoundary) {
    const pending = transition(target, false); const request = playbackRequest
    await pending
    if (request !== playbackRequest) return null
  }
  if (disposed || seeking.value || source.seeking || unverifiedSeeks.has(source) || source.readyState < 2 || !retained(source.currentTime)) return null
  const geometry = previewGeometry(props.project, source.currentTime, cursorAt(cursors.value, source.currentTime))
  return { time: source.currentTime, x: (geometry.sx + geometry.sw / 2) / props.project.width, y: (geometry.sy + geometry.sh / 2) / props.project.height }
}
function toggle() { if (playing.value) pause(); else void play() }
function measureViewport() {
  const target = viewport.value
  if (target && (viewportSize.value.width !== target.clientWidth || viewportSize.value.height !== target.clientHeight)) viewportSize.value = { width: target.clientWidth, height: target.clientHeight }
  pixelRatio.value = window.devicePixelRatio || 1
}
function centerViewport() {
  const target = viewport.value
  if (target) target.scrollTo((target.scrollWidth - target.clientWidth) / 2, (target.scrollHeight - target.clientHeight) / 2)
}
function panWithKeyboard(event: KeyboardEvent) {
  const target = viewport.value
  if (!target || event.target !== target || previewScale.value === 'fit' || !canPan.value || event.ctrlKey || event.metaKey || event.altKey) return
  const step = event.shiftKey ? 120 : 40
  const moves: Record<string, [number, number]> = { ArrowLeft: [-step, 0], ArrowRight: [step, 0], ArrowUp: [0, -step], ArrowDown: [0, step], PageUp: [0, -target.clientHeight * .8], PageDown: [0, target.clientHeight * .8] }
  if (event.key === 'Home') target.scrollTo(0, 0)
  else if (event.key === 'End') target.scrollTo(target.scrollWidth, target.scrollHeight)
  else if (moves[event.key]) target.scrollBy(...moves[event.key])
  else return
  event.preventDefault(); event.stopPropagation()
}
watch(viewportLayout, (layout, previous) => {
  const revision = ++viewportRevision
  const position = preserveViewportCenter(previous, layout, viewport.value?.scrollLeft ?? 0, viewport.value?.scrollTop ?? 0)
  void nextTick(() => { if (disposed || revision !== viewportRevision) return; viewport.value?.scrollTo(position.left, position.top); measureViewport(); requestDraw() })
})
function loaded() {
  if (disposed || ready.value) return
  ready.value = true
  const source = video.value
  if (!source || seeking.value) return
  if (playing.value || pausedFrameVerified(source, source.currentTime) && retained(source.currentTime)) {
    requestDraw()
    return
  }
  seek(ranges.value[0]?.start ?? props.project.edits.trimStart)
}
function auxiliaryLoaded() { if (disposed || seeking.value || playing.value) return; syncAux(video.value?.currentTime ?? props.project.edits.trimStart); requestDraw() }
function mediaError(event: Event) {
  if (disposed) return
  const target = event.target as HTMLMediaElement
  if (target === video.value) {
    screenLoadFailed.value = true
    ready.value = false
    stopPlayback()
    cameraBounds.value = null
    selectionGeometry.value = null
    clearFrame()
  }
  emit('error', `Cannot preview ${target.dataset.track ?? 'video'}. The media file may be unavailable or use an unsupported codec.`)
}
watch(() => props.originalFrame, value => { originalSuppressed.value = false; pause(); clearFrame(); if (!value) { anchor = null; draft = null }; requestDraw() }, { flush: 'sync' })
function hideOriginal() { originalSuppressed.value = true; pause(); anchor = null; draft = null; clearFrame(); requestDraw() }
watch(() => props.project.id, () => { ready.value = false; presentedTimes.clear(); verifiedPausedTimes.clear(); finishGesture(false); pause(); clearFrame() })
watch(() => props.project.screenPath, () => { ready.value = false; screenLoadFailed.value = false; presentedTimes.clear(); verifiedPausedTimes.clear(); selectionGeometry.value = null; pause(); clearFrame() }, { flush: 'sync' })
watch([() => props.loopRange?.start, () => props.loopRange?.end], () => { if (ready.value && playing.value) void transition(video.value?.currentTime ?? 0, true) }, { flush: 'sync' })
watch(() => props.monitor, value => audio?.setMonitor(value), { flush: 'sync' })
watch(() => JSON.stringify([props.project.edits.trimStart, props.project.edits.trimEnd, props.project.edits.videoClips]), () => { clearFrame(); if (ready.value) void transition(video.value?.currentTime ?? 0, playing.value) }, { flush: 'sync' })
watch(() => props.disabled, disabled => { if (disabled) { finishGesture(false); cancelDrawing(); pause() } })
watch([() => props.project.edits.camera.layout, () => props.project.edits.camera.enabled], () => { clearFrame(); cameraBounds.value = null; selectionGeometry.value = null; finishGesture(false); cancelDrawing() }, { flush: 'sync' })
watch(() => props.project.edits.fitMode, () => { clearFrame(); selectionGeometry.value = null; finishGesture(false); cancelDrawing() }, { flush: 'sync' })
watch(() => JSON.stringify(props.project.edits.reframeKeyframes), () => { clearFrame(); selectionGeometry.value = null; finishGesture(false); cancelDrawing() }, { flush: 'sync' })
watch([() => props.project.edits.backgroundMode, () => props.project.edits.backgroundBlur, () => props.project.edits.background], clearFrame, { flush: 'sync' })
watch([() => props.project.edits.showCursor, () => props.project.edits.showClicks], clearFrame, { flush: 'sync' })
watch(() => props.drawing, () => { anchor = null; draft = null; requestDraw() })
watch(() => props.selectedRegion, requestDraw)
watch(() => props.selectedCamera, requestDraw)
watch(() => JSON.stringify([props.project.edits.redactions, props.project.edits.crop]), clearFrame, { flush: 'sync' })
watch(() => JSON.stringify([props.project.edits.subtitles, props.project.edits.subtitleStyle, props.project.edits.aspectRatio, props.project.edits.crop]), () => {
  clearFrame(); subtitleKey = ''; subtitleBaseKey = ''
  subtitleDirty = true; clearTimeout(subtitleTimer)
  subtitleTimer = setTimeout(() => { subtitleDirty = false; prepareSubtitles(video.value?.currentTime ?? props.project.edits.trimStart); requestDraw() }, 250)
}, { flush: 'sync' })
watch(() => JSON.stringify(props.project.edits.music), () => {
  syncMusicTracks()
  syncMusic(transitionTarget ?? video.value?.currentTime ?? 0)
}, { flush: 'post' })
watch([() => props.project.edits.camera.enabled, () => props.project.edits.microphoneVolume > 0, () => props.project.edits.systemVolume > 0], (enabled, previous) => {
  if (disposed || !ready.value || !video.value) return
  const media = [camera.value, mic.value, system.value]
  for (const [index, element] of media.entries()) {
    if (!element || enabled[index]) continue
    retireTrack(element)
  }
  if (seeking.value) return
  const joined = media.some((element, index) => element && enabled[index] && !previous[index])
  const cameraShown = !!camera.value && enabled[0] && !previous[0]
  if (transitionTarget !== null || cameraShown || (joined && playing.value)) {
    void transition(transitionTarget ?? video.value.currentTime, playing.value)
  }
})
watch(() => props.project.edits, () => {
  if (disposed) return
  audio?.setVolumes(props.project.edits.microphoneVolume, props.project.edits.systemVolume)
  audio?.setProcessing(props.project.edits.audioProcessing)
  void nextTick(requestDraw)
}, { deep: true, immediate: true })
onMounted(() => {
  resizeObserver = new ResizeObserver(measureViewport)
  if (viewport.value) resizeObserver.observe(viewport.value)
  window.addEventListener('resize', measureViewport)
  document.addEventListener('visibilitychange', visibilityChanged)
  measureViewport()
  if (mic.value || system.value || props.project.edits.music?.length) {
    audio = new PreviewAudio(mic.value, system.value, message => { if (!disposed) emit('error', message) })
    audio.setVolumes(props.project.edits.microphoneVolume, props.project.edits.systemVolume)
    audio.setProcessing(props.project.edits.audioProcessing)
    audio.setMonitor(props.monitor)
    syncMusicTracks()
    void audio.prepare().catch(error => { if (!disposed) emit('error', String(error)) })
  }
})
watch(() => props.project.edits.music?.length, () => {
  if (disposed) return
  if (!audio && props.project.edits.music?.length) { audio = new PreviewAudio(mic.value, system.value, message => { if (!disposed) emit('error', message) }); audio.setVolumes(props.project.edits.microphoneVolume, props.project.edits.systemVolume); audio.setProcessing(props.project.edits.audioProcessing); audio.setMonitor(props.monitor); void audio.prepare().catch(error => { if (!disposed) emit('error', String(error)) }) }
})
let releasePromise:Promise<void> | undefined
function release():Promise<void> {
  if (releasePromise) return releasePromise
  finishGesture(false); cancelDrawing(); disposed = true; released.value = true; ready.value = false
  resizeObserver?.disconnect(); window.removeEventListener('resize',measureViewport)
  document.removeEventListener('visibilitychange', visibilityChanged); scheduler.dispose(); presentedTimes.clear(); verifiedPausedTimes.clear()
  clearTimeout(subtitleTimer); clearTimeout(subtitleRetryTimer); pause(); audio?.dispose(); audio = null
  subtitleImages.clear(); for (const image of loadingImages) { image.src = ''; image.onerror?.(new Event('error')) }; loadingImages.clear()
  subtitleLoading.value = false; clearFrame(); videoBackground.release(); redactedFrame.width = 0; blurPatch.width = 0
  const media = [video.value,camera.value,mic.value,system.value,...musicElements.values()].filter((element):element is HTMLMediaElement => !!element)
  for (const element of media) { element.pause(); element.srcObject = null; element.removeAttribute('src'); element.load() }
  releasePromise = (async () => {
    await nextTick()
    await Promise.all(media.map(element => new Promise<void>((resolve,reject) => {
      const unloaded = () => element.readyState === 0 && (element.networkState === 0 || element.networkState === 3) && !element.hasAttribute('src')
      if (unloaded()) { resolve(); return }
      const timer = setTimeout(() => { element.removeEventListener('emptied',empty); if (unloaded()) resolve(); else reject(new Error('Preview media is still in use. Retry deleting the project.')) },1500)
      const empty = () => { if (unloaded()) { clearTimeout(timer); element.removeEventListener('emptied',empty); resolve() } }
      element.addEventListener('emptied',empty,{ once:true }); element.load()
    })))
  })()
  return releasePromise
}
onBeforeUnmount(() => { void release().catch(() => undefined) })
defineExpose({ play, pause, pauseAtFrame, seek, toggle, release, hideOriginal })
</script>

<template>
  <div class="preview-host">
    <div class="preview-tools">
      <label :for="scaleId">Preview scale</label>
      <select :id="scaleId" v-model="previewScale" :aria-describedby="viewportHint">
        <option value="fit">Fit</option><option value="100">100%</option><option value="200">200%</option>
      </select>
      <span :id="viewportHint" class="preview-scale-hint">100% uses 1080p canvas</span>
      <template v-if="portraitGuide && !originalVisible">
        <label :for="guideId" title="Approximate space for app controls and captions">Safe area</label>
        <select :id="guideId" v-model="guide" class="safe-area-choice" :disabled="disabled" aria-description="Approximate platform layout. This guide appears only in the preview.">
          <option value="off">Off</option><option value="tiktok">TikTok</option><option value="reels">Instagram Reels</option>
        </select>
      </template>
      <span v-if="cameraStatus" class="camera-availability" role="status">{{ cameraStatus }}</span>
      <button type="button" :disabled="!canPan" @click="centerViewport">Center</button>
    </div>
    <div ref="viewport" class="preview-viewport" :class="{ 'preview-fit': previewScale === 'fit' }" tabindex="0" role="region" aria-label="Video preview viewport" :aria-description="canPan ? 'Scroll or use arrow keys to pan. Home and End move to the corners.' : 'Video preview fits in the viewport.'" @keydown="panWithKeyboard">
      <div class="preview-stage" :style="{ width: `${Math.max(viewportLayout.cssWidth, viewportLayout.viewportWidth)}px`, height: `${Math.max(viewportLayout.cssHeight, viewportLayout.viewportHeight)}px` }">
        <div class="preview-canvas-wrap" :style="{ width:`${viewportLayout.cssWidth}px`,height:`${viewportLayout.cssHeight}px` }">
          <canvas ref="canvas" class="preview-canvas" :class="{ drawing: drawing && !disabled, manipulating:editingGesture }" :style="{ width: `${viewportLayout.cssWidth}px`, height: `${viewportLayout.cssHeight}px` }" tabindex="0" aria-label="Edited video preview" :aria-description="canvasHelp" @keydown="editKeys($event)" @keyup="finishKeys" @blur="gesture?.pointerId === null && finishGesture(true)" @pointerdown="drawStart" @pointermove="drawMove" @pointerup="drawEnd" @pointercancel="abortDrawing($event.pointerId)" @lostpointercapture="abortDrawing($event.pointerId)" />
          <SafeAreaGuide v-if="portraitGuide && guide !== 'off' && !originalVisible && !released && ready" :guide="guide" />
          <div v-if="controlBounds && controlTarget && !drawing && !disabled && !released" class="preview-selection-layer" :style="controlTarget === 'region' && selectionGeometry ? { clipPath:`inset(${selectionGeometry.fy*viewportLayout.scale}px ${(selectionGeometry.width-selectionGeometry.fx-selectionGeometry.fw)*viewportLayout.scale}px ${(selectionGeometry.height-selectionGeometry.fy-selectionGeometry.fh)*viewportLayout.scale}px ${selectionGeometry.fx*viewportLayout.scale}px)` } : {}">
            <div class="preview-selection" :style="selectionStyle(controlBounds)">
              <button type="button" class="selection-move" :aria-label="controlTarget === 'camera' ? 'Move camera' : 'Move mask'" aria-description="Drag to move. Arrow keys move one pixel; hold Shift for ten pixels." @pointerdown="startPointer($event,controlTarget,'move')" @keydown="editKeys($event,controlTarget)" @keyup="finishKeys" @blur="gesture?.pointerId === null && finishGesture(true)" />
              <button v-for="corner in corners" :key="corner" type="button" class="selection-corner" :class="`corner-${corner}`" :aria-label="`Resize ${controlTarget === 'camera' ? 'camera' : 'mask'} ${cornerName[corner]}`" aria-description="Drag to resize. Arrow keys move this corner; hold Shift for ten pixels." @pointerdown="startPointer($event,controlTarget,corner)" @keydown="editKeys($event,controlTarget,corner)" @keyup="finishKeys" @blur="gesture?.pointerId === null && finishGesture(true)" />
            </div>
          </div>
        </div>
      </div>
    </div>
    <div v-if="screenLoadFailed" class="preview-loading" role="status">Video unavailable. Press Play to retry.</div>
    <div v-else-if="!ready || seeking" class="preview-loading" role="status">{{ ready ? 'Seeking edited video...' : 'Loading video...' }}</div>
    <div v-if="subtitleLoading" class="subtitle-loading" role="status">{{ project.edits.subtitleStyle.highlightWords ? 'Preparing word highlight...' : 'Rendering subtitle preview...' }}</div>
    <video ref="video" class="media-source" :src="released ? undefined : mediaUrl(project.screenPath)" crossorigin="anonymous" muted playsinline preload="auto" aria-hidden="true" tabindex="-1" data-track="screen recording" @durationchange="readScreenDuration" @loadeddata="loaded" @seeking="clearFrame" @seeked="requestDraw" @error="mediaError" @ended="ended" />
    <video v-if="project.cameraPath" ref="camera" class="media-source" :src="released ? undefined : mediaUrl(project.cameraPath)" crossorigin="anonymous" muted playsinline preload="auto" aria-hidden="true" tabindex="-1" data-track="webcam recording" @loadeddata="auxiliaryLoaded" @seeked="requestDraw" @error="mediaError" />
    <audio v-if="project.microphonePath" ref="mic" crossorigin="anonymous" :src="released ? undefined : mediaUrl(project.microphonePath)" muted preload="auto" aria-hidden="true" tabindex="-1" data-track="microphone recording" @loadedmetadata="auxiliaryLoaded" @error="mediaError" />
    <audio v-if="project.systemAudioPath" ref="system" crossorigin="anonymous" :src="released ? undefined : mediaUrl(project.systemAudioPath)" muted preload="auto" aria-hidden="true" tabindex="-1" data-track="system audio" @loadedmetadata="auxiliaryLoaded" @error="mediaError" />
    <audio v-for="clip in project.edits.music" :key="clip.id" :ref="value => registerMusic(clip.id, value)" crossorigin="anonymous" :src="released ? undefined : mediaUrl(clip.path)" muted preload="metadata" aria-hidden="true" tabindex="-1" :data-track="`music: ${clip.name}`" @loadedmetadata="auxiliaryLoaded" @error="mediaError" />
  </div>
</template>

<style scoped>
.preview-host { flex-direction: column; align-items: stretch; justify-content: flex-start; gap: 8px; }
.preview-tools { display: flex; flex-wrap: wrap; align-items: center; gap: 8px; flex-shrink: 0; font-size: 12px; }
.preview-tools label { margin: 0; }
.preview-tools select { width: 82px; min-height: 30px; padding: 4px 8px; }
.preview-tools .safe-area-choice { width: 126px; }
.preview-tools button { min-height: 30px; padding: 4px 10px; margin-left: auto; }
.preview-scale-hint { color: var(--muted); }
.preview-viewport { flex: 1; min-height: 0; width: 100%; overflow: auto; overscroll-behavior: contain; }
.preview-viewport.preview-fit { overflow: hidden; }
.preview-viewport:focus-visible { outline: 2px solid var(--focus); outline-offset: 2px; }
.preview-stage { display: flex; align-items: center; justify-content: center; }
.preview-canvas-wrap { position: relative; flex: none; }
.preview-canvas { flex: none; max-width: none; max-height: none; object-fit: fill; }
.preview-canvas:focus-visible { outline-offset:-3px; }
.preview-canvas.manipulating { cursor: grabbing; touch-action: none; }
.preview-selection-layer { position:absolute; inset:0; overflow:hidden; pointer-events:none; }
.preview-selection { position:absolute; }
.selection-move, .selection-corner { position:absolute; pointer-events:auto; }
.selection-move { inset:0; width:100%; height:100%; min-height:0; padding:0; border:2px solid #e9b451; border-radius:0; cursor:move; background:transparent; }
.selection-move:hover:enabled { background:transparent; }
.selection-corner { width:24px; height:24px; min-height:24px; padding:0; border:2px solid var(--focus); border-radius:3px; background:var(--panel); transform:translate(-50%,-50%); }
.selection-corner:active:enabled { transform:translate(-50%,-50%); }
.corner-nw { left:0; top:0; cursor:nwse-resize; }.corner-ne { left:100%; top:0; cursor:nesw-resize; }
.corner-sw { left:0; top:100%; cursor:nesw-resize; }.corner-se { left:100%; top:100%; cursor:nwse-resize; }
.preview-loading { left: 50%; top: 50%; transform: translate(-50%, -50%); pointer-events: none; }
@media (max-width: 600px) { .preview-scale-hint { flex-basis: 100%; order: 1; } }
</style>
