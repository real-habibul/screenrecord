import type { CursorSample, EditSettings, Project, ZoomSegment } from './types'
import { reframeAt } from './reframe'

export const clamp = (value: number, min = 0, max = 1) => Math.max(min, Math.min(max, value))
export const TIME_EPSILON = 1e-9
export const hasMinimumDuration = (start: number, end: number) => end - start + TIME_EPSILON >= 0.1
export function timeLabel(seconds: number, precision = false): string {
  const safe = Math.max(0, Number.isFinite(seconds) ? seconds : 0)
  const hours = Math.floor(safe / 3600)
  const minutes = Math.floor(safe / 60) % 60
  const sec = Math.floor(safe % 60).toString().padStart(2, '0')
  const base = hours ? `${hours}:${minutes.toString().padStart(2, '0')}:${sec}` : `${minutes}:${sec}`
  return precision ? `${base}.${Math.floor(safe * 10) % 10}` : base
}
export function cursorAt(samples: CursorSample[], time: number): CursorSample | null {
  if (!samples.length) return null
  if (time < samples[0].time) return { time, x: .5, y: .5, pressed: false, visible: false }
  let lo = 0; let hi = samples.length - 1
  while (lo < hi) { const mid = Math.ceil((lo + hi) / 2); if (samples[mid].time <= time) lo = mid; else hi = mid - 1 }
  const a = samples[lo]; const b = samples[lo + 1]
  if (!b || b.time <= a.time || b.time - a.time >= .5 || !a.visible || !b.visible) return a
  const t = clamp((time - a.time) / (b.time - a.time))
  return { ...a, x: a.x + (b.x - a.x) * t, y: a.y + (b.y - a.y) * t }
}
export function smoothCursor(samples: CursorSample[], smoothing: number): CursorSample[] {
  if (!samples.length || smoothing <= 0) return samples
  const result: CursorSample[] = [{ ...samples[0] }]
  const tau = Math.max(0.001, smoothing * 0.12)
  for (let i = 1; i < samples.length; i++) {
    const previous = result[i - 1]; const sample = samples[i]
    const dt = Math.max(0, sample.time - previous.time)
    if (dt >= .5 || !sample.visible || !previous.visible) { result.push({ ...sample }); continue }
    const alpha = 1 - Math.exp(-dt / tau)
    result.push({ ...sample, x: previous.x + alpha * (sample.x - previous.x), y: previous.y + alpha * (sample.y - previous.y) })
  }
  return result
}
export function zoomAt(zooms: ZoomSegment[], time: number, cursor: CursorSample | null) {
  const segment = zooms.find(z => time >= z.start && time < z.end)
  if (!segment) return { scale: 1, x: 0.5, y: 0.5, weight: 0 }
  const ramp = Math.max(.001, Math.min(0.35, (segment.end - segment.start) / 2))
  const phase = clamp(Math.min((time - segment.start) / ramp, (segment.end - time) / ramp))
  const ease = phase * phase * (3 - 2 * phase)
  const scale = 1 + (segment.scale - 1) * ease
  return { scale, x: segment.followCursor && cursor?.visible ? cursor.x : segment.x, y: segment.followCursor && cursor?.visible ? cursor.y : segment.y, weight: segment.scale > 1 ? clamp((scale - 1) / (segment.scale - 1)) : 0 }
}
export const even = (value: number) => Math.max(1, Math.floor(Math.max(2, Math.round(value)) / 2)) * 2
export function compositionLayout(project: Project) {
  const ratio = aspectRatio(project)
  const width = even(1080 * Math.max(1, ratio)); const height = even(1080 * Math.max(1, 1 / ratio))
  const innerWidth = even(width - 2 * project.edits.padding); const innerHeight = even(height - 2 * project.edits.padding)
  const inner = { x: (width - innerWidth) / 2, y: (height - innerHeight) / 2, width: innerWidth, height: innerHeight }
  const layout = project.edits.camera.layout ?? 'bubble'
  if (layout === 'fullscreen') return { width, height, layout, screenVisible: false, screenPane: inner, cameraPane: inner }
  if (layout !== 'split') return { width, height, layout, screenVisible: true, screenPane: inner, cameraPane: null }
  const gap = 2 * Math.floor(Math.round(Math.min(project.edits.padding, 24)) / 2)
  if (width >= height) {
    const usable = innerWidth - gap; const cameraWidth = even(usable * project.edits.camera.width); const screenWidth = usable - cameraWidth
    return { width, height, layout, screenVisible: true, screenPane: { ...inner, width: screenWidth }, cameraPane: { ...inner, x: inner.x + screenWidth + gap, width: cameraWidth } }
  }
  const usable = innerHeight - gap; const cameraHeight = even(usable * project.edits.camera.width); const screenHeight = usable - cameraHeight
  return { width, height, layout, screenVisible: true, screenPane: { ...inner, height: screenHeight }, cameraPane: { ...inner, y: inner.y + screenHeight + gap, height: cameraHeight } }
}
export function previewGeometry(project: Project, time: number, cursor: CursorSample | null) {
  const crop = project.edits.crop ?? { x: 0, y: 0, width: 1, height: 1 }
  const composition = compositionLayout(project)
  const { width, height, screenPane } = composition
  const bubble = composition.layout === 'bubble'
  const fit = Math.min((bubble ? Math.max(2, width - 2 * project.edits.padding) : screenPane.width) / (project.width * crop.width), (bubble ? Math.max(2, height - 2 * project.edits.padding) : screenPane.height) / (project.height * crop.height))
  const fw = Math.min(bubble ? width : screenPane.width, even(project.width * crop.width * fit)); const fh = Math.min(bubble ? height : screenPane.height, even(project.height * crop.height * fit))
  const zoom = zoomAt(project.edits.zooms, time, cursor)
  const manual = reframeAt(project.edits.reframeKeyframes ?? [], time)
  const scale = zoom.scale * (manual?.scale ?? 1)
  const bx = Math.floor(crop.x * project.width); const by = Math.floor(crop.y * project.height)
  const bw = Math.min(even(crop.width * project.width), project.width - bx); const bh = Math.min(even(crop.height * project.height), project.height - by)
  if (project.edits.fitMode === 'fill') {
    const ratio = screenPane.width / screenPane.height
    const coverWidth = Math.min(even(Math.min(bw, bh * ratio)), bw); const coverHeight = Math.min(even(Math.min(bh, bw / ratio)), bh)
    const sw = Math.min(even(coverWidth / scale), coverWidth); const sh = Math.min(even(coverHeight / scale), coverHeight)
    const baseX = (bx + bw / 2) / project.width; const baseY = (by + bh / 2) / project.height
    const centerX = manual?.x ?? baseX + (zoom.x - baseX) * zoom.weight; const centerY = manual?.y ?? baseY + (zoom.y - baseY) * zoom.weight
    const sx = Math.round(clamp(centerX * project.width - sw / 2, bx, bx + bw - sw)); const sy = Math.round(clamp(centerY * project.height - sh / 2, by, by + bh - sh))
    return { ...composition, width, height, fw: screenPane.width, fh: screenPane.height, fx: screenPane.x, fy: screenPane.y, sw, sh, sx, sy }
  }
  const sw = Math.min(even(bw / scale), bw); const sh = Math.min(even(bh / scale), bh)
  const sx = Math.round(clamp((manual?.x ?? zoom.x) * project.width - sw / 2, bx, bx + bw - sw)); const sy = Math.round(clamp((manual?.y ?? zoom.y) * project.height - sh / 2, by, by + bh - sh))
  return { ...composition, width, height, fw, fh, fx: bubble ? (width - fw) / 2 : screenPane.x + Math.floor((screenPane.width - fw) / 2), fy: bubble ? (height - fh) / 2 : screenPane.y + Math.floor((screenPane.height - fh) / 2), sw, sh, sx, sy }
}
export function aspectRatio(project: Project): number {
  const value = project.edits.aspectRatio
  if (value === '16:9') return 16 / 9
  if (value === '9:16') return 9 / 16
  if (value === '1:1') return 1
  const crop = project.edits.crop
  return (project.width * (crop?.width ?? 1)) / (project.height * (crop?.height ?? 1))
}
export function normalizeEdits(edits: EditSettings, duration: number): EditSettings {
  const copy: EditSettings = JSON.parse(JSON.stringify(edits))
  const start = Number(copy.trimStart); const end = Number(copy.trimEnd)
  copy.trimStart = clamp(Number.isFinite(start) ? start : 0, 0, Math.max(0, duration - 0.1))
  copy.trimEnd = clamp(Number.isFinite(end) ? end : duration, Math.min(duration, copy.trimStart + 0.1), duration)
  copy.zooms = copy.zooms.filter(z => Number.isFinite(z.start) && Number.isFinite(z.end)).map(z => ({ ...z, start: clamp(z.start, 0, duration), end: clamp(z.end, 0, duration), scale: clamp(z.scale, 1, 3), x: clamp(z.x), y: clamp(z.y) })).filter(z => hasMinimumDuration(z.start, z.end)).sort((a, b) => a.start - b.start)
  if (copy.crop) {
    copy.crop.x = clamp(copy.crop.x, 0, 0.95); copy.crop.y = clamp(copy.crop.y, 0, 0.95)
    copy.crop.width = clamp(copy.crop.width, 0.05, 1 - copy.crop.x); copy.crop.height = clamp(copy.crop.height, 0.05, 1 - copy.crop.y)
  }
  return copy
}
export function addZoom(edits: EditSettings, time: number, duration: number): ZoomSegment | null {
  const sorted = [...edits.zooms].sort((a, b) => a.start - b.start)
  let start = clamp(time, edits.trimStart, Math.max(edits.trimStart, edits.trimEnd - 0.1))
  for (const zoom of sorted) if (start >= zoom.start && start < zoom.end) start = zoom.end
  const next = sorted.find(z => z.start > start)
  const end = Math.min(start + 3, next?.start ?? duration, edits.trimEnd)
  if (!hasMinimumDuration(start, end)) return null
  return { id: crypto.randomUUID(), start, end, scale: 1.8, x: 0.5, y: 0.5, followCursor: true }
}
