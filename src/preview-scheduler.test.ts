import { describe, expect, it } from 'vitest'
import { PreviewScheduler, type FrameSource } from './preview-scheduler'

function fixture() {
  let next = 0; let visible = true; let paints = 0; let clocks = 0
  const frames = new Map<number, FrameRequestCallback>()
  const video = new Map<number, VideoFrameRequestCallback>()
  const source: FrameSource = { requestVideoFrameCallback: callback => { const id = ++next; video.set(id, callback); return id }, cancelVideoFrameCallback: id => { video.delete(id) } }
  let sources = [source]; let onClock: () => void = () => {}
  const observations: number[] = []
  const scheduler = new PreviewScheduler({ requestFrame: callback => { const id = ++next; frames.set(id, callback); return id }, cancelFrame: id => { frames.delete(id) }, sources: () => sources, visible: () => visible, clock: () => { clocks++; onClock() }, paint: () => { paints++ }, presented: (_source, metadata) => observations.push(metadata.mediaTime) })
  const tick = () => { const callbacks = [...frames.values()]; frames.clear(); for (const callback of callbacks) callback(100) }
  const present = (count: number) => { const callbacks = [...video.values()]; video.clear(); for (const callback of callbacks) callback(100, { presentedFrames: count, mediaTime: count / 30 } as VideoFrameCallbackMetadata) }
  return { scheduler, frames, video, source, tick, present, observations, setVisible: (value: boolean) => { visible = value }, setSources: (value: FrameSource[]) => { sources = value }, onClock: (value: () => void) => { onClock = value }, counts: () => ({ paints, clocks }) }
}

describe('preview presentation scheduler', () => {
  it('coalesces edit bursts and stops requesting frames once a paused repaint completes', () => {
    const f = fixture(); for (let i = 0; i < 100; i++) f.scheduler.invalidate()
    expect(f.frames.size).toBe(1); f.tick(); expect(f.counts()).toEqual({ paints: 1, clocks: 0 }); expect(f.frames.size).toBe(0)
  })
  it('keeps every clock tick but only paints new screen frames and explicit overlay changes', () => {
    const f = fixture(); f.scheduler.start(); f.tick()
    for (let frame = 1; frame <= 30; frame++) { f.present(frame); f.tick(); f.tick() }
    expect(f.counts()).toEqual({ paints: 31, clocks: 61 }); expect(f.frames.size).toBe(1); expect(f.video.size).toBe(1)
    f.scheduler.invalidate(); f.tick(); expect(f.counts().paints).toBe(32)
    f.present(30); f.tick(); expect(f.counts().paints).toBe(32)
  })
  it('observes later paused presentations without a clock or permanent RAF loop', () => {
    const f = fixture(); f.scheduler.paintNow(); expect(f.video.size).toBe(1); expect(f.frames.size).toBe(0)
    f.present(4); f.tick(); expect(f.counts()).toEqual({ paints: 2, clocks: 0 }); expect(f.frames.size).toBe(0); expect(f.video.size).toBe(1)
    const old = [...f.video.values()][0]; f.scheduler.stop(); old(0, { presentedFrames: 5 } as VideoFrameCallbackMetadata)
    expect(f.frames.size).toBe(0); expect(f.observations).toEqual([4 / 30])
  })
  it('ignores cancelled RAF and video callbacks after stop, restart and disposal', () => {
    const f = fixture(); f.scheduler.start(); const staleFrame = [...f.frames.values()][0]; f.tick(); const staleVideo = [...f.video.values()][0]
    f.scheduler.stop(); f.scheduler.start(); staleFrame(100); staleVideo(100, { presentedFrames: 99 } as VideoFrameCallbackMetadata)
    expect(f.observations).toEqual([])
    expect(f.frames.size).toBe(1); expect(f.video.size).toBe(0); f.tick(); expect(f.counts().paints).toBe(2)
    f.present(2); expect(f.observations).toEqual([2 / 30])
    f.scheduler.dispose(); staleVideo(100, { presentedFrames: 100 } as VideoFrameCallbackMetadata); f.scheduler.invalidate(); expect(f.frames.size).toBe(0); expect(f.video.size).toBe(0)
    expect(f.observations).toEqual([2 / 30])
  })
  it('does not paint an obsolete scene when a hard cut stops scheduling during the clock callback', () => {
    const f = fixture(); f.onClock(() => f.scheduler.stop()); f.scheduler.start(); f.tick()
    expect(f.counts()).toEqual({ paints: 0, clocks: 1 }); expect(f.frames.size).toBe(0); expect(f.video.size).toBe(0)
    f.scheduler.invalidate(); f.tick(); expect(f.counts().paints).toBe(1)
  })
  it('falls back to RAF if frame callbacks are unavailable or fail at runtime', () => {
    for (const source of [{}, { requestVideoFrameCallback: () => { throw new Error('Unsupported runtime') }, cancelVideoFrameCallback: () => {} }]) {
      const f = fixture(); f.setSources([source]); f.scheduler.start(); f.tick(); f.tick(); f.tick()
      expect(f.counts()).toEqual({ paints: 3, clocks: 3 }); f.scheduler.stop(); expect(f.frames.size).toBe(0)
    }
  })
  it('paints a new camera frame independently and removes detached source callbacks', () => {
    const f = fixture(); const camera = new Map<number, VideoFrameRequestCallback>(); let id = 0
    const cameraSource: FrameSource = { requestVideoFrameCallback: callback => { camera.set(++id, callback); return id }, cancelVideoFrameCallback: handle => { camera.delete(handle) } }
    f.setSources([f.source, cameraSource]); f.scheduler.start(); f.tick()
    const callback = [...camera.values()][0]; camera.clear(); callback(100, { presentedFrames: 1 } as VideoFrameCallbackMetadata); f.tick()
    expect(f.counts().paints).toBe(2); f.setSources([f.source]); f.tick(); expect(camera.size).toBe(0)
    callback(100, { presentedFrames: 2 } as VideoFrameCallbackMetadata); f.tick(); expect(f.counts().paints).toBe(2)
  })
  it('retains invalidations while hidden and resubscribes without a stale presentation on visibility return', () => {
    const f = fixture(); f.scheduler.start(); f.tick(); const old = [...f.video.values()][0]
    f.setVisible(false); f.scheduler.visibilityChanged(); f.tick(); expect(f.counts().paints).toBe(1); expect(f.video.size).toBe(0)
    f.setVisible(true); f.scheduler.visibilityChanged(); old(100, { presentedFrames: 12 } as VideoFrameCallbackMetadata); f.tick()
    expect(f.counts().paints).toBe(2); expect(f.video.size).toBe(1)
    f.scheduler.stop(); f.setVisible(false); f.scheduler.invalidate(); expect(f.frames.size).toBe(0); f.setVisible(true); f.scheduler.visibilityChanged(); f.tick(); expect(f.counts().paints).toBe(3)
  })
})
