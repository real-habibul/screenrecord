import { describe, expect, it } from 'vitest'
import { changeSourceTrim, changeVideoBoundary, cutVideoRange, keptClips, outputDuration, outputToSource, playbackRanges, projectedRange, removeVideo, sourceToOutput, splitVideo, type VideoEdits } from './video-edits'

const edits = (): VideoEdits => ({ trimStart: 1, trimEnd: 11, videoClips: [{ id: 'a', start: 0, end: 3 }, { id: 'b', start: 6, end: 12 }] })
describe('source edit decision list', () => {
  it('maps a contiguous output onto source ranges with following-clip boundary ownership', () => {
    const e = edits()
    expect(outputDuration(e)).toBe(7)
    expect([0, 1, 2, 3, 7].map(t => outputToSource(e, t))).toEqual([1, 2, 6, 7, 11])
    expect([0, 1, 3, 5, 6, 11, 12].map(t => sourceToOutput(e, t))).toEqual([0, 0, 2, 2, 2, 7, 7])
    expect(keptClips({ ...e, videoClips: [] })).toMatchObject([{ start: 1, end: 11, outputStart: 0, outputEnd: 10 }])
  })
  it('projects source-anchored effects into kept fragments without modifying originals', () => {
    expect(projectedRange(edits(), 2, 8)).toEqual([{ clipId: 'a', sourceStart: 2, sourceEnd: 3, start: 1, end: 2 }, { clipId: 'b', sourceStart: 6, sourceEnd: 8, start: 2, end: 4 }])
  })
  it('splits without changing duration or introducing a playback discontinuity', () => {
    const e: VideoEdits = { trimStart: 0, trimEnd: 12, videoClips: [] }
    const result = splitVideo(e, 5); expect(typeof result).not.toBe('string')
    if (typeof result === 'string') return
    e.videoClips = result.clips
    expect(result.clips).toHaveLength(2); expect(outputDuration(e)).toBe(12)
    expect(playbackRanges(e)).toMatchObject([{ start: 0, end: 12 }])
    expect(typeof splitVideo(e, 5)).toBe('string'); expect(typeof splitVideo(e, .09)).toBe('string')
  })
  it('ripple cuts across a removed gap and keeps all secondary source times untouched', () => {
    const e = edits(); const result = cutVideoRange(e, 1, 3)
    expect(result).toMatchObject([{ start: 0, end: 2 }, { start: 7, end: 12 }])
    if (typeof result === 'string') return
    expect(outputDuration({ ...e, videoClips: result })).toBe(5)
    expect(typeof cutVideoRange(e, 0, 7)).toBe('string')
    expect(typeof cutVideoRange({ trimStart: 0, trimEnd: 1, videoClips: [] }, .05, .5)).toBe('string')
  })
  it('refuses empty output, overlapping boundaries and clips shorter than100ms', () => {
    const e = edits(); expect(changeSourceTrim(e, 'trimStart', 11, 12)).toBeNull()
    expect(outputDuration(e)).toBeCloseTo(.1)
    e.trimStart = 3; e.trimEnd = 6
    expect(changeSourceTrim(e, 'trimStart', 4, 12)).toContain('removes all')
    const other = edits(); changeVideoBoundary(other, 'a', 'end', 10, 12)
    expect(other.videoClips[0].end).toBe(6)
    expect(typeof removeVideo({ ...other, videoClips: [other.videoClips[0]] }, 'a')).toBe('string')
  })
  it('retains tiny trim intersections but enforces the512 raw-clip limit', () => {
    const e = edits(); e.trimStart = 2.99
    expect(keptClips(e)[0].end - keptClips(e)[0].start).toBeCloseTo(.01)
    const many: VideoEdits = { trimStart: 0, trimEnd: 1024, videoClips: Array.from({ length: 512 }, (_, i) => ({ id: String(i), start: i * 2, end: i * 2 + 1 })) }
    expect(splitVideo(many, .5)).toContain('512')
  })
})
