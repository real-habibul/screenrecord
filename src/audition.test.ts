import { describe, expect, it } from 'vitest'
import { auditionFromPlayhead, fitAuditionRange } from './audition'

describe('edited-time audition selection', () => {
  it('starts at the playhead and keeps the requested length near the end', () => {
    expect(auditionFromPlayhead(19, 30)).toEqual({ start: 19, end: 22 })
    expect(auditionFromPlayhead(29.9, 30)).toEqual({ start: 27, end: 30 })
    expect(auditionFromPlayhead(1, 1.35)).toEqual({ start: 0, end: 1.35 })
  })

  it('keeps a playable interval after shortening or removing the selected cut', () => {
    expect(fitAuditionRange({ start: 2, end: 8 }, 3.5)).toEqual({ start: 2, end: 3.5 })
    expect(fitAuditionRange({ start: 20, end: 29 }, 4)).toEqual({ start: 3.9, end: 4 })
    expect(fitAuditionRange({ start: NaN, end: Infinity }, 0.05)).toEqual({ start: 0, end: 0.05 })
    expect(fitAuditionRange({ start: 1, end: 2 }, 0)).toEqual({ start: 0, end: 0 })
  })
})
