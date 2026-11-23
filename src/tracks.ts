import { clamp, TIME_EPSILON } from './editing'
import type { EditSettings, MusicClip, Project, Redaction, SubtitleCue, TrackSelection, ZoomSegment } from './types'
import { retimeCue } from './subtitle-editing'
import { retimeMask } from './mask-tracking'

export function initializeTracks(edits: EditSettings) {
  edits.fitMode ??= 'fit'
  edits.reframeKeyframes ??= []
  edits.backgroundMode ??= 'solid'; edits.backgroundBlur ??= 24
  if (edits.camera) edits.camera.layout ??= 'bubble'
  edits.videoClips ??= []; edits.music ??= []; edits.subtitles ??= []; edits.redactions ??= []
  edits.subtitleStyle ??= { enabled: true, fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: false, highlightColor: '#E9B451' }
  edits.subtitleStyle.highlightWords ??= false; edits.subtitleStyle.highlightColor ??= '#E9B451'
  edits.audioProcessing ??= { microphoneGain: 1, systemGain: 1, ducking: { enabled: false, amountDb: 12 } }
}

export function musicGain(clip: MusicClip, time: number): number {
  const elapsed = time - clip.start
  if (elapsed < 0 || elapsed >= clip.duration) return 0
  return clip.volume * Math.min(1, clip.fadeIn > 0 ? elapsed / clip.fadeIn : 1, clip.fadeOut > 0 ? (clip.duration - elapsed) / clip.fadeOut : 1)
}

export function updateMusic(clip: MusicClip, field: keyof MusicClip, value: number, duration: number) {
  if (!Number.isFinite(value)) return
  if (field === 'start') clip.start = clamp(value, 0, Math.max(0, duration - clip.duration))
  if (field === 'sourceStart') { clip.sourceStart = clamp(value, 0, Math.max(0, clip.sourceDuration - .1)); clip.duration = Math.min(clip.duration, clip.sourceDuration - clip.sourceStart) }
  if (field === 'duration') clip.duration = clamp(value, .1, Math.min(duration - clip.start, clip.sourceDuration - clip.sourceStart))
  if (field === 'volume') clip.volume = clamp(value, 0, 2)
  if (field === 'fadeIn') clip.fadeIn = clamp(value, 0, clip.duration - clip.fadeOut)
  if (field === 'fadeOut') clip.fadeOut = clamp(value, 0, clip.duration - clip.fadeIn)
  clip.fadeIn = Math.min(clip.fadeIn, clip.duration); clip.fadeOut = Math.min(clip.fadeOut, clip.duration - clip.fadeIn)
}

export function nextCue(project: Project, time: number): SubtitleCue | null {
  const cues = [...project.edits.subtitles].sort((a, b) => a.start - b.start)
  let start = clamp(time, project.edits.trimStart, project.edits.trimEnd)
  for (const cue of cues) if (start >= cue.start && start < cue.end) start = cue.end
  const end = Math.min(start + 3, cues.find(cue => cue.start > start)?.start ?? project.edits.trimEnd, project.edits.trimEnd)
  return end - start + TIME_EPSILON >= .01 ? { id: crypto.randomUUID(), start, end, text: 'Subtitle' } : null
}

export function updateInterval(item: SubtitleCue | Redaction, field: 'start' | 'end', value: number, duration: number, neighbors: SubtitleCue[] = [], minimum = .1) {
  if (!Number.isFinite(value)) return
  if (field === 'start') item.start = clamp(value, Math.max(0, ...neighbors.filter(cue => cue.id !== item.id && cue.end <= item.end).map(cue => cue.end)), item.end - minimum)
  else item.end = clamp(value, item.start + minimum, Math.min(duration, ...neighbors.filter(cue => cue.id !== item.id && cue.start >= item.start).map(cue => cue.start)))
}

export function trackLanes<T extends { start: number; end: number }>(items: T[]): Array<T & { lane: number }> {
  const ends: number[] = []
  return [...items].sort((a, b) => a.start - b.start).map(item => {
    let lane = ends.findIndex(end => end <= item.start)
    if (lane < 0) lane = ends.length
    ends[lane] = item.end
    return { ...item, lane }
  })
}

export function sourceRectangle(region: Pick<Redaction, 'x' | 'y' | 'width' | 'height'>, width: number, height: number) {
  const x = Math.floor(region.x * width); const y = Math.floor(region.y * height)
  return { x, y, width: Math.min(width, Math.ceil((region.x + region.width) * width)) - x, height: Math.min(height, Math.ceil((region.y + region.height) * height)) - y }
}

export function redactionsFit(regions: Redaction[]): boolean {
  const edges = regions.flatMap(item => [{ time: item.start, delta: 1 }, { time: item.end, delta: -1 }]).sort((a, b) => a.time - b.time || a.delta - b.delta)
  let active = 0
  return edges.every(edge => { active += edge.delta; return active <= 16 })
}

export type DragMode = 'move' | 'start' | 'end'
export function dragSourceTrim(edits: EditSettings, original: { start: number; end: number }, mode: 'start' | 'end', delta: number, duration: number) {
  if (mode === 'start') edits.trimStart = clamp(original.start + delta, 0, original.end - .1)
  else edits.trimEnd = clamp(original.end + delta, original.start + .1, duration)
}
export function dragZoom(edits: EditSettings, original: ZoomSegment, mode: DragMode, delta: number, duration: number) {
  const zoom = edits.zooms.find(item => item.id === original.id); if (!zoom) return
  const others = edits.zooms.filter(item => item.id !== original.id)
  const left = Math.max(0, ...others.filter(item => item.end <= original.start + TIME_EPSILON).map(item => item.end))
  const right = Math.min(duration, ...others.filter(item => item.start >= original.end - TIME_EPSILON).map(item => item.start))
  if (mode === 'move') { const change = clamp(delta, left - original.start, right - original.end); zoom.start = original.start + change; zoom.end = original.end + change }
  if (mode === 'start') zoom.start = clamp(original.start + delta, left, original.end - .1)
  if (mode === 'end') zoom.end = clamp(original.end + delta, original.start + .1, right)
}
export function dragTrack(edits: EditSettings, selection: TrackSelection, original: MusicClip | SubtitleCue | Redaction, mode: DragMode, delta: number, duration: number): boolean {
  if (!Number.isFinite(delta)) return false
  if (selection.kind === 'music') {
    const clip = edits.music.find(item => item.id === selection.id); if (!clip) return false
    const before = original as MusicClip; const result = { ...before }
    if (mode === 'move') result.start = clamp(before.start + delta, 0, duration - before.duration)
    if (mode === 'start') {
      const change = clamp(delta, -Math.min(before.start, before.sourceStart), before.duration - .1)
      result.start += change; result.sourceStart += change; result.duration -= change
    }
    if (mode === 'end') result.duration = clamp(before.duration + delta, .1, Math.min(duration - before.start, before.sourceDuration - before.sourceStart))
    result.fadeIn = Math.min(result.fadeIn, result.duration); result.fadeOut = Math.min(result.fadeOut, result.duration - result.fadeIn)
    Object.assign(clip, result); return true
  }
  const items = selection.kind === 'subtitles' ? edits.subtitles : edits.redactions
  const item = items.find(value => value.id === selection.id); if (!item) return false
  const before = original as SubtitleCue | Redaction; const result = { ...before }
  const minimum = selection.kind === 'subtitles' ? .01 : .1
  const others = selection.kind === 'subtitles' ? edits.subtitles.filter(value => value.id !== item.id) : []
  const left = Math.max(0, ...others.filter(value => value.end <= before.start + TIME_EPSILON).map(value => value.end))
  const right = Math.min(duration, ...others.filter(value => value.start >= before.end - TIME_EPSILON).map(value => value.start))
  if (mode === 'move') { const change = clamp(delta, left - before.start, right - before.end); result.start += change; result.end += change }
  if (mode === 'start') result.start = clamp(before.start + delta, left, before.end - minimum)
  if (mode === 'end') result.end = clamp(before.end + delta, before.start + minimum, right)
  if (selection.kind === 'subtitles') { const retimed = retimeCue(before as SubtitleCue, result.start, result.end, mode === 'move'); if (typeof retimed === 'string') return false; Object.assign(result, retimed) }
  if (selection.kind === 'redactions') { const retimed = retimeMask(before as Redaction, result.start, result.end, mode === 'move'); if (typeof retimed === 'string') return false; Object.assign(result, retimed); if (edits.redactions.reduce((sum, region) => sum + (region.id === item.id ? (result as Redaction).tracking?.keyframes.length ?? 0 : region.tracking?.keyframes.length ?? 0), 0) > 20_000) return false }
  if (selection.kind === 'redactions' && !redactionsFit(edits.redactions.map(value => value.id === item.id ? result as Redaction : value))) return false
  Object.assign(item, result); return true
}
