import type { AuditionRange } from './audition'
import type { VideoClip } from './types'
import { keptClips, type VideoEdits } from './video-edits'

export function loopSourceRanges(edits: VideoEdits, range?: AuditionRange | null): VideoClip[] | null {
  if (!range || !Number.isFinite(range.start) || !Number.isFinite(range.end) || range.end <= range.start) return null
  const result: VideoClip[] = []
  for (const clip of keptClips(edits)) {
    const start = Math.max(range.start, clip.outputStart)
    const end = Math.min(range.end, clip.outputEnd)
    if (end <= start) continue
    const sourceStart = start === clip.outputStart ? clip.start : clip.start + (start - clip.outputStart)
    const sourceEnd = end === clip.outputEnd ? clip.end : clip.start + (end - clip.outputStart)
    const previous = result.at(-1)
    if (previous && previous.end === sourceStart) previous.end = sourceEnd
    else result.push({ id: clip.id, start: sourceStart, end: sourceEnd })
  }
  return result.length ? result : null
}

export function loopSourceTarget(ranges: VideoClip[], time: number): number {
  for (const range of ranges) {
    if (time < range.start) return range.start
    if (time < range.end) return time
  }
  return ranges[0]?.start ?? time
}
