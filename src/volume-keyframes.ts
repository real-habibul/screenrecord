import type { VolumeKeyframe } from './types'
import type { KeptClip } from './video-edits'

export const MAX_VOLUME_KEYS = 2048
const GAP = 0.001
const EPSILON = 1e-9

function firstAtOrAfter(keys: readonly VolumeKeyframe[], time: number) {
  let low = 0
  let high = keys.length
  while (low < high) {
    const middle = (low + high) >>> 1
    if (keys[middle].time < time) low = middle + 1
    else high = middle
  }
  return low
}

export function volumeAt(keys: readonly VolumeKeyframe[], time: number) {
  if (!keys.length) return 1
  const index = firstAtOrAfter(keys, time)
  if (!index) return keys[0].volume
  const left = keys[index - 1]
  const right = keys[index]
  if (!right) return left.volume
  return left.volume + (right.volume - left.volume) * ((time - left.time) / (right.time - left.time))
}

export function insertVolumeKey(keys: VolumeKeyframe[], time: number, volume: number, duration: number) {
  if (![time, volume, duration].every(Number.isFinite) || time < 0 || time > duration || volume < 0 || volume > 1) {
    throw new Error('Choose a time within the recording and a level from 0 to 100%.')
  }
  const index = firstAtOrAfter(keys, time)
  const nearby = [keys[index - 1], keys[index]].filter((key): key is VolumeKeyframe => !!key)
    .filter(key => Math.abs(key.time - time) + EPSILON < GAP)
    .sort((a, b) => Math.abs(a.time - time) - Math.abs(b.time - time))[0]
  if (nearby) return { keys: keys.map(key => key === nearby ? { ...key, volume } : key), selected: nearby.id }
  if (keys.length >= MAX_VOLUME_KEYS) throw new Error('This track already has 2048 volume keyframes.')
  const key = { id: crypto.randomUUID(), time, volume }
  const next = keys.slice()
  next.splice(index, 0, key)
  return { keys: next, selected: key.id }
}

export function moveVolumeKey(keys: VolumeKeyframe[], id: string, time: number, volume: number, duration: number) {
  const index = keys.findIndex(key => key.id === id)
  if (index < 0 || ![time, volume, duration].every(Number.isFinite)) return keys
  const min = index > 0 ? keys[index - 1].time + GAP : 0
  const max = index + 1 < keys.length ? keys[index + 1].time - GAP : duration
  const next = keys.slice()
  next[index] = { id, time: Math.max(min, Math.min(time, max)), volume: Math.max(0, Math.min(1, volume)) }
  return next
}

export function projectVolumeKeys(keys: VolumeKeyframe[], clips: KeptClip[], duration: number) {
  const paths: string[] = []
  const visible: Array<{ key: VolumeKeyframe; output: number }> = []
  if (duration <= 0) return { paths, visible }
  const point = (output: number, volume: number) => `${100 * output / duration},${100 * (1 - volume)}`
  for (const [i, clip] of clips.entries()) {
    const line = [point(clip.outputStart, volumeAt(keys, clip.start))]
    for (let k = firstAtOrAfter(keys, clip.start); k < keys.length; k++) {
      const key = keys[k]
      if (key.time > clip.end || (key.time === clip.end && i + 1 < clips.length)) break
      const output = clip.outputStart + key.time - clip.start
      visible.push({ key, output })
      line.push(point(output, key.volume))
    }
    line.push(point(clip.outputEnd, volumeAt(keys, clip.end)))
    paths.push(line.join(' '))
  }
  return { paths, visible }
}
