import { markRaw, reactive } from 'vue'
import { command, errorText } from './api'
import type { AudioAnalysis, EditSettings, Project } from './types'

export const MAX_RECORDED_GAIN = 2

export interface AudioAnalysisState { projectId: string; state: 'loading' | 'ready' | 'error' | 'cancelled'; progress: number; data?: AudioAnalysis; signature?: string; attemptedSignature?: string; error?: string }
export function audioSignature(edits: Pick<EditSettings, 'trimStart' | 'trimEnd' | 'videoClips' | 'music'>, source: string): string {
  const clip = source.startsWith('music:') ? edits.music.find(item => item.id === source.slice(6)) : undefined
  return JSON.stringify([edits.trimStart, edits.trimEnd, edits.videoClips, clip && [clip.path, clip.start, clip.sourceStart, clip.duration]])
}
export function audioSources(project: Project): string[] {
  return [...(project.microphonePath ? ['microphone'] : []), ...(project.systemAudioPath ? ['system'] : []), ...project.edits.music.map(clip => `music:${clip.id}`)]
}

export function createAudioAnalysisStore(invoke: typeof command = command) {
  const states = reactive<Record<string, AudioAnalysisState>>({})
  let active: { jobId: string; projectId: string; source: string; signature: string; result: Promise<AudioAnalysis>; cancelled: boolean } | null = null
  let revision = 0
  function cached(projectId: string, source: string, signature: string): AudioAnalysis | undefined {
    const result = states[source]
    if (result?.state === 'ready' && result.projectId === projectId && result.signature === signature) return result.data
  }
  function pending(projectId: string, source: string, signature: string): Promise<void> | undefined {
    const job = active
    if (!job || job.cancelled || job.projectId !== projectId || job.source !== source || job.signature !== signature || states[source]?.state !== 'loading') return
    return job.result.then(() => undefined, error => { if (!job.cancelled) throw error })
  }
  async function cancel() {
    revision++
    const job = active; if (!job) return
    job.cancelled = true
    await invoke('cancel_audio_analysis', { jobId: job.jobId })
    await job.result.catch(() => undefined)
  }
  async function analyze(project: Project, source: string, signature: string, forceRefresh = false): Promise<AudioAnalysis | null> {
    if (active) await cancel()
    const request = ++revision; const jobId = crypto.randomUUID()
    const previous = states[source]?.projectId === project.id ? states[source] : undefined
    states[source] = { ...previous, projectId: project.id, state: 'loading', progress: 0, error: undefined, attemptedSignature: signature }
    const result = invoke<AudioAnalysis>('analyze_audio', { id: project.id, source, jobId, forceRefresh })
    const job = { jobId, projectId: project.id, source, signature, result, cancelled: false }; active = job
    try {
      const data = await result
      if (request !== revision || job.cancelled) return null
      states[source] = { projectId: project.id, state: 'ready', progress: 1, data: markRaw(data), signature, attemptedSignature: signature }; return data
    } catch (error) {
      if (request === revision && !job.cancelled) { states[source] = { ...states[source], state: 'error', error: errorText(error) }; throw error }
      return null
    } finally {
      if (active === job) {
        active = null
        if (job.cancelled && states[source]?.state === 'loading') states[source] = { ...states[source], state: 'cancelled', progress: 0 }
      }
    }
  }
  function progress(value: { jobId: string; projectId: string; source: string; progress: number }) {
    if (active?.jobId !== value.jobId || active.projectId !== value.projectId || active.source !== value.source || active.cancelled || !states[value.source]) return
    states[value.source].progress = Math.max(0, Math.min(1, value.progress))
  }
  function clear() { revision++; for (const key of Object.keys(states)) delete states[key] }
  return { states, cached, pending, analyze, cancel, progress, clear }
}
