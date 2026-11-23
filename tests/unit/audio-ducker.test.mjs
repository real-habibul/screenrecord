import { expect, it } from 'vitest'
import { readFileSync } from 'node:fs'
import { runInNewContext } from 'node:vm'
import { MusicDucker } from '../../public/audio-ducker.js'

function mixer() {
  let Processor
  runInNewContext(readFileSync('public/audio-ducker.js', 'utf8').replace('export class MusicDucker', 'class MusicDucker'), {
    sampleRate: 48000, AudioWorkletProcessor: class { port = { onmessage: null } }, registerProcessor: (_, value) => { Processor = value },
  })
  return new Processor()
}

it('routes one shared speech envelope only to each selected ducking bus', () => {
  for (const flags of [{ enabled: true }, { enabled: false, systemEnabled: true }, { enabled: true, systemEnabled: true }, { enabled: false, systemEnabled: false }]) {
    const actual = mixer(); const reference = new MusicDucker(); reference.configure(flags.enabled || flags.systemEnabled, 12)
    actual.port.onmessage({ data: { ...flags, amountDb: 12 } })
    for (let block = 0; block < 50; block++) {
      const output = [new Float32Array(128), new Float32Array(128)]
      const inputs = [[new Float32Array(128).fill(.2), new Float32Array(128).fill(-.1)], [new Float32Array(128).fill(.15), new Float32Array(128).fill(-.25)], [new Float32Array(128).fill(.3), new Float32Array(128).fill(-.4)]]
      actual.process(inputs, [output], { active: new Float32Array([1]) })
      for (let i = 0; i < 128; i++) {
        const gain = reference.process(Math.fround(.2), Math.fround(-.1))
        for (let channel = 0; channel < 2; channel++) {
          const expected = inputs[0][channel][i] + inputs[1][channel][i] * (flags.systemEnabled ? gain : 1) + inputs[2][channel][i] * (flags.enabled ? gain : 1)
          expect(output[channel][i]).toBe(Math.fround(expected))
        }
      }
    }
  }
})

it('keeps the detector state when switching ducked targets and resets only when both turn off', () => {
  const actual = mixer(); const input = [[new Float32Array(128).fill(.2)], [], []]
  const output = [[new Float32Array(128), new Float32Array(128)]]
  actual.port.onmessage({ data: { enabled: true, amountDb: 12 } })
  actual.process(input, output, { active: new Float32Array([1]) })
  const state = { envelope: actual.ducker.envelope, gain: actual.ducker.gain }
  for (const flags of [{ enabled: true, systemEnabled: true }, { enabled: false, systemEnabled: true }]) {
    actual.port.onmessage({ data: { ...flags, amountDb: 12 } })
    expect({ envelope: actual.ducker.envelope, gain: actual.ducker.gain }).toEqual(state)
  }
  actual.port.onmessage({ data: { enabled: false, systemEnabled: false, amountDb: 12 } })
  expect(actual.ducker.envelope).toBe(0); expect(actual.ducker.gain).toBe(1)
})

it('solo monitors stay post-fader and post-duck without muting the speech detector or resetting it', () => {
  const actual = mixer(); const reference = new MusicDucker(); reference.configure(true, 12)
  actual.port.onmessage({ data: { enabled: true, systemEnabled: true, amountDb: 12 } })
  for (const monitor of ['mix', 'system', 'microphone', 'mix']) {
    const before = { gain: actual.ducker.gain, envelope: actual.ducker.envelope }
    actual.port.onmessage({ data: { monitor } })
    expect({ gain: actual.ducker.gain, envelope: actual.ducker.envelope }).toEqual(before)
    const out = [new Float32Array(128), new Float32Array(128)]
    actual.process([[new Float32Array(128).fill(.2)], [new Float32Array(128).fill(.1)], [new Float32Array(128).fill(.3)]], [out], { active: new Float32Array([1]) })
    for (let i = 0; i < 128; i++) {
      const gain = reference.process(Math.fround(.2), Math.fround(.2))
      const expected = (monitor !== 'system' ? Math.fround(.2) : 0) + (monitor !== 'microphone' ? Math.fround(.1) * gain : 0) + (monitor === 'mix' ? Math.fround(.3) * gain : 0)
      expect(out[0][i]).toBe(Math.fround(expected)); expect(out[1][i]).toBe(out[0][i])
    }
    expect(actual.ducker.gain).toBe(reference.gain)
  }
})

it('matches the renderer reference for speech attack, stereo mixing and release', () => {
  const fixture = JSON.parse(readFileSync('crates/render/testdata/ducking-reference.json', 'utf8'))
  const ducker = new MusicDucker(fixture.sampleRate); ducker.configure(true, fixture.amountDb)
  const checkpoints = new Map(fixture.checkpoints.map(point => [point.frame, point]))
  for (let frame = 0; frame < fixture.frames; frame++) {
    const mic = frame >= 4800 && frame < 24000 || frame >= 28800 && frame < 52800 ? Math.fround(((frame % 97) - 48) / 240) : 0
    const gain = ducker.process(mic, -mic)
    const expected = checkpoints.get(frame); if (!expected) continue
    expect(Math.fround(mic + Math.fround(.015) + Math.fround(.2) * gain)).toBeCloseTo(expected.left, 7)
    expect(Math.fround(-mic + Math.fround(-.025) + Math.fround(-.15) * gain)).toBeCloseTo(expected.right, 7)
    expect(ducker.envelope).toBeCloseTo(expected.envelope, 12); expect(ducker.gain).toBeCloseTo(expected.gain, 12)
  }
})

it('does not feed removed samples or seek silence into the preserved duck envelope', () => {
  let Processor
  runInNewContext(readFileSync('public/audio-ducker.js', 'utf8').replace('export class MusicDucker', 'class MusicDucker'), {
    sampleRate: 48000, AudioWorkletProcessor: class { port = { onmessage: null } }, registerProcessor: (_, value) => { Processor = value },
  })
  const actual = new Processor(); const reference = new MusicDucker(); reference.configure(true, 12)
  actual.port.onmessage({ data: { enabled: true, amountDb: 12 } })
  const block = (mic, active) => {
    const out = [new Float32Array(128), new Float32Array(128)]
    actual.process([[new Float32Array(128).fill(mic)], [], [new Float32Array(128).fill(.2)]], [out], { active: new Float32Array([active]) })
    return out
  }
  for (let n = 0; n < 30; n++) { block(.1, 1); for (let i = 0; i < 128; i++) reference.process(Math.fround(.1), Math.fround(.1)) }
  const before = { envelope: actual.ducker.envelope, gain: actual.ducker.gain }
  for (let n = 0; n < 100; n++) expect([...block(1, 0)[0]].every(sample => sample === 0)).toBe(true)
  expect({ envelope: actual.ducker.envelope, gain: actual.ducker.gain }).toEqual(before)
  const kept = block(0, 1)
  for (let i = 0; i < 128; i++) expect(kept[0][i]).toBeCloseTo(Math.fround(Math.fround(.2) * reference.process(0, 0)), 7)
  actual.port.onmessage({ data: 'reset' }); expect(actual.ducker.gain).toBe(1); expect(actual.ducker.envelope).toBe(0)
})

it('updates reduction without restarting speech detection and resets when disabled', () => {
  const ducker = new MusicDucker(); ducker.configure(true, 12)
  for (let i = 0; i < 4800; i++) ducker.process(.2, -.2)
  const envelope = ducker.envelope; const gain = ducker.gain
  ducker.configure(true, 24); expect(ducker.envelope).toBe(envelope); expect(ducker.gain).toBe(gain)
  ducker.process(.2, -.2); expect(ducker.gain).toBeLessThan(gain)
  ducker.configure(false, 24); expect(ducker.process(1, -1)).toBe(1); expect(ducker.envelope).toBe(0)
})
