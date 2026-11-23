import { ViewportTiles, type CanvasRectangle } from './viewport-tiles'

export interface BackgroundViewport { sx: number; sy: number; sw: number; sh: number; width: number; height: number }
export const BACKGROUND_MAX_EDGE = 960

export function backgroundGeometry(view: BackgroundViewport, strength: number) {
  const scale = Math.min(1, BACKGROUND_MAX_EDGE / Math.max(view.width, view.height))
  const width = Math.max(2, Math.round(view.width * scale)); const height = Math.max(2, Math.round(view.height * scale))
  const sourceWidth = Math.max(1, Math.min(view.sw, Math.floor(view.sh * view.width / view.height)))
  const sourceHeight = Math.max(1, Math.min(view.sh, Math.floor(view.sw * view.height / view.width)))
  const sigma = Math.max(4, Math.min(64, strength)) * Math.min(view.width, view.height) / 1080 * width / view.width
  const padding = Math.ceil(sigma * 4) + 2
  return { width, height, sigma, padding, sx: view.sx + Math.floor((view.sw - sourceWidth) / 2), sy: view.sy + Math.floor((view.sh - sourceHeight) / 2), sw: sourceWidth, sh: sourceHeight }
}

export class VideoBackground {
  private source = document.createElement('canvas')
  private extended = document.createElement('canvas')
  private blurred = document.createElement('canvas')
  private tiles = new ViewportTiles()

  draw(frame: CanvasImageSource, viewport: BackgroundViewport, strength: number): HTMLCanvasElement {
    const { width, height, sigma, padding: p, sx, sy, sw, sh } = backgroundGeometry(viewport, strength)
    for (const [canvas, w, h] of [[this.source, width, height], [this.extended, width + p * 2, height + p * 2], [this.blurred, width, height]] as const) {
      if (canvas.width !== w || canvas.height !== h) { canvas.width = w; canvas.height = h }
    }
    const source = this.source.getContext('2d', { alpha: false })!
    source.imageSmoothingEnabled = false; source.fillStyle = '#000'; source.fillRect(0, 0, width, height)
    this.tiles.draw(source, frame, { x: sx, y: sy, width: sw, height: sh }, { x: 0, y: 0, width, height })
    const extended = this.extended.getContext('2d', { alpha: false })!
    for (const [x, y, w, h, dx, dy, dw, dh] of [
      [0, 0, width, height, p, p, width, height],
      [0, 0, width, 1, p, 0, width, p], [0, height - 1, width, 1, p, p + height, width, p],
      [0, 0, 1, height, 0, p, p, height], [width - 1, 0, 1, height, p + width, p, p, height],
      [0, 0, 1, 1, 0, 0, p, p], [width - 1, 0, 1, 1, p + width, 0, p, p],
      [0, height - 1, 1, 1, 0, p + height, p, p], [width - 1, height - 1, 1, 1, p + width, p + height, p, p],
    ]) extended.drawImage(this.source, x, y, w, h, dx, dy, dw, dh)
    const blurred = this.blurred.getContext('2d', { alpha: false })!
    blurred.filter = `blur(${sigma}px)`; blurred.drawImage(this.extended, -p, -p); blurred.filter = 'none'
    return this.blurred
  }

  drawForeground(target: CanvasRenderingContext2D, frame: CanvasImageSource, viewport: BackgroundViewport, destination: CanvasRectangle) { this.tiles.draw(target, frame, { x: viewport.sx, y: viewport.sy, width: viewport.sw, height: viewport.sh }, destination) }
  clear() { this.tiles.clear(); for (const canvas of [this.source, this.extended, this.blurred]) { const context = canvas.getContext('2d', { alpha: false })!; context.filter = 'none'; context.fillStyle = '#000'; context.fillRect(0, 0, canvas.width, canvas.height) } }
  release() { this.tiles.release(); for (const canvas of [this.source, this.extended, this.blurred]) { canvas.width = 0; canvas.height = 0 } }
}
