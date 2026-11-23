import type { CropSettings, EditSettings, SubtitleCue, SubtitleRange, SubtitleStyle } from './types'
import { cueRanges, mergeRanges } from './subtitle-editing'
import { playbackRanges, type VideoEdits } from './video-edits'

function intersect(left: SubtitleRange[], right: SubtitleRange[]): SubtitleRange[] {
  return mergeRanges(left.flatMap(a => right.flatMap(b => {
    const start = Math.max(a.start, b.start); const end = Math.min(a.end, b.end)
    return end > start ? [{ start, end }] : []
  })))
}
export function projectedSubtitle(edits: VideoEdits, cue: SubtitleCue): SubtitleCue | null {
  const declared = intersect(cueRanges(cue), [{ start: edits.trimStart, end: edits.trimEnd }])
  const coverage = intersect(declared, playbackRanges(edits))
  if (!coverage.length) return null
  const complete = (range: SubtitleRange) => coverage.some(kept => range.start >= kept.start && range.end <= kept.end)
  const words = cue.words?.length ? cue.words.filter(complete).map(word => ({ ...word })) : []
  const length = (ranges: SubtitleRange[]) => ranges.reduce((sum, range) => sum + range.end - range.start, 0)
  if (cue.words?.length ? !words.length : edits.videoClips.length > 0 && length(coverage) + 1e-9 < length(declared)) return null
  const first = words[0]?.start ?? coverage[0].start; const last = words.at(-1)?.end ?? coverage.at(-1)!.end
  const start = Math.max(cue.start, Math.min(first, last - .01)); const end = Math.min(cue.end, Math.max(last, start + .01))
  return { ...cue, start, end, sourceRanges: intersect(coverage, [{ start, end }]), words, text: words.length ? words.map(word => word.text).join(' ') : cue.text }
}
export function subtitleAt(cue: SubtitleCue, time: number): boolean { return cueRanges(cue).some(range => time >= range.start && time < range.end) }
export function activeWord(cue: SubtitleCue, time: number): number | null { const index = cue.words?.findIndex(word => time >= word.start && time < word.end) ?? -1; return index < 0 ? null : index }
export function subtitleRasterKey(cue: SubtitleCue, wordIndex: number | null, style: SubtitleStyle, aspect: EditSettings['aspectRatio'], crop: CropSettings | null): string {
  return JSON.stringify([cue.text, cue.words?.map(word => word.text) ?? [], wordIndex, style, aspect, crop])
}
