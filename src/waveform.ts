import type { AudioAnalysis } from './types'
export interface WaveformSpan { sourceStart: number; sourceEnd: number; start: number; end: number }

export function waveformColumns(analysis: AudioAnalysis, spans: WaveformSpan[], width: number, duration: number) {
  const count = Math.max(1, Math.min(4096, Math.ceil(width)))
  const columns = Array.from({ length: count }, () => ({ min: 0, max: 0 }))
  if (analysis.sourceDuration <= 0 || duration <= 0 || !analysis.bins.length) return columns
  for (const span of spans) {
    const first = Math.max(0, Math.floor(span.start / duration * count)); const last = Math.min(count, Math.ceil(span.end / duration * count))
    for (let x = first; x < last; x++) {
      const from = Math.max(span.start, x / count * duration); const to = Math.min(span.end, (x + 1) / count * duration)
      const sourceFrom = span.sourceStart + from - span.start; const sourceTo = span.sourceStart + to - span.start
      const left = Math.max(0, Math.floor(sourceFrom / analysis.sourceDuration * analysis.bins.length)); const right = Math.min(analysis.bins.length, Math.ceil(sourceTo / analysis.sourceDuration * analysis.bins.length))
      for (let i = left; i < right; i++) { columns[x].min = Math.min(columns[x].min, analysis.bins[i].min); columns[x].max = Math.max(columns[x].max, analysis.bins[i].max) }
    }
  }
  return columns
}
