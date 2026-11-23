export function runCountdown(seconds: number, signal: AbortSignal, tick: (remaining: number) => void): Promise<boolean> {
  return new Promise(resolve => {
    let timer: ReturnType<typeof setTimeout> | undefined
    let settled = false
    const deadline = performance.now() + Math.max(0, seconds) * 1000
    const finish = (started: boolean) => { if (settled) return; settled = true; clearTimeout(timer); signal.removeEventListener('abort', cancel); resolve(started) }
    const cancel = () => finish(false)
    const update = () => {
      if (signal.aborted) { finish(false); return }
      const remaining = Math.ceil((deadline - performance.now()) / 1000)
      if (remaining <= 0) { finish(true); return }
      tick(remaining); if (!settled) timer = setTimeout(update, Math.min(100, deadline - performance.now()))
    }
    signal.addEventListener('abort', cancel, { once: true }); update()
  })
}
