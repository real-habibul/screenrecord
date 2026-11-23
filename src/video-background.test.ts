import { describe, expect, it } from 'vitest'
import { backgroundGeometry } from './video-background'
import { MAX_VIEWPORT_TILES, VIEWPORT_TILE_EDGE } from './viewport-tiles'
import reference from '../testdata/background-reference.json'

describe('bounded video background geometry', () => {
  for (const value of reference.cases) it(`independent fixture: ${value.id}`, () => {
    const view = value.viewport
    const g = backgroundGeometry({ sx: view.x, sy: view.y, sw: view.width, sh: view.height, ...value.canvas }, value.strength)
    expect({ x: g.sx, y: g.sy, width: g.sw, height: g.sh }).toEqual(value.expected.crop)
    expect({ width: g.width, height: g.height }).toEqual(value.expected.scratch)
    expect(g.sigma).toBeCloseTo(value.expected.sigmaX, 10)
  })
  it('center-covers only the final viewport with integer source coordinates', () => {
    const geometry = backgroundGeometry({ sx: 103, sy: 41, sw: 321, sh: 181, width: 1080, height: 1920 }, 24)
    expect([geometry.sx, geometry.sy, geometry.sw, geometry.sh]).toEqual([213, 41, 101, 181])
    expect([geometry.width, geometry.height]).toEqual([540, 960]); expect(geometry.sigma).toBe(12)
  })
  it('bounds all three allocated surfaces even for one-pixel and extreme-aspect viewports', () => {
    for (const source of [{ sw: 1820, sh: 6 }, { sw: 1, sh: 1 }, { sw: 1, sh: 8192 }, { sw: 8192, sh: 1 }]) {
      for (const canvas of [{ width: 1080, height: 1920 }, { width: 8192, height: 1080 }, { width: 2160, height: 2160 }]) {
        const g = backgroundGeometry({ sx: 7, sy: 13, ...source, ...canvas }, 64)
        expect(Math.max(g.width, g.height)).toBeLessThanOrEqual(960)
        expect(g.sx).toBeGreaterThanOrEqual(7); expect(g.sy).toBeGreaterThanOrEqual(13)
        expect(g.sx + g.sw).toBeLessThanOrEqual(7 + source.sw); expect(g.sy + g.sh).toBeLessThanOrEqual(13 + source.sh)
        const bytes = 4 * (g.width * g.height * 2 + (g.width + g.padding * 2) * (g.height + g.padding * 2) + MAX_VIEWPORT_TILES * VIEWPORT_TILE_EDGE ** 2)
        expect(bytes).toBeLessThan(20 * 1024 * 1024)
      }
    }
  })
  it('scales reference sigma and clamps unsupported strengths', () => {
    const view = { sx: 0, sy: 0, sw: 1920, sh: 1080, width: 1920, height: 1080 }
    expect(backgroundGeometry(view, 4).sigma).toBe(2); expect(backgroundGeometry(view, 64).sigma).toBe(32)
    expect(backgroundGeometry(view, 0).sigma).toBe(2); expect(backgroundGeometry(view, 1000).sigma).toBe(32)
    expect(backgroundGeometry({ ...view, width: 3840, height: 2160 }, 24).sigma).toBe(12)
  })
})
