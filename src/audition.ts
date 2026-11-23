export type AudioMonitor = 'mix' | 'microphone' | 'system'
export interface AuditionRange { start: number; end: number }

export function fitAuditionRange(range: AuditionRange, duration: number): AuditionRange {
  if (!Number.isFinite(duration) || duration <= 0) return { start: 0, end: 0 }
  const minimum = Math.min(0.1, duration)
  const from = Number.isFinite(range.start) ? range.start : 0
  const to = Number.isFinite(range.end) ? range.end : duration
  const start = Math.min(duration - minimum, Math.max(0, from))
  return { start, end: Math.min(duration, Math.max(start + minimum, to)) }
}

export function auditionFromPlayhead(time: number, duration: number, length = 3): AuditionRange {
  const width = Math.min(duration, Math.max(0.1, length))
  const start = Math.max(0, Math.min(time, duration - width))
  return fitAuditionRange({ start, end: start + width }, duration)
}
