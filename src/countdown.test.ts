import { afterEach, expect, it, vi } from 'vitest'
import { runCountdown } from './countdown'
afterEach(() => vi.useRealTimers())
it('does not start before the monotonic deadline and zero starts immediately', async () => {
  vi.useFakeTimers(); const tick = vi.fn(); const controller = new AbortController(); let completed = false
  const countdown = runCountdown(3, controller.signal, tick).then(value => { completed = value })
  await vi.advanceTimersByTimeAsync(2999); expect(completed).toBe(false); expect(tick.mock.calls[0]).toEqual([3])
  await vi.advanceTimersByTimeAsync(1); await countdown; expect(completed).toBe(true)
  expect(await runCountdown(0, controller.signal, tick)).toBe(true)
})
it('cancellation at the last tick cannot start recording or disturb a restarted countdown', async () => {
  vi.useFakeTimers(); const first = new AbortController(); const tick = vi.fn(); const old = runCountdown(3, first.signal, tick)
  await vi.advanceTimersByTimeAsync(2900); first.abort(); expect(await old).toBe(false)
  const next = runCountdown(5, new AbortController().signal, tick); await vi.advanceTimersByTimeAsync(5000); expect(await next).toBe(true)
  const cancelled = new AbortController(); cancelled.abort(); expect(await runCountdown(0, cancelled.signal, tick)).toBe(false)
})
