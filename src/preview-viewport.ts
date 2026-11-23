export type PreviewScale = 'fit' | '100' | '200'
export interface PreviewViewport {
  width: number; height: number; viewportWidth: number; viewportHeight: number;
  scale: number; cssWidth: number; cssHeight: number; bufferWidth: number; bufferHeight: number;
}

export function previewViewport(width: number, height: number, viewportWidth: number, viewportHeight: number, mode: PreviewScale, pixelRatio: number): PreviewViewport {
  const availableWidth = Math.max(1, viewportWidth); const availableHeight = Math.max(1, viewportHeight)
  const scale = mode === 'fit' ? Math.min(availableWidth / width, availableHeight / height) : mode === '100' ? 1 : 2
  const bufferScale = Math.min(scale * Math.max(1, pixelRatio), 4096 / Math.max(width, height), Math.sqrt(16_000_000 / (width * height)))
  return {
    width, height, viewportWidth: availableWidth, viewportHeight: availableHeight, scale,
    cssWidth: width * scale, cssHeight: height * scale,
    bufferWidth: Math.max(1, Math.floor(width * bufferScale)), bufferHeight: Math.max(1, Math.floor(height * bufferScale)),
  }
}

export function preserveViewportCenter(previous: PreviewViewport, next: PreviewViewport, left: number, top: number) {
  const center = (scroll: number, viewport: number, canvas: number) => canvas <= viewport ? .5 : (scroll + viewport / 2) / canvas
  const offset = (position: number, canvas: number, viewport: number) => Math.max(0, Math.min(canvas - viewport, position * canvas - viewport / 2))
  return {
    left: offset(center(left, previous.viewportWidth, previous.cssWidth), next.cssWidth, next.viewportWidth),
    top: offset(center(top, previous.viewportHeight, previous.cssHeight), next.cssHeight, next.viewportHeight),
  }
}

export function canvasPoint(clientX: number, clientY: number, box: Pick<DOMRect, 'left' | 'top' | 'width' | 'height'>, width: number, height: number) {
  return { x: (clientX - box.left) * width / box.width, y: (clientY - box.top) * height / box.height }
}
