import { describe, expect, it } from 'vitest'
import { execFileSync } from 'node:child_process'
import { Buffer } from 'node:buffer'
import { existsSync } from 'node:fs'
import path from 'node:path'
import process from 'node:process'
import { LookaheadLimiter } from '../../public/audio-limiter.js'

const engine = process.env.SCREENRECORD_FFMPEG ?? path.resolve('src-tauri/resources/ffmpeg.exe')
const rate = 48_000
function render(input) {
  const limiter = new LookaheadLimiter(rate)
  const frames = input.length / 2
  const output = new Float32Array(input.length)
  for (let frame = 0; frame < frames + limiter.latencyFrames; frame++) {
    limiter.process(input[frame * 2] ?? 0, input[frame * 2 + 1] ?? 0)
    if (frame >= limiter.latencyFrames) {
      const index = (frame - limiter.latencyFrames) * 2
      output[index] = limiter.left; output[index + 1] = limiter.right
    }
  }
  return output
}

describe('preview limiter', () => {
  it('keeps quiet audio at unity and resets its lookahead on seek', () => {
    const input = new Float32Array(4800).fill(.1)
    expect(render(input)).toEqual(input)
    const limiter = new LookaheadLimiter(rate)
    for (let frame = 0; frame < 1000; frame++) limiter.process(2, -2)
    limiter.reset()
    for (let frame = 0; frame < limiter.latencyFrames; frame++) {
      limiter.process(.1, -.1)
      expect(limiter.left).toBe(0); expect(limiter.right).toBe(0)
    }
    limiter.process(.1, -.1)
    expect(limiter.left).toBeCloseTo(.1, 12); expect(limiter.right).toBeCloseTo(-.1, 12)
  })

  it.skipIf(!existsSync(engine))('matches pinned FFmpeg for stereo transients, clipping and release', () => {
    const input = new Float32Array(rate * 2)
    for (let frame = 0; frame < rate; frame++) {
      const envelope = frame < 4000 ? .2 : frame < 24000 ? 2.3 : frame < 32000 ? 0 : .4
      input[frame * 2] = Math.sin(frame * 2 * Math.PI * 997 / rate) * envelope + (frame % 977 === 0 ? 1.7 : 0)
      input[frame * 2 + 1] = Math.sin(frame * 2 * Math.PI * 193 / rate) * envelope * .7
    }
    const pcm = execFileSync(engine, ['-hide_banner', '-loglevel', 'error', '-f', 'f32le', '-ar', String(rate), '-ac', '2', '-i', 'pipe:0', '-af', 'alimiter=level=0:limit=0.95:attack=5:release=50:asc=0:latency=1', '-f', 'f32le', '-c:a', 'pcm_f32le', 'pipe:1'], { input: Buffer.from(input.buffer), windowsHide: true, maxBuffer: 2_000_000 })
    const reference = new Float32Array(pcm.buffer, pcm.byteOffset, pcm.byteLength / 4)
    const actual = render(input)
    expect(actual.length).toBe(reference.length)
    let largestError = 0; let peak = 0
    for (let sample = 0; sample < actual.length; sample++) {
      largestError = Math.max(largestError, Math.abs(actual[sample] - reference[sample]))
      peak = Math.max(peak, Math.abs(actual[sample]))
    }
    expect(largestError).toBeLessThan(1e-6)
    expect(peak).toBeLessThanOrEqual(.950001)
  })
})
