import { clamp, even } from './editing'
import type { CameraSettings } from './types'

export interface Point { x: number; y: number }
export interface Rectangle extends Point { width: number; height: number }
export type Corner = 'nw' | 'ne' | 'sw' | 'se'
export type Manipulation = Corner | 'move'
export const corners: Corner[] = ['nw', 'ne', 'sw', 'se']

export function manipulateRegion(original: Rectangle, delta: Point, handle: Manipulation, sourceWidth: number, sourceHeight: number): Rectangle {
  if (handle === 'move') return { ...original, x: clamp(original.x + delta.x, 0, 1 - original.width), y: clamp(original.y + delta.y, 0, 1 - original.height) }
  const minWidth = Math.max(.001, 2 / sourceWidth); const minHeight = Math.max(.001, 2 / sourceHeight)
  const fixedLeft = Math.min(original.x,1-minWidth); const fixedRight = Math.max(original.x+original.width,minWidth)
  const fixedTop = Math.min(original.y,1-minHeight); const fixedBottom = Math.max(original.y+original.height,minHeight)
  const left = handle.includes('w') ? clamp(original.x + delta.x, 0, fixedRight - minWidth) : fixedLeft
  const right = handle.includes('e') ? clamp(original.x + original.width + delta.x, fixedLeft + minWidth, 1) : fixedRight
  const top = handle.includes('n') ? clamp(original.y + delta.y, 0, fixedBottom - minHeight) : fixedTop
  const bottom = handle.includes('s') ? clamp(original.y + original.height + delta.y, fixedTop + minHeight, 1) : fixedBottom
  return { x:left, y:top, width:right-left, height:bottom-top }
}

export function cameraRectangle(camera: Pick<CameraSettings, 'x' | 'y' | 'width'>, width: number, height: number, ratio: number): Rectangle {
  const cw = Math.min(even(camera.width * width), width, even(height * ratio))
  const ch = Math.min(even(cw / ratio), height)
  return { x:clamp(Math.round(camera.x * width), 0, width - cw), y:clamp(Math.round(camera.y * height), 0, height - ch), width:cw, height:ch }
}

export function manipulateCamera(original: Pick<CameraSettings, 'x' | 'y' | 'width'>, delta: Point, handle: Manipulation, width: number, height: number, ratio: number): Pick<CameraSettings, 'x' | 'y' | 'width'> {
  const box = cameraRectangle(original, width, height, ratio)
  if (handle === 'move') return { width:original.width, x:clamp(box.x + delta.x, 0, width - box.width) / width, y:clamp(box.y + delta.y, 0, height - box.height) / height }
  const horizontal = delta.x * (handle.includes('w') ? -1 : 1)
  const vertical = delta.y * (handle.includes('n') ? -1 : 1) * ratio
  const change = Math.abs(horizontal) >= Math.abs(vertical) ? horizontal : vertical
  const size = clamp((box.width + change) / width, .08, .6)
  const resized = cameraRectangle({ x:0, y:0, width:size }, width, height, ratio)
  const x = handle.includes('w') ? box.x + box.width - resized.width : box.x
  const y = handle.includes('n') ? box.y + box.height - resized.height : box.y
  return { width:size, x:clamp(x, 0, width - resized.width) / width, y:clamp(y, 0, height - resized.height) / height }
}

export function contains(point: Point, rectangle: Rectangle) {
  return point.x >= rectangle.x && point.y >= rectangle.y && point.x <= rectangle.x + rectangle.width && point.y <= rectangle.y + rectangle.height
}
