import { expect, it } from 'vitest'
import { scheduleVolume } from './preview-volume'
import { PreviewAudio } from './preview-audio'
import type { AudioProcessing } from './types'

const parameter = () => {
  const events: Array<[string, number, number?]> = []
  return { events, param: { value: 0, cancelScheduledValues(time: number) { events.push(['cancel', time]) }, setValueAtTime(value: number, time: number) { events.push(['set', value, time]) }, linearRampToValueAtTime(value: number, time: number) { events.push(['ramp', value, time]) } } as unknown as AudioParam }
}

it('interpolates toward a key beyond the source span and cancels an obsolete schedule at the new anchor', () => {
  const { param, events } = parameter()
  const keys = [{ id: 'a', time: 0, volume: 0 }, { id: 'b', time: 4, volume: 1 }]
  scheduleVolume(param, keys, 1, 2, 10)
  expect(events).toEqual([['cancel', 10], ['set', .25, 10], ['ramp', .5, 11]])
  events.length = 0; scheduleVolume(param, keys, 3, 5, 11)
  expect(events).toEqual([['cancel', 11], ['set', .75, 11], ['ramp', 1, 12], ['ramp', 1, 13]])
  events.length = 0; scheduleVolume(param, [], 1, 2, 20)
  expect(events).toEqual([['cancel', 20], ['set', 1, 20]])
})

it('schedules once per span, refreshes live edits and rollback, then reanchors on pause and source cuts', () => {
  const preview = new PreviewAudio(undefined, undefined, () => {})
  const mic = parameter(); const system = parameter(); const music = parameter(); const gate = parameter()
  const context = { currentTime: 0, state: 'closed' }
  Object.assign(preview, { context, mixer: { parameters: new Map([['microphoneVolume', mic.param], ['systemVolume', system.param], ['musicVolume', music.param], ['active', gate.param]]), port: { postMessage() {} } }, inputGate: { gain: gate.param }, outputGate: { gain: gate.param } })
  const processing: AudioProcessing = { microphoneGain: 1, systemGain: 1, ducking: { enabled: false, amountDb: 12 }, volumeKeyframes: { microphone: [{ id: 'a', time: 0, volume: 0 }, { id: 'b', time: 2, volume: 1 }] } }
  preview.setProcessing(processing); preview.gateSpan(0, 1)
  expect(mic.events[1][1]).toBeCloseTo(preview.latencySeconds / 2, 9)
  const initial = mic.events.length
  for (let frame = 1; frame < 30; frame++) { context.currentTime = frame / 60; preview.gateSpan(frame / 60, 1) }
  expect(mic.events.length).toBe(initial)
  processing.volumeKeyframes!.microphone![1].volume = .4; preview.setProcessing(processing); preview.gateSpan(.5, 1)
  expect(mic.events.at(-1)?.[1]).toBe(.2)
  processing.volumeKeyframes!.microphone![1].volume = 1; preview.setProcessing(processing); preview.gateSpan(.5, 1)
  expect(mic.events.at(-1)?.[1]).toBe(.5)
  const beforeRepeat = mic.events.length; preview.setProcessing(structuredClone(processing)); preview.gateSpan(.5, 1); expect(mic.events.length).toBe(beforeRepeat)
  preview.pause(); context.currentTime = 2; preview.gateSpan(1.5, 2)
  expect(mic.events.slice(-3)[1][1]).toBeCloseTo((1.5 + preview.latencySeconds) / 2, 9)
  expect(system.events.at(-1)?.[1]).toBe(1); expect(music.events.at(-1)?.[1]).toBe(1)
})
