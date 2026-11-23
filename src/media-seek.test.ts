import { describe, expect, it, vi } from 'vitest'
import { seekDecoded } from './media-seek'

class Media extends EventTarget {
  readyState = 2; duration = 20; seeking = false; dataset = { track: 'test video' }; private time = 0; next = 0
  callbacks = new Map<number, VideoFrameRequestCallback>()
  get currentTime() { return this.time }
  set currentTime(value: number) { this.time = value; this.seeking = true; this.readyState = 1 }
  requestVideoFrameCallback = (callback: VideoFrameRequestCallback) => { const id = ++this.next; this.callbacks.set(id, callback); return id }
  cancelVideoFrameCallback = (id: number) => { this.callbacks.delete(id) }
  decoded() { this.seeking = false; this.readyState = 2; this.dispatchEvent(new Event('seeked')) }
  present(mediaTime = this.currentTime) { const callbacks = [...this.callbacks.values()]; this.callbacks.clear(); for (const callback of callbacks) callback(0, { mediaTime } as VideoFrameCallbackMetadata) }
  element() { return this as unknown as HTMLMediaElement }
}

describe('decoded seek presentation', () => {
  it('registers before assignment and waits for a paused video presentation after seeked', async () => {
    const media = new Media(); const done = vi.fn(); const pending = seekDecoded(media.element(), 2.5, new AbortController().signal).then(done)
    expect(media.callbacks.size).toBe(1); media.decoded(); await Promise.resolve(); expect(done).not.toHaveBeenCalled()
    media.present(); await pending; expect(done).toHaveBeenCalledOnce(); expect(media.callbacks.size).toBe(0)
  })
  it('ignores a presentation for another target and cancels an obsolete callback', async () => {
    const media = new Media(); const controller = new AbortController(); const pending = seekDecoded(media.element(), 3, controller.signal)
    const rejected = expect(pending).rejects.toMatchObject({ name: 'AbortError' }); const old = [...media.callbacks.values()][0]
    media.currentTime = 0; media.present(); expect(media.callbacks.size).toBe(1); controller.abort(); await rejected; expect(media.callbacks.size).toBe(0)
    const current = seekDecoded(media.element(), 5, new AbortController().signal); old(0, {} as VideoFrameCallbackMetadata)
    expect(media.callbacks.size).toBe(1); media.decoded(); media.present(); await current
  })
  it('does not require another presentation for an already decoded unchanged time', async () => {
    const media = new Media(); await seekDecoded(media.element(), 0, new AbortController().signal); expect(media.callbacks.size).toBe(0)
  })
  it('latches a decoded presentation before seeked without waiting for a second paused frame', async () => {
    const media = new Media(); const done = vi.fn(); const pending = seekDecoded(media.element(), 2, new AbortController().signal).then(done)
    media.present(); await Promise.resolve(); expect(done).not.toHaveBeenCalled()
    media.decoded(); await pending; expect(done).toHaveBeenCalledOnce(); expect(media.callbacks.size).toBe(0)
  })
  it('accepts a verified immutable decoded frame when compositor callbacks do not arrive', async () => {
    const close = vi.fn()
    vi.stubGlobal('VideoFrame', class { timestamp = 1_100_000; duration = 1_900_000; close = close })
    try {
      const media = new Media(); const pending = seekDecoded(media.element(), 1.205, new AbortController().signal)
      media.decoded(); await pending; expect(close).toHaveBeenCalledTimes(2); expect(media.callbacks.size).toBe(0)
    } finally { vi.unstubAllGlobals() }
  })
  it('does not accept a stale frame or invent the duration of a sparse frame', async () => {
    for (const duration of [33_333, null]) {
      const close = vi.fn(); vi.stubGlobal('VideoFrame', class { timestamp = 0; duration = duration; close = close })
      try {
        const media = new Media(); const done = vi.fn(); const pending = seekDecoded(media.element(), 2.5, new AbortController().signal).then(done)
        media.decoded(); await Promise.resolve(); expect(done).not.toHaveBeenCalled(); expect(close).toHaveBeenCalledTimes(2)
        media.present(); await pending; expect(done).toHaveBeenCalledOnce()
      } finally { vi.unstubAllGlobals() }
    }
  })
  it('rejects a new callback carrying a known old frame which cannot cover the seek target', async () => {
    const close = vi.fn(); vi.stubGlobal('VideoFrame', class { timestamp = 0; duration = 33_333; close = close })
    try {
      const media = new Media(); const done = vi.fn(); const pending = seekDecoded(media.element(), 2.5, new AbortController().signal).then(done)
      media.present(0); media.decoded(); await Promise.resolve(); expect(done).not.toHaveBeenCalled(); expect(media.callbacks.size).toBe(1)
      media.present(2.5); await pending; expect(done).toHaveBeenCalledOnce(); expect(close).toHaveBeenCalledTimes(2)
    } finally { vi.unstubAllGlobals() }
  })
  it('keeps decoded-event fallback for audio and unsupported frame callbacks', async () => {
    for (const throws of [false, true]) {
      const media = new Media(); Object.defineProperty(media, 'requestVideoFrameCallback', { value: throws ? () => { throw new Error('Unavailable') } : undefined })
      const pending = seekDecoded(media.element(), 2, new AbortController().signal); media.decoded(); await pending; expect(media.callbacks.size).toBe(0)
    }
  })
  it('fails closed with a bounded timeout when a supported video never presents', async () => {
    vi.useFakeTimers()
    try {
      const media = new Media(); const pending = seekDecoded(media.element(), 2, new AbortController().signal); const rejected = expect(pending).rejects.toThrow('Presenting test video took too long')
      media.decoded(); await vi.advanceTimersByTimeAsync(7000); await rejected; expect(media.callbacks.size).toBe(0)
    } finally { vi.useRealTimers() }
  })
})
