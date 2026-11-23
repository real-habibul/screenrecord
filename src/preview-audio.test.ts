import { expect, it } from 'vitest'
import { PreviewAudio } from './preview-audio'

it('sends independent ducking choices and defaults older projects to music only', () => {
  for (const hasMicrophone of [true, false]) {
    const messages: unknown[] = []
    const preview = new PreviewAudio(hasMicrophone ? {} as HTMLAudioElement : undefined, {} as HTMLAudioElement, () => {})
    const state = preview as unknown as { mixer: { port: { postMessage: (data: unknown) => void } } }
    state.mixer = { port: { postMessage: data => messages.push(data) } }
    for (const ducking of [{ enabled: true, amountDb: 12 }, { enabled: false, systemEnabled: true, amountDb: 18 }, { enabled: true, systemEnabled: true, amountDb: 24 }]) {
      preview.setProcessing({ microphoneGain: 1, systemGain: 1, ducking })
      expect(messages.at(-1)).toEqual({ enabled: hasMicrophone && ducking.enabled, systemEnabled: hasMicrophone && !!ducking.systemEnabled, amountDb: ducking.amountDb })
    }
  }
})

it('never extends or reopens a cut gate when HTML media clocks lag behind the render clock', () => {
  const events: Array<{ value: number; time: number }> = []
  const gate = () => ({ gain: { cancelScheduledValues() {}, setValueAtTime(value: number, time: number) { events.push({ value, time }) } } })
  const preview = new PreviewAudio(undefined, undefined, () => {})
  const state = preview as unknown as { context: { currentTime: number; state: string }; inputGate: ReturnType<typeof gate>; outputGate: ReturnType<typeof gate> }
  state.context = { currentTime: 0, state: 'closed' }; state.inputGate = gate(); state.outputGate = gate()
  preview.gateSpan(0, 1)
  const firstDeadline = events[1].time
  state.context.currentTime = .9; events.length = 0; preview.gateSpan(.2, 1)
  expect(events[1].time).toBe(firstDeadline)
  state.context.currentTime = 1.1; events.length = 0; preview.gateSpan(.5, 1)
  expect(events.every(event => event.value === 0)).toBe(true)
  preview.pause(); events.length = 0; preview.gateSpan(.5, 1)
  expect(events[0].value).toBe(1)
})
