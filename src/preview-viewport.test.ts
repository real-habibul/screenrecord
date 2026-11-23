import { describe, expect, it } from 'vitest'
import { canvasPoint, preserveViewportCenter, previewViewport } from './preview-viewport'

describe('preview viewport', () => {
  it('fits landscape and portrait canvases without cropping or overflow', () => {
    const landscape = previewViewport(1920, 1080, 800, 300, 'fit', 1)
    expect(landscape.cssWidth).toBeCloseTo(533.333333)
    expect(landscape.cssHeight).toBe(300)
    const portrait = previewViewport(1080, 1920, 800, 300, 'fit', 1)
    expect(portrait.cssWidth).toBe(168.75)
    expect(portrait.cssHeight).toBe(300)
  })
  it('defines 100 and 200 percent in canonical canvas pixels, independently of DPI', () => {
    const normal = previewViewport(1920, 1080, 800, 300, '100', 1)
    const retina = previewViewport(1920, 1080, 800, 300, '100', 2)
    const double = previewViewport(1920, 1080, 800, 300, '200', 2)
    expect([normal.cssWidth, retina.cssWidth, double.cssWidth]).toEqual([1920, 1920, 3840])
    expect([normal.bufferWidth, retina.bufferWidth, double.bufferWidth]).toEqual([1920, 3840, 4096])
  })
  it('bounds the render allocation while preserving large CSS canvases', () => {
    for (const [width, height] of [[8192, 1080], [1080, 8192], [4096, 4096], [1080, 1080]]) {
      const result = previewViewport(width, height, 800, 300, '200', 3)
      expect(result.cssWidth).toBe(width * 2)
      expect(Math.max(result.bufferWidth, result.bufferHeight)).toBeLessThanOrEqual(4096)
      expect(result.bufferWidth * result.bufferHeight).toBeLessThanOrEqual(16_000_000)
    }
  })
  it('preserves the viewed center through zoom and viewport resizing, then resets Fit scroll', () => {
    const fit = previewViewport(1920, 1080, 800, 300, 'fit', 1)
    const one = previewViewport(1920, 1080, 800, 300, '100', 1)
    const two = previewViewport(1920, 1080, 600, 400, '200', 1)
    expect(preserveViewportCenter(fit, one, 0, 0)).toEqual({ left: 560, top: 390 })
    const resized = preserveViewportCenter(one, two, 100, 50)
    expect(resized.left).toBeCloseTo(700); expect(resized.top).toBeCloseTo(200)
    expect(preserveViewportCenter(two, fit, 700, 200)).toEqual({ left: 0, top: 0 })
    expect(preserveViewportCenter(one, two, 99999, -99).left).toBe(3240)
  })
  it('maps a scrolled, doubled canvas back into canonical pixels without a letterbox offset', () => {
    expect(canvasPoint(320, 180, { left: -1600, top: -900, width: 3840, height: 2160 }, 1920, 1080)).toEqual({ x: 960, y: 540 })
    expect(canvasPoint(320, 180, { left: 80, top: 45, width: 480, height: 270 }, 1920, 1080)).toEqual({ x: 960, y: 540 })
  })
})
