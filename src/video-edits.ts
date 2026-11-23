import type { EditSettings, VideoClip } from './types'

export const FULL_VIDEO_ID = 'full-recording'
const EPSILON = 1e-9
const MINIMUM = .1
const MAX_CLIPS = 512
export interface KeptClip extends VideoClip { outputStart: number; outputEnd: number }
export type VideoEdits = Pick<EditSettings, 'trimStart' | 'trimEnd' | 'videoClips'>

export function keptClips(edits: VideoEdits): KeptClip[] {
  let output = 0
  return (edits.videoClips?.length ? edits.videoClips : [{ id: FULL_VIDEO_ID, start: edits.trimStart, end: edits.trimEnd }]).flatMap(clip => {
    const start = Math.max(clip.start, edits.trimStart); const end = Math.min(clip.end, edits.trimEnd)
    if (end <= start) return []
    const outputStart = output; output += end - start
    return [{ id: clip.id, start, end, outputStart, outputEnd: output }]
  })
}
export function outputDuration(edits: VideoEdits): number { return keptClips(edits).at(-1)?.outputEnd ?? 0 }
export function sourceToOutput(edits: VideoEdits, source: number): number {
  for (const clip of keptClips(edits)) {
    if (source < clip.start) return clip.outputStart
    if (source < clip.end) return clip.outputStart + source - clip.start
  }
  return outputDuration(edits)
}
export function outputToSource(edits: VideoEdits, output: number): number {
  const clips = keptClips(edits)
  if (!clips.length) return edits.trimStart
  for (const clip of clips) if (output < clip.outputEnd) return clip.start + Math.max(0, output - clip.outputStart)
  return clips.at(-1)!.end
}
export function retainedSource(edits: VideoEdits, source: number): number { return outputToSource(edits, sourceToOutput(edits, source)) }
export function projectedRange(edits: VideoEdits, start: number, end: number) {
  return keptClips(edits).flatMap(clip => {
    const sourceStart = Math.max(start, clip.start); const sourceEnd = Math.min(end, clip.end)
    return sourceEnd > sourceStart ? [{ clipId: clip.id, sourceStart, sourceEnd, start: clip.outputStart + sourceStart - clip.start, end: clip.outputStart + sourceEnd - clip.start }] : []
  })
}
export function playbackRanges(edits: VideoEdits): VideoClip[] {
  const ranges: VideoClip[] = []
  for (const clip of keptClips(edits)) {
    const previous = ranges.at(-1)
    if (previous && previous.end === clip.start) previous.end = clip.end
    else ranges.push({ id: clip.id, start: clip.start, end: clip.end })
  }
  return ranges
}
export function subtitleRetained(edits: VideoEdits, cue: { start: number; end: number }): boolean {
  if (!edits.videoClips?.length) return true
  const trimmed = Math.max(0, Math.min(cue.end, edits.trimEnd) - Math.max(cue.start, edits.trimStart))
  const kept = projectedRange(edits, cue.start, cue.end).reduce((sum, part) => sum + part.end - part.start, 0)
  return trimmed > 0 && kept >= trimmed - EPSILON
}
function materialized(edits: VideoEdits): VideoClip[] {
  return edits.videoClips.length ? edits.videoClips.map(clip => ({ ...clip })) : [{ id: crypto.randomUUID(), start: edits.trimStart, end: edits.trimEnd }]
}
function check(edits: VideoEdits, clips: VideoClip[]): string | null {
  if (!clips.length || outputDuration({ ...edits, videoClips: clips }) < MINIMUM - EPSILON) return 'Keep at least 0.1 seconds of video.'
  if (clips.length > MAX_CLIPS) return 'A project can contain at most 512 video clips.'
  if (clips.some(clip => clip.end - clip.start < MINIMUM - EPSILON)) return 'Each video clip must keep at least 0.1 seconds. Adjust the cut boundary.'
  return null
}
export function splitVideo(edits: VideoEdits, source: number): { clips: VideoClip[]; selected: string } | string {
  const visible = keptClips(edits).find(clip => source > clip.start + EPSILON && source < clip.end - EPSILON)
  if (!visible) return 'Move the playhead inside a video clip to split it.'
  const clips = materialized(edits); const index = edits.videoClips.length ? clips.findIndex(clip => clip.id === visible.id) : 0
  const clip = clips[index]; const right = { id: crypto.randomUUID(), start: source, end: clip.end }
  clips.splice(index, 1, { ...clip, end: source }, right)
  return check(edits, clips) ?? { clips, selected: right.id }
}
export function removeVideo(edits: VideoEdits, id: string): VideoClip[] | string {
  if (!edits.videoClips.length) return 'Keep at least one video clip.'
  const clips = edits.videoClips.filter(clip => clip.id !== id)
  return check(edits, clips) ?? clips
}
export function cutVideoRange(edits: VideoEdits, outputStart: number, outputEnd: number): VideoClip[] | string {
  const duration = outputDuration(edits)
  if (![outputStart, outputEnd].every(Number.isFinite) || outputStart < 0 || outputEnd > duration + EPSILON || outputEnd <= outputStart + EPSILON) return 'Choose a non-empty range within the edited video.'
  const start = outputToSource(edits, outputStart); const end = outputToSource(edits, outputEnd)
  const clips = materialized(edits).flatMap(clip => {
    if (clip.end <= start || clip.start >= end) return [clip]
    const pieces: VideoClip[] = []
    if (clip.start < start) pieces.push({ ...clip, end: start })
    if (clip.end > end) pieces.push({ id: pieces.length ? crypto.randomUUID() : clip.id, start: end, end: clip.end })
    return pieces
  })
  return check(edits, clips) ?? clips
}
export function changeVideoBoundary(edits: VideoEdits, id: string, field: 'start' | 'end', value: number, duration: number): string | null {
  if (!Number.isFinite(value)) return 'Enter a valid source time.'
  if (!edits.videoClips.length) return changeSourceTrim(edits, field === 'start' ? 'trimStart' : 'trimEnd', value, duration)
  const clips = materialized(edits)
  const index = edits.videoClips.length ? clips.findIndex(clip => clip.id === id) : 0
  if (index < 0) return 'Select a video clip first.'
  const clip = clips[index]
  if (field === 'start') clip.start = Math.min(clip.end - MINIMUM, Math.max(clips[index - 1]?.end ?? 0, value))
  else clip.end = Math.max(clip.start + MINIMUM, Math.min(clips[index + 1]?.start ?? duration, value))
  const error = check(edits, clips)
  if (!error) edits.videoClips = clips
  return error
}
export function changeSourceTrim(edits: VideoEdits, field: 'trimStart' | 'trimEnd', value: number, duration: number): string | null {
  if (!Number.isFinite(value)) return 'Enter a valid source time.'
  const candidate = { ...edits, [field]: field === 'trimStart' ? Math.max(0, Math.min(value, edits.trimEnd - MINIMUM)) : Math.min(duration, Math.max(value, edits.trimStart + MINIMUM)) }
  if (outputDuration(candidate) < MINIMUM - EPSILON) return 'This trim removes all kept video. Keep at least 0.1 seconds.'
  edits[field] = candidate[field]; return null
}
