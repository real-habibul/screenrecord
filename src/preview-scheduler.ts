export interface FrameSource {
  requestVideoFrameCallback?: (callback: VideoFrameRequestCallback) => number
  cancelVideoFrameCallback?: (handle: number) => void
}
interface SourceSubscription { source: FrameSource; handle: number | null; lastFrame: number | null; failed: boolean }
export interface PreviewSchedulerOptions {
  requestFrame: (callback: FrameRequestCallback) => number
  cancelFrame: (handle: number) => void
  sources: () => FrameSource[]
  visible: () => boolean
  clock: (now: number) => void
  paint: () => void
  presented?: (source: FrameSource, metadata: VideoFrameCallbackMetadata) => void
}

export class PreviewScheduler {
  private revision = 0
  private pending: number | null = null
  private running = false
  private dirty = false
  private disposed = false
  private flushing = false
  private subscriptions = new Map<FrameSource, SourceSubscription>()
  constructor(private options: PreviewSchedulerOptions) {}

  invalidate = () => { if (!this.disposed) { this.dirty = true; this.schedule() } }
  start() { if (this.disposed) return; this.running = true; this.invalidate() }
  paintNow() { if (!this.disposed && this.options.visible()) { this.updateSources(); this.dirty = false; this.options.paint() } }
  stop() {
    this.revision++; this.running = false; this.dirty = false
    if (this.pending !== null) this.options.cancelFrame(this.pending)
    this.pending = null
    for (const item of this.subscriptions.values()) this.cancelSource(item)
    this.subscriptions.clear()
  }
  visibilityChanged() {
    if (this.disposed) return
    for (const item of this.subscriptions.values()) this.cancelSource(item)
    this.subscriptions.clear(); this.invalidate()
  }
  dispose() { this.stop(); this.disposed = true }
  private cancelSource(item: SourceSubscription) { if (item.handle !== null) { try { item.source.cancelVideoFrameCallback?.(item.handle) } catch { /* Clock fallback remains available if native callback cancellation fails. */ }; item.handle = null } }
  private subscribe(item: SourceSubscription) {
    if (item.failed || item.handle !== null || !this.options.visible()) return
    const revision = this.revision
    try {
      item.handle = item.source.requestVideoFrameCallback!((_now, metadata) => {
        if (revision !== this.revision || this.subscriptions.get(item.source) !== item) return
        item.handle = null
        this.options.presented?.(item.source, metadata)
        if (item.lastFrame !== metadata.presentedFrames) { item.lastFrame = metadata.presentedFrames; this.invalidate() }
        this.subscribe(item)
      })
    } catch { item.failed = true; item.handle = null }
  }
  private updateSources() {
    const sources = this.options.sources()
    for (const [source, item] of this.subscriptions) if (!sources.includes(source)) { this.cancelSource(item); this.subscriptions.delete(source) }
    for (const source of sources) {
      let item = this.subscriptions.get(source)
      if (!item) { item = { source, handle: null, lastFrame: null, failed: typeof source.requestVideoFrameCallback !== 'function' || typeof source.cancelVideoFrameCallback !== 'function' }; this.subscriptions.set(source, item) }
      this.subscribe(item)
    }
    return [...this.subscriptions.values()].some(item => item.failed)
  }
  private schedule() {
    if (this.pending !== null || this.flushing || this.disposed || !this.running && (!this.dirty || !this.options.visible())) return
    const revision = this.revision
    const handle = this.options.requestFrame(now => {
      if (revision !== this.revision || this.pending !== handle || this.disposed) return
      this.pending = null; this.flushing = true
      try {
        const fallback = this.updateSources()
        if (this.running) { if (fallback) this.dirty = true; this.options.clock(now) }
        if (revision !== this.revision || this.disposed) return
        if (this.dirty && this.options.visible()) { this.dirty = false; this.options.paint() }
      } finally { this.flushing = false; this.schedule() }
    })
    this.pending = handle
  }
}
