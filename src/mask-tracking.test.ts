import { describe, expect, it } from 'vitest'
import { deleteMaskKeyframe, fullMask, manualCorrection, maskAt, maskPixels, retimeMask, trackedBlurPatch, trackingBudgetError } from './mask-tracking'
import type { MaskKeyframe, Redaction } from './types'
import sharedReference from '../testdata/mask-tracking-reference.json'
const key = (time: number, x: number, kind: MaskKeyframe['kind'] = 'auto'): MaskKeyframe => ({ time, x, y: .2, width: .1, height: .15, confidence: .9, kind })
const region = (keyframes: MaskKeyframe[] = [key(0, .1, 'manual'), key(1, .3), key(2, .6, 'manual')]): Redaction => ({ id: 'mask', start: 0, end: 6, x: .1, y: .2, width: .1, height: .15, mode: 'blur', strength: 12, tracking: { keyframes, reviewed: true } })
describe('tracked mask composition contract', () => {
  it('matches the Rust-owned fractional, loss and manual-boundary reference', () => {
    for (const expected of sharedReference.cases) {
      const actual = maskAt(sharedReference.region as Redaction, expected.time)
      expect(actual.lost).toBe(expected.lost)
      for (const field of ['x', 'y', 'width', 'height'] as const) expect(actual[field]).toBeCloseTo(expected[field], 12)
    }
  })
  it('unions toward auto frames even at exact left timestamps and holds until a manual correction', () => {
    const item = region()
    expect(maskAt(item, 0)).toMatchObject({ x: .1, lost: false }); expect(maskAt(item, 0).width).toBeCloseTo(.3)
    expect(maskAt(item, .9)).toEqual(maskAt(item, 0))
    expect(maskAt(item, 1)).toMatchObject({ x: .3, width: .1 })
    expect(maskAt(item, 1.99)).toEqual(maskAt(item, 1))
    expect(maskAt(item, 2)).toMatchObject({ x: .6, width: .1 })
    expect(maskAt(item, 5)).toEqual(maskAt(item, 2))
  })
  it('forces full solid masking before and after loss until the exact manual timestamp', () => {
    const item = region([key(0, .1, 'manual'), { ...key(2, 0, 'lost'), ...fullMask() }, key(4, .5, 'manual')])
    for (const time of [-1, 0, 1.999, 2, 3.999, NaN]) expect(maskAt(item, time)).toEqual({ ...fullMask(), lost: true })
    expect(maskAt(item, 4)).toMatchObject({ x: .5, width: .1, lost: false })
    expect(maskAt({ ...item, tracking: null }, 3)).toMatchObject({ x: .1, width: .1, lost: false })
  })
  it('uses fixed tracked blur context, clips actual pixels and caches by tracking identity', () => {
    const item = region([key(0, .1, 'manual'), key(1, .3), key(2, .9, 'manual')])
    expect(trackedBlurPatch(item, 0, 1000, 500)).toEqual({ x: 100, y: 100, width: 300, height: 75 })
    expect(trackedBlurPatch(item, 2, 1000, 500)).toEqual({ x: 700, y: 100, width: 300, height: 75 })
    expect(maskPixels(maskAt(item, 2), 1000, 500)).toEqual({ x: 900, y: 100, width: 100, height: 75 })
    const corrected = manualCorrection(item, 1, { x: .1, y: .2, width: .1, height: .15 })
    expect(trackedBlurPatch(corrected, 2, 1000, 500).width).toBe(100)
  })
})
describe('manual correction and retiming', () => {
  it('counts a replacement once while enforcing both per-mask and project keyframe budgets', () => {
    const dense = (id: string, count: number) => ({ ...region(), id, tracking: { reviewed: false, keyframes: Array.from({ length: count }, (_, i) => key(i / 10, .1)) } })
    const regions = [...Array.from({ length: 6 }, (_, i) => dense(String(i), 3000)), dense('last', 2000)]
    expect(trackingBudgetError(regions, dense('last', 2000))).toBeNull()
    expect(trackingBudgetError(regions, dense('last', 2001))).toContain('20,000')
    expect(trackingBudgetError([dense('only', 2)], dense('only', 3003))).toContain('3,002')
  })
  it('adds one manual key or replaces one timestamp without mutating original tracking', () => {
    const item = region(); const changed = manualCorrection(item, .5, { x: .4, y: .3, width: .2, height: .2 })
    expect(item.tracking!.keyframes).toHaveLength(3); expect(changed.tracking!.keyframes).toHaveLength(4); expect(changed.tracking!.reviewed).toBe(false)
    expect(manualCorrection(changed, .5, { x: .6, y: .3, width: .1, height: .2 }).tracking!.keyframes).toHaveLength(4)
    expect(changed.tracking!.keyframes[1]).toMatchObject({ time: .5, kind: 'manual', confidence: 1 })
  })
  it('shifts tracking with a moved region and retains a conservative start and end when trimming', () => {
    const item = region(); const moved = retimeMask(item, 2, 8, true); const trimmed = retimeMask(item, .5, 1.5)
    if (typeof moved === 'string' || typeof trimmed === 'string') throw new Error('Retiming failed')
    expect(moved.tracking!.keyframes.map(key => key.time)).toEqual([2, 3, 4]); expect(moved.tracking!.reviewed).toBe(false)
    expect(trimmed.tracking!.keyframes[0].time).toBe(.5); expect(trimmed.tracking!.keyframes.at(-1)!.time).toBe(1.5)
    expect(maskAt(trimmed, .5).width).toBeCloseTo(maskAt(item, .5).width)
  })
  it('inherits only contributing confidence for synthesized boundaries instead of inventing certainty', () => {
    const item = region([{ ...key(0, .1, 'manual'), confidence: 1 }, { ...key(1, .3), confidence: .7 }, { ...key(2, .6, 'manual'), confidence: 1 }])
    const trimmed = retimeMask(item, .5, 1.5)
    if (typeof trimmed === 'string') throw new Error(trimmed)
    expect(trimmed.tracking!.keyframes[0].confidence).toBe(.7)
    expect(trimmed.tracking!.keyframes.at(-1)!.confidence).toBe(.7)
    const extended = retimeMask(item, 0, 7)
    if (typeof extended === 'string') throw new Error(extended)
    expect(extended.tracking!.keyframes.at(-1)!.confidence).toBe(0)
  })
  it('adds lost protection when extending past known footage and rejects overlong tracked ranges', () => {
    const item = region(); const longer = retimeMask(item, 0, 7); const before = retimeMask({ ...item, start: 1, tracking: { ...item.tracking!, keyframes: item.tracking!.keyframes.map(key => ({ ...key, time: key.time + 1 })) } }, 0, 6)
    if (typeof longer === 'string' || typeof before === 'string') throw new Error('Retiming failed')
    expect(longer.tracking!.keyframes.at(-1)).toMatchObject({ time: 6, kind: 'lost' }); expect(maskAt(longer, 6.5).lost).toBe(true)
    expect(before.tracking!.keyframes[0]).toMatchObject({ time: 0, kind: 'lost' }); expect(maskAt(before, .5).lost).toBe(true)
    expect(retimeMask(item, 0, 301)).toContain('300 seconds')
  })
  it('keeps first/lost markers and rejects removal that would resume automatic tracking after loss', () => {
    const item = region([key(0, .1, 'manual'), { ...key(1, 0, 'lost'), ...fullMask() }, key(2, .4, 'manual'), key(3, .5)])
    expect(deleteMaskKeyframe(item, 0)).toContain('first keyframe'); expect(deleteMaskKeyframe(item, 1)).toContain('lost marker'); expect(deleteMaskKeyframe(item, 2)).toContain('needed after tracking was lost')
    const changed = deleteMaskKeyframe(item, 3); if (typeof changed === 'string') throw new Error(changed)
    expect(changed.tracking!.keyframes).toHaveLength(3); expect(changed.tracking!.reviewed).toBe(false)
  })
})
