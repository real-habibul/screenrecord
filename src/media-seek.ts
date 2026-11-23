export async function seekDecoded(media: HTMLMediaElement, time: number, signal: AbortSignal, force = false): Promise<void> {
  if (signal.aborted) throw new DOMException('Seek superseded', 'AbortError')
  if (media.readyState < 1) await waitFor(media, () => media.readyState >= 1, ['loadedmetadata'], signal)
  const target = Math.max(0, Math.min(time, Number.isFinite(media.duration) ? media.duration : time))
  if (!force && Math.abs(media.currentTime - target) < 1e-6 && !media.seeking && media.readyState >= 2) return
  const presentation = preparePresentation(media, target, signal)
  const decoded = waitFor(media, () => !media.seeking && media.readyState >= 2 && Math.abs(media.currentTime - target) < 1e-5, ['seeked', 'loadeddata', 'canplay'], signal, false)
  try {
    media.currentTime = target
    await Promise.all([decoded.then(() => { if (presentation && decodedFrameCovers(media, target)) presentation.cancel() }), presentation?.ready])
  } finally { presentation?.cancel() }
}

function decodedFrameCovers(media: HTMLMediaElement, target: number) {
  const frame = frameInfo(media)
  return frame != null && frame.duration != null && frame.duration > 0
    && target * 1e6 >= frame.timestamp - 1 && target * 1e6 < frame.timestamp + frame.duration
}

function frameInfo(media: HTMLMediaElement) {
  if (typeof VideoFrame !== 'function') return null
  let frame: VideoFrame | undefined
  try {
    frame = new VideoFrame(media as HTMLVideoElement)
    return Number.isFinite(frame.timestamp) ? { timestamp: frame.timestamp, duration: frame.duration } : null
  } catch { return null }
  finally { frame?.close() }
}

function preparePresentation(media: HTMLMediaElement, target: number, signal: AbortSignal) {
  const video = media as HTMLVideoElement
  if (typeof video.requestVideoFrameCallback !== 'function' || typeof video.cancelVideoFrameCallback !== 'function') return null
  const previous = frameInfo(media)
  let handle: number | null = null; let timeout: ReturnType<typeof setTimeout> | undefined; let settled = false
  let cancel = () => {}
  const ready = new Promise<void>((resolve, reject) => {
    const cleanup = () => { if (handle !== null) { try { video.cancelVideoFrameCallback(handle) } catch { /* Aborted callbacks are also guarded by settled. */ } }; handle = null; clearTimeout(timeout); signal.removeEventListener('abort', aborted) }
    const finish = (error?: Error) => { if (settled) return; settled = true; cleanup(); if (error) reject(error); else resolve() }
    const aborted = () => finish(new DOMException('Seek superseded', 'AbortError'))
    const subscribe = () => {
      try { handle = video.requestVideoFrameCallback((_now, metadata) => {
        handle = null
        if (settled) return
        const oldFrame = previous && Math.abs(metadata.mediaTime * 1e6 - previous.timestamp) <= 1
          && (target * 1e6 < previous.timestamp - 1 || previous.duration != null && previous.duration > 0 && target * 1e6 >= previous.timestamp + previous.duration)
        if (!oldFrame && Math.abs(video.currentTime - target) < 1e-5) finish()
        else subscribe()
      }) } catch { finish() }
    }
    cancel = () => finish()
    signal.addEventListener('abort', aborted, { once: true })
    timeout = setTimeout(() => finish(new Error(`Presenting ${media.dataset.track ?? 'video'} took too long. Try playback again.`)), 7000)
    subscribe()
    if (signal.aborted) aborted()
  })
  return { ready, cancel }
}

function waitFor(media: HTMLMediaElement, ready: () => boolean, events: string[], signal: AbortSignal, immediate = true): Promise<void> {
  return new Promise((resolve, reject) => {
    let timeout: ReturnType<typeof setTimeout>
    const cleanup = () => { clearTimeout(timeout); events.forEach(event => media.removeEventListener(event, check)); media.removeEventListener('error', failed); signal.removeEventListener('abort', aborted) }
    const check = () => { if (ready()) { cleanup(); resolve() } }
    const failed = () => { cleanup(); reject(new Error(`Cannot seek ${media.dataset.track ?? 'recording'} to decoded media.`)) }
    const aborted = () => { cleanup(); reject(new DOMException('Seek superseded', 'AbortError')) }
    events.forEach(event => media.addEventListener(event, check)); media.addEventListener('error', failed); signal.addEventListener('abort', aborted, { once: true })
    timeout = setTimeout(() => { cleanup(); reject(new Error(`Seeking ${media.dataset.track ?? 'recording'} took too long. Try playback again.`)) }, 7000)
    if (signal.aborted) aborted(); else if (immediate) check()
  })
}
