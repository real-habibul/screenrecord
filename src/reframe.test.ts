import { describe, expect, it } from 'vitest'
import { reframeAt, setReframeKey } from './reframe'
import type { ReframeKeyframe } from './types'
const key = (time: number, x = .5, y = .5, scale = 1): ReframeKeyframe => ({ id: `key-${time}`, time, x, y, scale })

describe('manual framing keyframes', () => {
  it('holds endpoints and interpolates all three source-time properties', () => {
    const keys = [key(2, .2, .1, 1), key(6, .8, .9, 3)]
    expect(reframeAt(keys, -1)).toEqual({ x: .2, y: .1, scale: 1 })
    expect(reframeAt(keys, 4)).toEqual({ x: .5, y: .5, scale: 2 })
    expect(reframeAt(keys, 8)).toEqual({ x: .8, y: .9, scale: 3 })
    expect(reframeAt(keys, NaN)).toBeNull(); expect(reframeAt([], 1)).toBeNull()
  })
  it('updates the closest near key without moving its source timestamp or corrupting its neighbors', () => {
    const keys = [key(1), key(1.001), key(2)]
    const result = setReframeKey(keys, 1.0008, { x: .8, y: .3, scale: 2 }, 12)
    expect(typeof result).not.toBe('string'); if (typeof result === 'string') return
    expect(result.selected).toBe('key-1.001'); expect(result.keys.map(key => key.time)).toEqual([1, 1.001, 2]); expect(result.keys[1].x).toBe(.8)
    expect(keys[1].x).toBe(.5)
  })
  it('allows the minimum gap and a valid update at capacity but refuses another key', () => {
    const boundary = setReframeKey([key(0)], .001, key(.001), 12)
    expect(typeof boundary).not.toBe('string'); if (typeof boundary !== 'string') expect(boundary.keys).toHaveLength(2)
    const keys = Array.from({ length: 3000 }, (_, i) => key(i / 100))
    expect(setReframeKey(keys, 40, key(40), 50)).toContain('at most 3000')
    const updated = setReframeKey(keys, 10.0005, { x: .1, y: .2, scale: 3 }, 50)
    expect(typeof updated).not.toBe('string'); if (typeof updated !== 'string') { expect(updated.keys).toHaveLength(3000); expect(updated.keys[1000].scale).toBe(3) }
  })
  it('rejects invalid input rather than serializing invalid metadata', () => {
    for (const invalid of [{ time: NaN }, { time: -1 }, { time: 13 }, { x: 1.1 }, { y: -1 }, { scale: 0 }, { scale: 3.1 }]) {
      const value = { ...key(1), ...invalid }
      expect(typeof setReframeKey([], value.time, value, 12)).toBe('string')
    }
  })
})
