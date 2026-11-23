import type { CaptureRegion } from './types'

export function fitCaptureRegion(
  value: CaptureRegion,
  width: number,
  height: number,
): CaptureRegion {
  const even = (n: number) => Math.max(2, Math.floor(n / 2) * 2)
  const w = Math.min(even(width), even(Number.isFinite(value.width) ? value.width : width))
  const h = Math.min(even(height), even(Number.isFinite(value.height) ? value.height : height))
  return {
    x: Math.max(0, Math.min(width - w, Math.round(Number.isFinite(value.x) ? value.x : 0))),
    y: Math.max(0, Math.min(height - h, Math.round(Number.isFinite(value.y) ? value.y : 0))),
    width: w,
    height: h,
  }
}

export function drawCaptureRegion(
  a: { x: number; y: number },
  b: { x: number; y: number },
  width: number,
  height: number,
): CaptureRegion {
  return fitCaptureRegion(
    {
      x: Math.min(a.x, b.x),
      y: Math.min(a.y, b.y),
      width: Math.abs(a.x - b.x),
      height: Math.abs(a.y - b.y),
    },
    width,
    height,
  )
}
