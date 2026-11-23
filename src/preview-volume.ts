import { clamp } from './editing'
import { volumeAt } from './volume-keyframes'
import type { VolumeKeyframe } from './types'

export const VOLUME_BUSES = ['microphone', 'system', 'music'] as const

export function sameVolumeKeys(left: readonly VolumeKeyframe[], right: readonly VolumeKeyframe[]) {
  return left.length === right.length && left.every((key, index) => key.time === right[index].time && key.volume === right[index].volume)
}

export function scheduleVolume(param: AudioParam, keys: readonly VolumeKeyframe[], sourceTime: number, sourceEnd: number, contextTime: number) {
  param.cancelScheduledValues(contextTime)
  param.setValueAtTime(clamp(volumeAt(keys, sourceTime), 0, 1), contextTime)
  if (!keys.length || sourceEnd <= sourceTime) return
  for (const key of keys) {
    if (key.time <= sourceTime) continue
    if (key.time >= sourceEnd) break
    param.linearRampToValueAtTime(clamp(key.volume, 0, 1), contextTime + key.time - sourceTime)
  }
  param.linearRampToValueAtTime(clamp(volumeAt(keys, sourceEnd), 0, 1), contextTime + sourceEnd - sourceTime)
}
