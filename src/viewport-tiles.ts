export interface CanvasRectangle { x: number; y: number; width: number; height: number }
export const VIEWPORT_TILE_EDGE = 512
export const MAX_VIEWPORT_TILES = 4

export class ViewportTiles {
  private tiles = new Map<string, HTMLCanvasElement>()

  draw(target: CanvasRenderingContext2D, frame: CanvasImageSource, source: CanvasRectangle, destination: CanvasRectangle) {
    const transform = target.getTransform()
    // Preview transforms are axis-aligned; shared device-pixel edges prevent translucent tile seams.
    const alignX = (value: number) => (Math.round(value * transform.a + transform.e) - transform.e) / transform.a
    const alignY = (value: number) => (Math.round(value * transform.d + transform.f) - transform.f) / transform.d
    for (let y = 0; y < source.height; y += VIEWPORT_TILE_EDGE) for (let x = 0; x < source.width; x += VIEWPORT_TILE_EDGE) {
      const width = Math.min(VIEWPORT_TILE_EDGE, source.width - x); const height = Math.min(VIEWPORT_TILE_EDGE, source.height - y)
      const shape = `${width}:${height}`
      let tile = this.tiles.get(shape)
      if (!tile) {
        if (this.tiles.size >= MAX_VIEWPORT_TILES) { const oldest = this.tiles.keys().next().value!; const removed = this.tiles.get(oldest)!; removed.width = 0; removed.height = 0; this.tiles.delete(oldest) }
        tile = document.createElement('canvas'); tile.width = width; tile.height = height; this.tiles.set(shape, tile)
      } else { this.tiles.delete(shape); this.tiles.set(shape, tile) }
      const native = tile.getContext('2d', { alpha: false })!
      // Native copies isolate the crop before Chromium's resampler can read adjacent source pixels.
      native.imageSmoothingEnabled = false; native.globalCompositeOperation = 'copy'
      native.drawImage(frame, source.x + x, source.y + y, width, height, 0, 0, width, height)
      const dx = alignX(destination.x + x / source.width * destination.width); const dy = alignY(destination.y + y / source.height * destination.height)
      const dw = alignX(destination.x + (x + width) / source.width * destination.width) - dx; const dh = alignY(destination.y + (y + height) / source.height * destination.height) - dy
      if (dw > 0 && dh > 0) target.drawImage(tile, dx, dy, dw, dh)
    }
  }
  clear() { for (const tile of this.tiles.values()) { const context = tile.getContext('2d', { alpha: false })!; context.fillStyle = '#000'; context.fillRect(0, 0, tile.width, tile.height) } }
  release() { for (const tile of this.tiles.values()) { tile.width = 0; tile.height = 0 }; this.tiles.clear() }
}
