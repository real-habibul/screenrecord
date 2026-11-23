import { describe, expect, it } from 'vitest'
import { drawCaptureRegion, fitCaptureRegion } from './capture-region'

describe('source pixel capture region', () => {
  it('keeps even dimensions inside odd source bounds without widening to full source', () => {
    expect(fitCaptureRegion({ x: 1918, y: 1078, width: 103, height: 51 }, 1921, 1081)).toEqual({
      x: 1819,
      y: 1031,
      width: 102,
      height: 50,
    })
    expect(fitCaptureRegion({ x: -5, y: -5, width: 1, height: 1 }, 1920, 1080)).toEqual({
      x: 0,
      y: 0,
      width: 2,
      height: 2,
    })
  })
  it('draws in either direction and retains original-source pixel coordinates', () => {
    expect(drawCaptureRegion({ x: 151.7, y: 90.2 }, { x: 20.1, y: 10.1 }, 200, 100)).toEqual({
      x: 20,
      y: 10,
      width: 130,
      height: 80,
    })
  })
  it('constrains numeric resizing and invalid numeric input', () => {
    expect(fitCaptureRegion({ x: Infinity, y: NaN, width: 2001, height: NaN }, 1920, 1080)).toEqual(
      { x: 0, y: 0, width: 1920, height: 1080 },
    )
  })
})
