import { clamp } from './editing'
import { musicGain } from './tracks'
import type { AudioProcessing, MusicClip, VolumeKeyframe } from './types'
import type { AudioMonitor } from './audition'
import { sameVolumeKeys, scheduleVolume, VOLUME_BUSES } from './preview-volume'

export class PreviewAudio {
  private context: AudioContext | null = null
  private limiter: AudioWorkletNode | null = null
  private mixer: AudioWorkletNode | null = null
  private inputGate: GainNode | null = null
  private outputGate: GainNode | null = null
  private spanEnd: number | null = null
  private inputDeadline = 0
  private outputDeadline = 0
  private sources: MediaElementAudioSourceNode[] = []
  private gains: Array<{ node: GainNode; kind: 'microphone' | 'system' }> = []
  private initialization: Promise<void> | null = null
  private disposed = false
  private failure: string | null = null
  private wantedRunning = false
  private cancelResume?: () => void
  private musicWanted: Array<{ clip: MusicClip; element: HTMLAudioElement }> = []
  private music = new Map<string, { element: HTMLAudioElement; source: MediaElementAudioSourceNode; gain: GainNode }>()
  private volumes = { microphone: 1, system: 1 }
  private processing: AudioProcessing = { microphoneGain: 1, systemGain: 1, ducking: { enabled: false, amountDb: 12 } }
  private monitor: AudioMonitor = 'mix'
  private volumeKeys: Record<typeof VOLUME_BUSES[number], VolumeKeyframe[]> = { microphone: [], system: [], music: [] }
  private volumeScheduleEnd: number | null = null
  readonly latencySeconds = 239 / 48_000
  get running() { return this.context?.state === 'running' }

  constructor(private microphone: HTMLAudioElement | undefined, private system: HTMLAudioElement | undefined, private onError: (message: string) => void) {}

  prepare(): Promise<void> {
    if (!this.initialization) this.initialization = this.initialize()
    return this.initialization
  }

  private async initialize() {
    if (this.disposed) return
    const context = new AudioContext({ sampleRate: 48_000, latencyHint: 'interactive' })
    this.context = context
    try {
      await context.audioWorklet.addModule(`${import.meta.env.BASE_URL}audio-limiter.js`)
      await context.audioWorklet.addModule(`${import.meta.env.BASE_URL}audio-ducker.js`)
      if (this.disposed) return
      const limiter = new AudioWorkletNode(context, 'screenrecord-limiter', { numberOfInputs: 1, numberOfOutputs: 1, outputChannelCount: [2], channelCount: 2, channelCountMode: 'explicit' })
      this.limiter = limiter
      this.mixer = new AudioWorkletNode(context, 'screenrecord-music-mixer', { numberOfInputs: 3, numberOfOutputs: 1, outputChannelCount: [2], channelCount: 2, channelCountMode: 'explicit' })
      limiter.addEventListener('processorerror', () => {
        this.microphone?.pause(); this.system?.pause(); this.music.forEach(track => track.element.pause())
        this.failure = 'Audio preview processing failed. Reopen the project to restore audio playback.'
        this.pause(); this.onError(this.failure)
      })
      this.mixer.addEventListener('processorerror', () => {
        this.failure = 'Audio ducking preview failed. Reopen the project to restore audio playback.'
        this.microphone?.pause(); this.system?.pause(); this.music.forEach(track => track.element.pause())
        this.pause(); this.onError(this.failure)
      })
      this.inputGate = context.createGain(); this.outputGate = context.createGain()
      this.inputGate.gain.value = 0; this.outputGate.gain.value = 0
      this.inputGate.connect(limiter); limiter.connect(this.outputGate); this.outputGate.connect(context.destination)
      this.mixer.connect(this.inputGate)
      for (const [element, kind] of [[this.microphone, 'microphone'], [this.system, 'system']] as const) {
        if (!element) continue
        const source = context.createMediaElementSource(element)
        const gain = context.createGain()
        source.connect(gain); gain.connect(this.mixer, 0, kind === 'microphone' ? 0 : 1)
        this.sources.push(source); this.gains.push({ node: gain, kind })
        element.volume = 1; element.muted = false
      }
      this.setVolumes(this.volumes.microphone, this.volumes.system)
      this.setProcessing(this.processing)
      this.setMonitor(this.monitor)
      this.connectMusic()
    } catch (error) {
      void context.close().catch(() => undefined)
      throw new Error(`Audio preview could not start: ${String(error)}. Reopen the project to try again.`)
    }
  }

  setVolumes(microphone: number, system: number) {
    this.volumes = { microphone: clamp(microphone, 0, 2), system: clamp(system, 0, 2) }
    for (const gain of this.gains) gain.node.gain.value = this.volumes[gain.kind] * this.processing[gain.kind === 'microphone' ? 'microphoneGain' : 'systemGain']
  }

  setProcessing(processing?: AudioProcessing) {
    this.processing = processing ?? { microphoneGain: 1, systemGain: 1, ducking: { enabled: false, amountDb: 12 } }
    this.setVolumes(this.volumes.microphone, this.volumes.system)
    this.mixer?.port.postMessage({ enabled: !!this.microphone && this.processing.ducking.enabled, systemEnabled: !!this.microphone && !!this.processing.ducking.systemEnabled, amountDb: this.processing.ducking.amountDb })
    for (const bus of VOLUME_BUSES) {
      const keys = this.processing.volumeKeyframes?.[bus] ?? []
      if (sameVolumeKeys(this.volumeKeys[bus], keys)) continue
      this.volumeKeys[bus] = keys.map(key => ({ ...key }))
      this.volumeScheduleEnd = null
    }
  }

  setMusicTracks(tracks: Array<{ clip: MusicClip; element: HTMLAudioElement }>) {
    this.musicWanted = tracks
    this.connectMusic()
  }

  setMonitor(monitor: AudioMonitor = 'mix') {
    this.monitor = monitor
    this.mixer?.port.postMessage({ monitor })
  }

  private connectMusic() {
    if (!this.context || !this.limiter || this.disposed) return
    for (const [id, track] of this.music) {
      if (this.musicWanted.some(wanted => wanted.clip.id === id && wanted.element === track.element)) continue
      track.element.pause(); track.element.muted = true; track.source.disconnect(); track.gain.disconnect(); this.music.delete(id)
    }
    for (const { clip, element } of this.musicWanted) {
      if (this.music.has(clip.id)) continue
      const source = this.context.createMediaElementSource(element); const gain = this.context.createGain()
      gain.gain.value = 0; source.connect(gain); gain.connect(this.mixer!, 0, 2)
      this.music.set(clip.id, { element, source, gain }); element.volume = 1; element.muted = false
    }
  }

  setMusicTime(time: number, running: boolean) {
    if (!this.context) return
    const now = this.context.currentTime
    for (const { clip } of this.musicWanted) {
      const gain = this.music.get(clip.id)?.gain.gain; if (!gain) continue
      gain.cancelScheduledValues(now); gain.setValueAtTime(musicGain(clip, time), now)
      if (!running || time >= clip.start + clip.duration) continue
      for (const point of [clip.start, clip.start + clip.fadeIn, clip.start + clip.duration - clip.fadeOut, clip.start + clip.duration]) {
        if (point <= time) continue
        const value = point === clip.start + clip.duration ? 0 : musicGain(clip, point)
        if (point === clip.start || (point === clip.start + clip.duration && clip.fadeOut === 0)) gain.setValueAtTime(value, now + point - time)
        else gain.linearRampToValueAtTime(value, now + point - time)
      }
    }
  }

  async resume() {
    this.cancelResume?.()
    this.wantedRunning = true
    let cancel!: () => void
    let timer!: ReturnType<typeof setTimeout>
    const deadline = new Promise<void>((resolve, reject) => {
      cancel = resolve
      timer = setTimeout(() => reject(new Error('Audio preview took too long to start. Pause and play again to retry audio.')), 7000)
    })
    this.cancelResume = cancel
    const start = async () => {
      await this.prepare()
      if (this.disposed || this.cancelResume !== cancel || !this.wantedRunning) return
      if (this.failure) throw new Error(this.failure)
      const context = this.context
      if (!context) return
      await context.resume()
      if (!this.disposed && !this.wantedRunning && context.state === 'running') await context.suspend()
    }
    try { await Promise.race([start(), deadline]) }
    finally {
      clearTimeout(timer)
      if (this.cancelResume === cancel) this.cancelResume = undefined
    }
  }

  reset(resetDucking = true) { this.limiter?.port.postMessage('reset'); if (resetDucking) this.mixer?.port.postMessage('reset') }

  gateSpan(sourceTime: number, sourceEnd: number) {
    if (!this.context || !this.inputGate || !this.outputGate) return
    const now = this.context.currentTime
    const clocks = [sourceTime + this.latencySeconds]
    for (const media of [this.microphone, this.system]) if (media && !media.paused) clocks.push(media.currentTime)
    for (const { clip, element } of this.musicWanted) if (!element.paused && sourceTime >= clip.start && sourceTime < clip.start + clip.duration) clocks.push(element.currentTime - clip.sourceStart + clip.start)
    const remaining = Math.max(0, sourceEnd - Math.max(...clocks))
    if (this.volumeScheduleEnd !== sourceEnd && this.mixer) {
      const source = Math.min(sourceEnd, Math.max(...clocks))
      for (const bus of VOLUME_BUSES) {
        const param = this.mixer.parameters.get(`${bus}Volume`)
        if (param) scheduleVolume(param, this.volumeKeys[bus], source, sourceEnd, now)
      }
      this.volumeScheduleEnd = sourceEnd
    }
    if (this.spanEnd !== sourceEnd) { this.spanEnd = sourceEnd; this.inputDeadline = now + remaining; this.outputDeadline = now + remaining + this.latencySeconds }
    else { this.inputDeadline = Math.min(this.inputDeadline, now + remaining); this.outputDeadline = Math.min(this.outputDeadline, now + remaining + this.latencySeconds) }
    for (const [gate, deadline] of [[this.inputGate, this.inputDeadline], [this.outputGate, this.outputDeadline]] as const) {
      gate.gain.cancelScheduledValues(now); gate.gain.setValueAtTime(deadline > now ? 1 : 0, now); gate.gain.setValueAtTime(0, Math.max(now, deadline))
    }
    const active = this.mixer?.parameters.get('active')
    active?.cancelScheduledValues(now); active?.setValueAtTime(this.inputDeadline > now ? 1 : 0, now); active?.setValueAtTime(0, Math.max(now, this.inputDeadline))
  }

  pause() {
    this.wantedRunning = false
    this.cancelResume?.()
    this.spanEnd = null
    this.volumeScheduleEnd = null
    if (this.context) { const active = this.mixer?.parameters.get('active'); active?.cancelScheduledValues(this.context.currentTime); active?.setValueAtTime(0, this.context.currentTime) }
    if (this.context) for (const gate of [this.inputGate, this.outputGate]) { gate?.gain.cancelScheduledValues(this.context.currentTime); gate?.gain.setValueAtTime(0, this.context.currentTime) }
    if (this.context && this.context.state !== 'closed') void this.context.suspend().catch(error => this.onError(`Audio preview could not pause: ${String(error)}`))
  }

  dispose() {
    this.disposed = true; this.wantedRunning = false
    this.cancelResume?.()
    this.sources.forEach(source => source.disconnect())
    this.gains.forEach(gain => gain.node.disconnect())
    this.music.forEach(track => { track.element.pause(); track.source.disconnect(); track.gain.disconnect() }); this.music.clear()
    this.limiter?.disconnect(); this.limiter?.port.close()
    this.mixer?.disconnect(); this.mixer?.port.close()
    this.inputGate?.disconnect(); this.outputGate?.disconnect()
    if (this.context && this.context.state !== 'closed') void this.context.close().catch(() => undefined)
  }
}
