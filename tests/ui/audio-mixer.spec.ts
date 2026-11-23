import { expect, test } from '@playwright/test'
import { readFileSync } from 'node:fs'

for (const routing of [
  { name: 'legacy music only', enabled: true },
  { name: 'computer audio only', enabled: false, systemEnabled: true },
  { name: 'computer audio and music', enabled: true, systemEnabled: true },
  { name: 'microphone solo', enabled: true, systemEnabled: true, monitor: 'microphone' },
  { name: 'computer solo with active microphone detector', enabled: true, systemEnabled: true, monitor: 'system' },
]) test(`actual AudioWorklet ${routing.name} matches renderer PCM and freezes its detector throughout a gated removed span`, async ({ page }) => {
  await page.goto('/')
  const fixture = JSON.parse(readFileSync('crates/render/testdata/ducking-reference.json', 'utf8'))
  const result = await page.evaluate(async ({ fixture, routing }) => {
    async function render(gap: number) {
      const context = new OfflineAudioContext(2, fixture.frames + gap, fixture.sampleRate)
      await context.audioWorklet.addModule('/audio-ducker.js')
      const mixer = new AudioWorkletNode(context, 'screenrecord-music-mixer', { numberOfInputs: 3, numberOfOutputs: 1, outputChannelCount: [2], channelCount: 2, channelCountMode: 'explicit' })
      mixer.port.postMessage({ enabled: routing.enabled, systemEnabled: routing.systemEnabled, amountDb: fixture.amountDb }); mixer.connect(context.destination)
      if (routing.monitor) mixer.port.postMessage({ monitor: routing.monitor })
      const active = mixer.parameters.get('active')!; active.setValueAtTime(1, 0)
      if (gap) { active.setValueAtTime(0, 9600 / fixture.sampleRate); active.setValueAtTime(1, (9600 + gap) / fixture.sampleRate) }
      for (let bus = 0; bus < 3; bus++) {
        const buffer = context.createBuffer(2, fixture.frames + gap, fixture.sampleRate)
        for (let frame = 0; frame < buffer.length; frame++) {
          const removed = gap && frame >= 9600 && frame < 9600 + gap
          const source = frame >= 9600 + gap ? frame - gap : frame
          const mic = removed ? 1 : source >= 4800 && source < 24000 || source >= 28800 && source < 52800 ? Math.fround(((source % 97) - 48) / 240) : 0
          buffer.getChannelData(0)[frame] = bus === 0 ? mic : bus === 1 ? .015 : .2
          buffer.getChannelData(1)[frame] = bus === 0 ? -mic : bus === 1 ? -.025 : -.15
        }
        const source = context.createBufferSource(); source.buffer = buffer; source.connect(mixer, 0, bus); source.start()
      }
      return context.startRendering()
    }
    const contiguous = await render(0); const gapped = await render(25_600)
    let referenceError = 0; let cutError = 0; let removedPeak = 0
    for (const point of fixture.checkpoints) {
      const frame = point.frame
      const mic = frame >= 4800 && frame < 24000 || frame >= 28800 && frame < 52800 ? Math.fround(((frame % 97) - 48) / 240) : 0
      for (let channel = 0; channel < 2; channel++) {
        const expected = (routing.monitor !== 'system' ? channel ? -mic : mic : 0) + (routing.monitor !== 'microphone' ? Math.fround(channel ? -.025 : .015) * (routing.systemEnabled ? point.gain : 1) : 0) + (!routing.monitor ? Math.fround(channel ? -.15 : .2) * (routing.enabled ? point.gain : 1) : 0)
        referenceError = Math.max(referenceError, Math.abs(contiguous.getChannelData(channel)[frame] - Math.fround(expected)))
      }
    }
    for (let frame = 0; frame < fixture.frames; frame++) for (let channel = 0; channel < 2; channel++) cutError = Math.max(cutError, Math.abs(contiguous.getChannelData(channel)[frame] - gapped.getChannelData(channel)[frame < 9600 ? frame : frame + 25_600]))
    for (let frame = 9600; frame < 9600 + 25_600; frame++) removedPeak = Math.max(removedPeak, Math.abs(gapped.getChannelData(0)[frame]), Math.abs(gapped.getChannelData(1)[frame]))
    return { referenceError, cutError, removedPeak }
  }, { fixture, routing })
  expect(result.referenceError).toBeLessThan(1e-7); expect(result.cutError).toBeLessThan(1e-7); expect(result.removedPeak).toBe(0)
})
