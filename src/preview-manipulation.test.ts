import { describe, expect, it } from 'vitest'
import { cameraRectangle, corners, manipulateCamera, manipulateRegion } from './preview-manipulation'

describe('direct preview manipulation', () => {
  it('moves a mask inside original-source bounds without resizing', () => {
    const original = { x:.3,y:.4,width:.2,height:.1 }
    expect(manipulateRegion(original,{ x:2,y:-2 },'move',1920,1080)).toEqual({ x:.8,y:0,width:.2,height:.1 })
    expect(original).toEqual({ x:.3,y:.4,width:.2,height:.1 })
  })
  it('resizes every corner without inversion or leaving source bounds', () => {
    for (const corner of corners) for (const value of [-10,10]) {
      const result = manipulateRegion({ x:.3,y:.4,width:.2,height:.1 },{ x:value,y:value },corner,1920,1080)
      expect(result.width).toBeGreaterThanOrEqual(2/1920-1e-9); expect(result.height).toBeGreaterThanOrEqual(2/1080-1e-9)
      expect(result.x).toBeGreaterThanOrEqual(0); expect(result.y).toBeGreaterThanOrEqual(0)
      expect(result.x+result.width).toBeLessThanOrEqual(1); expect(result.y+result.height).toBeLessThanOrEqual(1)
    }
  })
  it('expands a valid subpixel legacy mask to minimum drag size at the source edge', () => {
    const result = manipulateRegion({ x:.999,y:.999,width:.001,height:.001 },{ x:0,y:0 },'se',960,540)
    expect(result.x+result.width).toBeLessThanOrEqual(1); expect(result.y+result.height).toBeLessThanOrEqual(1)
    expect(result.width).toBeCloseTo(2/960); expect(result.height).toBeCloseTo(2/540)
  })
  it('moves from the camera position actually drawn when saved coordinates exceed the output edge', () => {
    const original = { x:.95,y:.95,width:.2 }
    const box = cameraRectangle(original,1920,1080,16/9)
    const result = manipulateCamera(original,{ x:-10,y:-20 },'move',1920,1080,16/9)
    expect(result.x).toBeCloseTo((box.x-10)/1920); expect(result.y).toBeCloseTo((box.y-20)/1080)
    expect(result.width).toBe(.2)
  })
  it('preserves the opposite camera corner during resizing and respects width bounds', () => {
    const original = { x:.3,y:.3,width:.2 }
    const initial = cameraRectangle(original,1920,1080,1)
    const resized = manipulateCamera(original,{ x:-96,y:-96 },'nw',1920,1080,1)
    const box = cameraRectangle(resized,1920,1080,1)
    expect(box.width).toBe(480); expect(box.height).toBe(480)
    expect(box.x+box.width).toBe(initial.x+initial.width); expect(box.y+box.height).toBe(initial.y+initial.height)
    for (const corner of corners) for (const amount of [-20000,20000]) {
      const result = manipulateCamera(original,{ x:amount,y:amount },corner,1080,1920,16/9)
      const bounds = cameraRectangle(result,1080,1920,16/9)
      expect(result.width).toBeGreaterThanOrEqual(.08); expect(result.width).toBeLessThanOrEqual(.6)
      expect(bounds.x).toBeGreaterThanOrEqual(0); expect(bounds.y).toBeGreaterThanOrEqual(0)
      expect(bounds.x+bounds.width).toBeLessThanOrEqual(1080); expect(bounds.y+bounds.height).toBeLessThanOrEqual(1920)
    }
  })
})
