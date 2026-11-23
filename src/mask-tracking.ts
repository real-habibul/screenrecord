import type { MaskKeyframe, MaskTracking, Redaction } from './types'
import type { Rectangle } from './preview-manipulation'

export const fullMask = (): Rectangle => ({ x: 0, y: 0, width: 1, height: 1 })
const rectangle = (value: Rectangle): Rectangle => ({ x: value.x, y: value.y, width: value.width, height: value.height })
export function maskAt(region: Redaction, time: number): Rectangle & { lost: boolean } {
  const frames = region.tracking?.keyframes
  if (!frames?.length) return { ...rectangle(region), lost: false }
  if (!Number.isFinite(time)) return { ...fullMask(), lost: true }
  let low = 0; let high = frames.length
  while (low < high) { const middle = (low + high) >>> 1; if (frames[middle].time <= time) low = middle + 1; else high = middle }
  const left = frames[low - 1]; const right = frames[low]
  if (!left || left.kind === 'lost' || right?.kind === 'lost') return { ...fullMask(), lost: true }
  if (!right || right.kind === 'manual') return { ...rectangle(left), lost: false }
  const x = Math.min(left.x, right.x); const y = Math.min(left.y, right.y)
  return { x, y, width: Math.max(left.x + left.width, right.x + right.width) - x, height: Math.max(left.y + left.height, right.y + right.height) - y, lost: false }
}
export function manualCorrection(region: Redaction, time: number, value: Rectangle): Redaction {
  const keyframes = region.tracking?.keyframes.map(frame => ({ ...frame })) ?? [{ ...rectangle(region), time: region.start, confidence: 1, kind: 'manual' as const }]
  const frame: MaskKeyframe = { ...rectangle(value), time: Math.max(region.start, Math.min(region.end, time)), confidence: 1, kind: 'manual' }
  const existing = keyframes.findIndex(key => Math.abs(key.time - frame.time) < 1e-9)
  if (existing >= 0) { frame.time = keyframes[existing].time; keyframes[existing] = frame } else keyframes.push(frame)
  return { ...region, tracking: { keyframes: keyframes.sort((a, b) => a.time - b.time), reviewed: false } }
}
function boundary(region: Redaction, time: number): MaskKeyframe {
  const exact = region.tracking?.keyframes.find(frame => frame.time === time)
  if (exact) return { ...exact }
  const value = maskAt(region, time)
  const frames = region.tracking?.keyframes ?? []
  const next = frames.findIndex(frame => frame.time > time)
  const left = frames[(next < 0 ? frames.length : next) - 1]; const right = frames[next]
  const confidence = value.lost ? 0 : right?.kind === 'auto' ? Math.min(left?.confidence ?? 0, right.confidence) : left?.confidence ?? 0
  return { ...rectangle(value), time, confidence, kind: value.lost ? 'lost' : 'auto' }
}
export function retimeMask(region: Redaction, start: number, end: number, move = false): Redaction | string {
  if (!region.tracking) return { ...region, start, end }
  if (end - start > 300 + 1e-9) return 'Tracked regions cannot exceed 300 seconds. Clear tracking before extending this range.'
  let keyframes: MaskKeyframe[]
  if (move) { const delta = start - region.start; keyframes = region.tracking.keyframes.map(frame => ({ ...frame, time: frame.time + delta })) }
  else {
    keyframes = [boundary(region, start), ...region.tracking.keyframes.filter(frame => frame.time > start && frame.time < Math.min(end, region.end)).map(frame => ({ ...frame }))]
    if (end > region.end) { const time = Math.max(start, region.end); if (keyframes.at(-1)?.time === time) keyframes.pop(); keyframes.push({ ...fullMask(), time, confidence: 0, kind: 'lost' }) }
    else keyframes.push(boundary(region, end))
    keyframes = keyframes.filter((frame, index) => !index || frame.time !== keyframes[index - 1].time)
  }
  let lost = false
  keyframes = keyframes.filter(frame => { if (frame.kind === 'lost') lost = true; else if (frame.kind === 'manual') lost = false; return !lost || frame.kind === 'lost' })
  if (keyframes.length > 3002) return 'This region already has the maximum number of tracking keyframes.'
  return { ...region, start, end, tracking: { keyframes, reviewed: false } }
}
export function deleteMaskKeyframe(region: Redaction, time: number): Redaction | string {
  const frames = region.tracking?.keyframes
  if (!frames?.length) return region
  const index = frames.findIndex(frame => frame.time === time)
  if (index < 0) return region
  if (index === 0) return 'The first keyframe anchors the region start and cannot be deleted.'
  if (frames[index].kind === 'lost') return 'Keep the lost marker. Add a manual correction where the target is visible again.'
  const next = frames.filter((_, i) => i !== index)
  let lost = false
  for (const frame of next) { if (frame.kind === 'lost') lost = true; else if (frame.kind === 'manual') lost = false; else if (lost) return 'This correction is needed after tracking was lost. Retrack from another correction or clear tracking first.' }
  return { ...region, tracking: { keyframes: next.map(frame => ({ ...frame })), reviewed: false } }
}
export function maskFingerprint(region: Redaction): string { return JSON.stringify([region.start, region.end, region.x, region.y, region.width, region.height, region.mode, region.strength, region.tracking?.keyframes]) }
export function trackingBudgetError(regions: Redaction[], candidate: Redaction): string | null {
  const count = candidate.tracking?.keyframes.length ?? 0
  const total = regions.reduce((sum, region) => sum + (region.id === candidate.id ? count : region.tracking?.keyframes.length ?? 0), 0)
  return count > 3002 || total > 20_000 ? 'Tracking supports up to 3,002 keyframes per region and 20,000 per project. Clear another track before adding keyframes.' : null
}
export function maskPixels(value: Rectangle, width: number, height: number): Rectangle {
  const x = Math.floor(value.x * width); const y = Math.floor(value.y * height)
  return { x, y, width: Math.min(width, Math.ceil((value.x + value.width) * width)) - x, height: Math.min(height, Math.ceil((value.y + value.height) * height)) - y }
}
const patchSizes = new WeakMap<MaskTracking, Map<string, { width: number; height: number }>>()
export function trackedBlurPatch(region: Redaction, time: number, width: number, height: number): Rectangle {
  const actual = maskPixels(maskAt(region, time), width, height); const tracking = region.tracking
  if (!tracking) return actual
  let entries = patchSizes.get(tracking); if (!entries) { entries = new Map(); patchSizes.set(tracking, entries) }
  const key = `${width}:${height}`; let size = entries.get(key)
  if (!size) {
    size = { width: 1, height: 1 }
    for (const frame of tracking.keyframes) { const value = maskAt(region, frame.time); if (!value.lost) { const pixels = maskPixels(value, width, height); size.width = Math.max(size.width, pixels.width); size.height = Math.max(size.height, pixels.height) } }
    entries.set(key, size)
  }
  return { x: Math.min(actual.x, width - size.width), y: Math.min(actual.y, height - size.height), ...size }
}
