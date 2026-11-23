import { describe, expect, it } from 'vitest'
import { mergeCues, mergeRanges, retimeCue, splitCue } from './subtitle-editing'
import { activeWord, projectedSubtitle, subtitleAt, subtitleRasterKey } from './subtitle-projection'
import type { SubtitleCue, SubtitleStyle } from './types'

const cue = (): SubtitleCue => ({ id: 'cue', start: 0, end: 6, text: 'Keep secret also keep', words: [{ start: .1, end: 1, text: 'Keep' }, { start: 1, end: 3, text: 'secret' }, { start: 3, end: 4, text: 'also' }, { start: 4, end: 5, text: 'keep' }] })
const edits = { trimStart: 0, trimEnd: 6, videoClips: [{ id: 'first', start: 0, end: 1.5 }, { id: 'last', start: 3, end: 6 }] }
describe('safe subtitle projection', () => {
  it('keys glyph images by word segmentation and appearance, excluding timing and source coverage', () => {
    const style: SubtitleStyle = { enabled: true, highlightWords: true, highlightColor: '#E9B451', fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom' }
    const original = cue(); const key = (value: SubtitleCue, index: number | null = 1) => subtitleRasterKey(value, index, style, 'source', null)
    expect(key(original)).toBe(key({ ...original, start: 1, end: 7, sourceRanges: [{ start: 1, end: 7 }], words: original.words!.map(word => ({ ...word, start: word.start + 1, end: word.end + 1 })) }))
    expect(key(original)).not.toBe(key(original, null))
    expect(key(original)).not.toBe(key({ ...original, words: [{ start: .1, end: 3, text: 'Keep secret' }, { start: 3, end: 5, text: 'also keep' }] }))
    expect(key(original)).not.toBe(key(projectedSubtitle(edits, original)!))
  })
  it('removes an entire word even if only part intersects a cut without changing raw text', () => {
    const raw = cue(); const safe = projectedSubtitle(edits, raw)!
    expect(safe.text).toBe('Keep also keep'); expect(safe.words?.map(word => word.text)).toEqual(['Keep', 'also', 'keep'])
    expect(raw.text).toBe('Keep secret also keep'); expect(subtitleAt(safe, 2)).toBe(false); expect(subtitleAt(safe, 3)).toBe(true)
    expect(activeWord(safe, 3)).toBe(1); expect(activeWord(safe, 4)).toBe(2); expect(activeWord(safe, 5)).toBeNull()
  })
  it('honors imported cross-cut coverage and hides legacy text only when declared footage is removed', () => {
    const raw = { ...cue(), words: [] }
    expect(projectedSubtitle(edits, raw)).toBeNull()
    const imported = { ...raw, sourceRanges: [{ start: 0, end: 1.5 }, { start: 3, end: 6 }] }
    expect(projectedSubtitle(edits, imported)?.text).toBe(raw.text)
    expect(projectedSubtitle({ ...edits, videoClips: [{ id: 'a', start: .2, end: 1.5 }, { id: 'b', start: 3, end: 6 }] }, imported)).toBeNull()
  })
  it('never joins a positive removed gap and treats adjacent split clips identically', () => {
    const tinyGap = { trimStart: 0, trimEnd: 6, videoClips: [{ id: 'a', start: 0, end: 2 }, { id: 'b', start: 2 + 1e-10, end: 6 }] }
    expect(projectedSubtitle(tinyGap, cue())?.text).toBe('Keep also keep')
    expect(projectedSubtitle({ ...tinyGap, videoClips: [{ id: 'a', start: 0, end: 2 }, { id: 'b', start: 2, end: 6 }] }, cue())?.text).toBe(cue().text)
    expect(mergeRanges([{ start: 0, end: 1 }, { start: 1 + 1e-10, end: 2 }])).toHaveLength(2)
  })
})
describe('subtitle editing without lost text', () => {
  it('splits at a complete word boundary while retaining all word metadata', () => {
    const result = splitCue(cue(), 2.8)
    if (typeof result === 'string') throw new Error(result)
    expect(result.left.text).toBe('Keep secret'); expect(result.right.text).toBe('also keep')
    expect(result.left.end).toBe(3); expect(result.right.start).toBe(3); expect(result.right.id).not.toBe(result.left.id)
    expect([...result.left.words!, ...result.right.words!]).toEqual(cue().words)
  })
  it('supports caret and estimated splits without making up word timestamps', () => {
    const raw = { ...cue(), words: [] }
    const caret = splitCue(raw, 3, 4); const estimated = splitCue(raw, 3)
    if (typeof caret === 'string' || typeof estimated === 'string') throw new Error('Split failed')
    expect(caret.left.text).toBe('Keep'); expect(caret.right.text).toBe('secret also keep'); expect(caret.left.words).toEqual([]); expect(caret.estimated).toBe(false); expect(estimated.estimated).toBe(true)
    expect(`${estimated.left.text} ${estimated.right.text}`).toBe(raw.text)
  })
  it('keeps accepted sub-nanosecond word overlaps inside their split coverage', () => {
    const result = splitCue({ id: 'precise', start: 0, end: 2, text: 'one two', words: [{ start: 0, end: 1 + 1e-10, text: 'one' }, { start: 1, end: 2, text: 'two' }] }, 1)
    if (typeof result === 'string') throw new Error(result)
    expect(result.left.sourceRanges![0].end).toBeGreaterThanOrEqual(result.left.words![0].end)
    expect(result.left.end - result.right.start).toBeLessThan(1e-9)
    expect(result.right.words![0].start).toBe(result.right.sourceRanges![0].start)
  })
  it('merges safe visible cues without recovering hidden text or bridging removed ranges', () => {
    const safe = projectedSubtitle(edits, cue())!
    const parts = splitCue(safe, 3.5)
    if (typeof parts === 'string') throw new Error(parts)
    const merged = mergeCues(parts.left, parts.right)
    if (typeof merged === 'string') throw new Error(merged)
    expect(merged.text).toBe('Keep also keep'); expect(merged.sourceRanges).toEqual(safe.sourceRanges)
    expect(mergeCues({ ...parts.left, words: [] }, parts.right)).toMatchObject({ words: [] })
  })
  it('moves source ranges and word times together, trims whole words, and rejects empty timed output', () => {
    const original = cue(); original.sourceRanges = [{ start: 0, end: 6 }]
    expect(retimeCue(original, 2, 8, true)).toMatchObject({ start: 2, end: 8, sourceRanges: [{ start: 2, end: 8 }], words: [{ start: 2.1, end: 3, text: 'Keep' }, { start: 3, end: 5, text: 'secret' }, { start: 5, end: 6, text: 'also' }, { start: 6, end: 7, text: 'keep' }] })
    expect(retimeCue(original, 1.2, 6)).toMatchObject({ text: 'also keep' })
    expect(retimeCue(original, 5.1, 6)).toContain('complete timed word')
  })
  it('merges interleaved source ranges with chronological words and their original coverage', () => {
    const left: SubtitleCue = { id: 'outer', start: 0, end: 6, text: 'first last', words: [{ start: .2, end: .8, text: 'first' }, { start: 5.2, end: 5.8, text: 'last' }], sourceRanges: [{ start: 0, end: 1 }, { start: 5, end: 6 }] }
    const right: SubtitleCue = { id: 'inner', start: 2, end: 3, text: 'middle', words: [{ start: 2.2, end: 2.8, text: 'middle' }], sourceRanges: [{ start: 2, end: 3 }] }
    const result = mergeCues(left, right)
    if (typeof result === 'string') throw new Error(result)
    expect(result.text).toBe('first middle last')
    expect(result.words).toEqual([left.words![0], right.words![0], left.words![1]])
    expect(result.sourceRanges).toEqual([{ start: 0, end: 1 }, { start: 2, end: 3 }, { start: 5, end: 6 }])
    expect([result.start, result.end]).toEqual([0, 6])
    expect(left.text).toBe('first last')
  })
})
