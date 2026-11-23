import { expect, it, vi } from 'vitest'
import { audioSignature, createAudioAnalysisStore } from './audio-analysis'
import { waveformColumns } from './waveform'
import type { AudioAnalysis, EditSettings, Project } from './types'
import type { command } from './api'

const data: AudioAnalysis = { sourceDuration: 6, bins: [{ min: -.1, max: .1 }, { min: -.2, max: .2 }, { min: -.9, max: .9 }, { min: -.9, max: .9 }, { min: -.3, max: .3 }, { min: -.4, max: .4 }], peak: .4, rms: .1, analyzedSeconds: 4, recommendedGain: 1.25 }
it('projects source peaks through cuts without drawing discarded source bins and caps output allocation', () => {
  const spans = [{ sourceStart: 0, sourceEnd: 2, start: 0, end: 2 }, { sourceStart: 4, sourceEnd: 6, start: 2, end: 4 }]
  expect(waveformColumns(data, spans, 4, 4).map(value => value.max)).toEqual([.1, .2, .3, .4])
  expect(waveformColumns(data, spans, 100_000, 4)).toHaveLength(4096)
})
it('marks only level-affecting timing as stale, leaving volume and gain independent', () => {
  const edits = { trimStart: 0, trimEnd: 6, videoClips: [] as EditSettings['videoClips'], music: [], microphoneVolume: 1, audioProcessing: { microphoneGain: 1 } }
  const signature = audioSignature(edits, 'microphone')
  edits.microphoneVolume = .3
  edits.audioProcessing.microphoneGain = 2
  expect(audioSignature(edits, 'microphone')).toBe(signature)
  edits.videoClips = [{ id: 'kept', start: 0, end: 2 }]; expect(audioSignature(edits, 'microphone')).not.toBe(signature)
})
it('reuses only a finished analysis for the same project, source and cuts', async () => {
  const invoke = vi.fn().mockResolvedValue(data)
  const store = createAudioAnalysisStore(invoke)
  const project = { id: 'project-a' } as Project
  expect(store.cached(project.id, 'microphone', 'full')).toBeUndefined()
  await store.analyze(project, 'microphone', 'full')
  expect(store.cached(project.id, 'microphone', 'full')).toBe(data)
  expect(store.cached('project-b', 'microphone', 'full')).toBeUndefined()
  expect(store.cached(project.id, 'system', 'full')).toBeUndefined()
  expect(store.cached(project.id, 'microphone', 'trimmed')).toBeUndefined()
  expect(invoke).toHaveBeenCalledTimes(1)

  invoke.mockRejectedValueOnce(new Error('Decode failed'))
  const refreshing = store.analyze(project, 'microphone', 'full')
  expect(store.cached(project.id, 'microphone', 'full')).toBeUndefined()
  await expect(refreshing).rejects.toThrow('Decode failed')
  expect(store.states.microphone.attemptedSignature).toBe('full')
  expect(store.cached(project.id, 'microphone', 'full')).toBeUndefined()
  await store.analyze(project, 'microphone', 'trimmed', true)
  expect(invoke.mock.calls.at(-1)).toEqual(['analyze_audio', { id: project.id, source: 'microphone', jobId: expect.any(String), forceRefresh: true }])
  store.clear()
  expect(store.cached(project.id, 'microphone', 'trimmed')).toBeUndefined()
})
it('awaits cancellation and ignores late progress and results before the next analysis', async () => {
  let rejectFirst!: (error: Error) => void
  let count = 0
  const invoke = vi.fn((name: string) => name === 'analyze_audio' && ++count === 1 ? new Promise((_, reject) => { rejectFirst = reject }) : name === 'cancel_audio_analysis' ? (rejectFirst(new Error('cancelled')), Promise.resolve()) : Promise.resolve(data)) as unknown as typeof command
  const store = createAudioAnalysisStore(invoke)
  const first = store.analyze({ id: 'first' } as Project, 'microphone', 'old')
  await store.cancel(); await expect(first).resolves.toBeNull()
  store.progress({ jobId: 'obsolete', projectId: 'first', source: 'microphone', progress: .8 }); expect(store.states.microphone.state).toBe('cancelled')
  await store.analyze({ id: 'second' } as Project, 'microphone', 'new')
  expect(store.states.microphone.data).toEqual(data); expect(store.states.microphone.signature).toBe('new')
})
it('shares an in-progress scan only for the matching source, project and cuts', async () => {
  let finish!: (result: AudioAnalysis) => void
  const invoke = vi.fn().mockImplementation(() => new Promise<AudioAnalysis>(resolve => { finish = resolve }))
  const store = createAudioAnalysisStore(invoke)
  const scan = store.analyze({ id: 'one' } as Project, 'microphone', 'full')
  const pending = store.pending('one', 'microphone', 'full')
  expect(pending).toBeInstanceOf(Promise)
  expect(store.pending('two', 'microphone', 'full')).toBeUndefined()
  expect(store.pending('one', 'system', 'full')).toBeUndefined()
  expect(store.pending('one', 'microphone', 'trimmed')).toBeUndefined()
  finish(data)
  await Promise.all([scan, pending])
  expect(invoke).toHaveBeenCalledTimes(1)
  expect(store.cached('one', 'microphone', 'full')).toBe(data)
})
it('does not finish cancellation until a late completed analysis releases its reader', async () => {
  let finish!: (value: AudioAnalysis) => void
  const invoke = ((name: string) => name === 'analyze_audio' ? new Promise<AudioAnalysis>(resolve => { finish = resolve }) : Promise.resolve()) as typeof command
  const store = createAudioAnalysisStore(invoke)
  const result = store.analyze({ id: 'project' } as Project, 'system', 'obsolete')
  const pending = store.pending('project', 'system', 'obsolete')
  let cancelled = false; const stopping = store.cancel().then(() => { cancelled = true })
  await Promise.resolve(); expect(cancelled).toBe(false)
  finish(data); await stopping; await pending
  expect(await result).toBeNull()
  expect(store.cached('project', 'system', 'obsolete')).toBeUndefined()
  expect(store.states.system.signature).toBeUndefined()
})
