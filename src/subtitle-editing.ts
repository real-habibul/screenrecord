import type { SubtitleCue, SubtitleRange } from './types'

const MINIMUM = .01
export function cueRanges(cue: SubtitleCue): SubtitleRange[] { return cue.sourceRanges?.length ? cue.sourceRanges.map(range => ({ ...range })) : [{ start: cue.start, end: cue.end }] }
export function mergeRanges(ranges: SubtitleRange[]): SubtitleRange[] {
  const result: SubtitleRange[] = []
  for (const range of [...ranges].sort((a, b) => a.start - b.start)) {
    const previous = result.at(-1)
    if (previous && range.start <= previous.end) previous.end = Math.max(previous.end, range.end)
    else result.push({ ...range })
  }
  return result
}
function restrict(ranges: SubtitleRange[], start: number, end: number) { return ranges.map(range => ({ start: Math.max(start, range.start), end: Math.min(end, range.end) })).filter(range => range.end > range.start) }
function duration(ranges: SubtitleRange[]) { return ranges.reduce((sum, range) => sum + range.end - range.start, 0) }
function within(ranges: SubtitleRange[], start: number, end: number) { return ranges.some(range => start >= range.start && end <= range.end) }
function fragment(cue: SubtitleCue, ranges: SubtitleRange[], text: string, id: string): SubtitleCue {
  return { ...cue, id, start: ranges[0].start, end: ranges.at(-1)!.end, text, sourceRanges: ranges }
}
export function splitCue(cue: SubtitleCue, time: number, caret?: number): { left: SubtitleCue; right: SubtitleCue; estimated: boolean } | string {
  if (!(time > cue.start && time < cue.end)) return 'Move the playhead inside the selected cue before splitting.'
  const ranges = cueRanges(cue)
  const words = cue.words ?? []
  if (words.length) {
    if (words.length < 2) return 'A timed cue needs at least two words to split.'
    let index = 1
    for (let i = 2; i < words.length; i++) if (Math.abs(words[i].start - time) < Math.abs(words[index].start - time)) index = i
    const boundary = words[index].start
    const leftRanges = restrict(ranges, cue.start, Math.max(boundary, words[index - 1].end)); const rightRanges = restrict(ranges, boundary, cue.end)
    if (duration(leftRanges) + 1e-9 < MINIMUM || duration(rightRanges) + 1e-9 < MINIMUM) return 'Each subtitle needs at least 0.01 seconds.'
    const leftWords = words.slice(0, index).map(word => ({ ...word })); const rightWords = words.slice(index).map(word => ({ ...word }))
    return { left: { ...fragment(cue, leftRanges, leftWords.map(word => word.text).join(' '), cue.id), words: leftWords }, right: { ...fragment(cue, rightRanges, rightWords.map(word => word.text).join(' '), crypto.randomUUID()), words: rightWords }, estimated: false }
  }
  const leftRanges = restrict(ranges, cue.start, time); const rightRanges = restrict(ranges, time, cue.end)
  if (duration(leftRanges) + 1e-9 < MINIMUM || duration(rightRanges) + 1e-9 < MINIMUM) return 'Each subtitle needs at least 0.01 seconds.'
  const explicit = caret !== undefined && caret > 0 && caret < cue.text.length
  let offset = explicit ? caret! : Math.round(cue.text.length * duration(leftRanges) / duration(ranges))
  if (!explicit) {
    const spaces = [...cue.text.matchAll(/\s+/g)].map(match => match.index!).filter(index => cue.text.slice(0, index).trim() && cue.text.slice(index).trim())
    if (spaces.length) offset = spaces.reduce((best, index) => Math.abs(index - offset) < Math.abs(best - offset) ? index : best)
  }
  if (/^[\uDC00-\uDFFF]$/.test(cue.text[offset] ?? '')) offset++
  const left = cue.text.slice(0, offset).trim(); const right = cue.text.slice(offset).trim()
  if (!left || !right) return 'Place the text caret between two non-empty parts of the caption.'
  return { left: { ...fragment(cue, leftRanges, left, cue.id), words: [] }, right: { ...fragment(cue, rightRanges, right, crypto.randomUUID()), words: [] }, estimated: !explicit }
}
export function mergeCues(left: SubtitleCue, right: SubtitleCue): SubtitleCue | string {
  const text = `${left.text.trim()} ${right.text.trim()}`
  if (text.length > 500 || text.split('\n').length > 3) return 'Merged captions must fit within 500 characters and three lines.'
  const ranges = mergeRanges([...cueRanges(left), ...cueRanges(right)])
  if (ranges.length > 512) return 'Merged captions cannot contain more than 512 separate source ranges.'
  const words = left.words?.length && right.words?.length ? [...left.words, ...right.words].map(word => ({ ...word })).sort((a, b) => a.start - b.start || a.end - b.end) : []
  return { ...fragment(left, ranges, words.length ? words.map(word => word.text).join(' ') : text, left.id), words }
}
export function retimeCue(cue: SubtitleCue, start: number, end: number, move = false): SubtitleCue | string {
  if (move) {
    const delta = start - cue.start
    return { ...cue, start, end, words: cue.words?.map(word => ({ ...word, start: word.start + delta, end: word.end + delta })), sourceRanges: cue.sourceRanges?.map(range => ({ start: range.start + delta, end: range.end + delta })) }
  }
  const originalRanges = cueRanges(cue)
  if (start < cue.start) originalRanges[0].start = start
  if (end > cue.end) originalRanges.at(-1)!.end = end
  const ranges = restrict(originalRanges, start, end)
  if (duration(ranges) + 1e-9 < MINIMUM) return 'Keep at least 0.01 seconds of the cue source ranges.'
  const words = cue.words?.filter(word => within(ranges, word.start, word.end)).map(word => ({ ...word })) ?? []
  if (cue.words?.length && !words.length) return 'Keep at least one complete timed word, or edit the text to remove word timing.'
  return { ...fragment(cue, ranges, words.length ? words.map(word => word.text).join(' ') : cue.text, cue.id), words }
}
