import { expect, it } from 'vitest'
import { loopSourceRanges, loopSourceTarget } from './preview-loop'

it('projects a half-open output loop through cuts and preserves exact source boundaries', () => {
  const ranges = loopSourceRanges({ trimStart: 0, trimEnd: 12, videoClips: [{ id: 'a', start: 1, end: 3 }, { id: 'b', start: 6, end: 9 }] }, { start: .5, end: 3 })!
  expect(ranges).toEqual([{ id: 'a', start: 1.5, end: 3 }, { id: 'b', start: 6, end: 7 }])
  expect([0, 1.5, 2, 3, 5, 6.2, 7, 12].map(time => loopSourceTarget(ranges, time))).toEqual([1.5, 1.5, 2, 6, 6, 6.2, 1.5, 1.5])
})

it('merges truly contiguous split clips and never includes the next clip at output B', () => {
  const edits = { trimStart: 0, trimEnd: 12, videoClips: [{ id: 'a', start: .1, end: 1.3 }, { id: 'b', start: 1.3, end: 2.4 }, { id: 'c', start: 5, end: 8 }] }
  expect(loopSourceRanges(edits, { start: 0, end: 2.3 })).toEqual([{ id: 'a', start: .1, end: 2.4 }])
  expect(loopSourceRanges(edits, { start: 2.3, end: 3.3 })).toEqual([{ id: 'c', start: 5, end: 6 }])
})

it('uses trimmed legacy source and rejects an empty or invalid output selection', () => {
  const edits = { trimStart: 3, trimEnd: 8, videoClips: [] }
  expect(loopSourceRanges(edits, { start: 1, end: 3 })).toEqual([{ id: 'full-recording', start: 4, end: 6 }])
  for (const range of [null, { start: 3, end: 3 }, { start: NaN, end: 4 }, { start: 7, end: 9 }]) expect(loopSourceRanges(edits, range)).toBeNull()
})
