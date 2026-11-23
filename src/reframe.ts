import type { ReframeKeyframe } from './types'

export const MAX_REFRAME_KEYS = 3000
export const REFRAME_GAP = .001
export type ReframeSample = Pick<ReframeKeyframe, 'x' | 'y' | 'scale'>

export function reframeAt(keys: ReframeKeyframe[], time: number): ReframeSample | null {
  if (!keys.length || !Number.isFinite(time)) return null
  let low = 0; let high = keys.length
  while (low < high) { const middle = Math.floor((low + high) / 2); if (keys[middle].time <= time) low = middle + 1; else high = middle }
  const left = keys[Math.max(0, low - 1)]; const right = keys[low] ?? left
  const amount = right.time > left.time ? Math.max(0, Math.min(1, (time - left.time) / (right.time - left.time))) : 0
  return { x: left.x + (right.x - left.x) * amount, y: left.y + (right.y - left.y) * amount, scale: left.scale + (right.scale - left.scale) * amount }
}

export function setReframeKey(keys: ReframeKeyframe[], time: number, sample: ReframeSample, duration: number): { keys: ReframeKeyframe[]; selected: string } | string {
  if (![time, duration, sample.x, sample.y, sample.scale].every(Number.isFinite) || time < 0 || time > duration || sample.x < 0 || sample.x > 1 || sample.y < 0 || sample.y > 1 || sample.scale < 1 || sample.scale > 3) return 'Choose a valid frame, focus between 0 and 100%, and scale between 1 and 3.'
  const nearby = keys.reduce<ReframeKeyframe | null>((found, key) => Math.abs(key.time - time) < REFRAME_GAP && (!found || Math.abs(key.time - time) < Math.abs(found.time - time)) ? key : found, null)
  if (!nearby && keys.length >= MAX_REFRAME_KEYS) return 'A project can contain at most 3000 framing keyframes. Delete a keyframe before adding another.'
  const value = { id: nearby?.id ?? crypto.randomUUID(), time: nearby?.time ?? time, ...sample }
  return { keys: [...keys.filter(key => key.id !== value.id), value].sort((a, b) => a.time - b.time), selected: value.id }
}
