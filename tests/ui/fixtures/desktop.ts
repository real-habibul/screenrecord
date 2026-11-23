import type { Page } from '@playwright/test'
import { readFileSync, writeFileSync } from 'node:fs'
import path from 'node:path'
import { editorVideo, generateMedia, mediaEngine, prepareEditorVideo, publishFixture, serveMedia } from './media'

const trialFixture = path.resolve('artifacts/ui/recording-test.mp4')
const cameraFixture = path.resolve('artifacts/ui/camera-short.mp4')
const cameraColorsFixture = path.resolve('artifacts/ui/camera-colors.mp4')
const legacyCameraClockFixture = path.resolve('artifacts/ui/camera-clock-legacy.mp4')
const realtimeCameraClockFixture = path.resolve('artifacts/ui/camera-clock-realtime.mp4')
const screenClockFixture = path.resolve('artifacts/ui/screen-clock.mp4')
export const audioFixture = path.resolve('artifacts/ui/audio.wav')
const mixAudioFixture = path.resolve('artifacts/ui/mix-audio.wav')
const previewFixture = path.resolve('artifacts/ui/camera-preview.jpg')
const cutFixture = path.resolve('artifacts/ui/cut-zones.mp4')
const cutAudio = path.resolve('artifacts/ui/cut-zones-fractional.wav')
const loopAudio = path.resolve('artifacts/ui/loop-zones-fractional.wav')

const videoEncoder = process.platform === 'win32'
  ? ['-c:v', 'h264_mf', '-hw_encoding', '0', '-b:v', '4000k']
  : ['-c:v', 'libx264', '-preset', 'ultrafast']

export function prepareProjectMedia() {
  const engine = mediaEngine()
  prepareEditorVideo(engine, videoEncoder)
  generateMedia(previewFixture, engine, [
    '-f', 'lavfi', '-i', 'color=c=blue:size=480x270', '-frames:v', '1', '-c:v', 'mjpeg', '-threads', '1',
  ])
}

function writeStereoTestTone(file: string, audible: (seconds: number) => boolean) {
  publishFixture(file, temporaryFile => {
    const sampleRate = 48_000
    const frames = 6 * sampleRate
    const bytesPerFrame = 8
    const headerSize = 44
    const data = Buffer.alloc(headerSize + frames * bytesPerFrame)
    data.write('RIFF', 0)
    data.writeUInt32LE(data.length - 8, 4)
    data.write('WAVEfmt ', 8)
    data.writeUInt32LE(16, 16)
    data.writeUInt16LE(3, 20) // IEEE float PCM
    data.writeUInt16LE(2, 22)
    data.writeUInt32LE(sampleRate, 24)
    data.writeUInt32LE(sampleRate * bytesPerFrame, 28)
    data.writeUInt16LE(bytesPerFrame, 32)
    data.writeUInt16LE(32, 34)
    data.write('data', 36)
    data.writeUInt32LE(frames * bytesPerFrame, 40)
    for (let frame = 0; frame < frames; frame++) {
      const time = frame / sampleRate
      const value = audible(time) ? .8 * Math.sin(2 * Math.PI * 997 * time) : 0
      const offset = headerSize + frame * bytesPerFrame
      data.writeFloatLE(value, offset)
      data.writeFloatLE(value, offset + 4)
    }
    writeFileSync(temporaryFile, data)
  })
}

export function prepareDesktopMedia() {
  prepareProjectMedia()
  const engine = mediaEngine()
  const generateVideo = (file: string, filter: string, duration?: string) => {
    generateMedia(file, engine, [
      '-f', 'lavfi', '-i', filter, ...(duration ? ['-t', duration] : []),
      ...videoEncoder, '-pix_fmt', 'yuv420p', '-movflags', '+faststart',
    ])
  }

  generateVideo(legacyCameraClockFixture, 'color=c=blue:size=320x180:rate=30', '35.5')
  generateVideo(realtimeCameraClockFixture, 'color=c=blue:size=320x180:rate=30', '36.5')
  generateVideo(screenClockFixture, 'color=c=red:size=320x180:rate=30', '36.5')
  generateVideo(trialFixture, 'testsrc2=size=320x180:rate=30', '5')
  generateVideo(cameraFixture, 'color=c=blue:size=160x160:rate=30', '1')
  generateVideo(cameraColorsFixture, 'color=c=blue:size=320x180:rate=30,drawbox=x=0:y=0:w=160:h=180:color=red:t=fill', '1')
  generateVideo(cutFixture, "color=c=green:size=960x540:rate=30:duration=6,drawbox=c=red:t=fill:enable='gte(t,1)*lt(t,2)+gte(t,3)*lt(t,4)+gte(t,5)',drawbox=c=blue:t=fill:enable='gte(t,2)*lt(t,3)'")
  generateMedia(audioFixture, engine, [
    '-f', 'lavfi', '-i', 'sine=frequency=997:sample_rate=48000', '-t', '12',
    '-af', 'volume=8', '-ac', '2', '-c:a', 'pcm_f32le',
  ])
  generateMedia(mixAudioFixture, engine, [
    '-f', 'lavfi', '-i', 'aevalsrc=0.6|0.6:s=48000', '-t', '12', '-c:a', 'pcm_f32le',
  ])
  writeStereoTestTone(cutAudio, time => (time >= .9977 && time < 2.0023) ||
    (time >= 2.9977 && time < 4.0023) || time >= 4.9977)
  writeStereoTestTone(loopAudio, time => !((time >= .2023 && time < .9977) || (time >= 2.0023 && time < 2.7977)))
}

export async function mockedDesktop(page: Page, initialFailures: Record<string, string> = {}) {
  await page.route('**/test-media/recording-test.mp4', route => serveMedia(route, trialFixture, 'video/mp4', '*'))
  await page.route('**/test-media/source.mp4', route => serveMedia(route, editorVideo, 'video/mp4', '*'))
  await page.route('**/test-media/camera-short.mp4', route => serveMedia(route, cameraFixture, 'video/mp4', '*'))
  await page.route('**/test-media/camera-colors.mp4', route => serveMedia(route, cameraColorsFixture, 'video/mp4', '*'))
  await page.route('**/test-media/camera-clock-legacy.mp4', route => serveMedia(route, legacyCameraClockFixture, 'video/mp4', '*'))
  await page.route('**/test-media/camera-clock-realtime.mp4', route => serveMedia(route, realtimeCameraClockFixture, 'video/mp4', '*'))
  await page.route('**/test-media/screen-clock.mp4', route => serveMedia(route, screenClockFixture, 'video/mp4', '*'))
  await page.route('**/test-media/audio.wav', route => serveMedia(route, audioFixture, 'audio/wav', '*'))
  await page.route('**/test-media/mix-audio.wav', route => serveMedia(route, mixAudioFixture, 'audio/wav', '*'))
  await page.route('**/test-media/cut-zones.mp4', route => serveMedia(route, cutFixture, 'video/mp4', '*'))
  await page.route('**/test-media/cut-zones.wav', route => serveMedia(route, cutAudio, 'audio/wav', '*'))
  await page.route('**/test-media/loop-zones.wav', route => serveMedia(route, loopAudio, 'audio/wav', '*'))
  await page.route('**/test-media/subtitle.png', route => route.fulfill({ contentType: 'image/png', body: Buffer.from('iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVQIHWP4z8DwHwAFgAI/ScLbtAAAAABJRU5ErkJggg==', 'base64') }))
  await page.addInitScript(({ previewJpeg, initialFailures }) => {
    const w = window as any
    w.isTauri = true
    localStorage.setItem('screenrecord-theme', 'dark')
    localStorage.setItem('screenrecord-library', 'open')
    const callbacks = new Map<number, (data: unknown) => void>()
    const listeners = new Map<string, number[]>()
    let callbackId = 0
    const state = { active: false, paused: false, phase: 'idle', cameraAvailable: false, elapsed: 0, projectId: null as string | null }
    let progress: any = null
    let transcription: any = null
    let previewSession: string | null = null
    let previewSequence = 0
    let meterSession: string | null = null
    let meterSequence = 0
    const additionalProjects: any[] = []
    const deletedProjects = new Set<string>()
    let project: any = {
      version: 1, id: 'ui-test', name: 'UI verification recording', createdAt: '2026-11-11T06:00:00Z', directory: 'D:\\UI-test',
      screenPath: '/test-media/source.mp4', microphonePath: null, systemAudioPath: null, cameraPath: '/test-media/source.mp4', cameraOffsetSeconds: 0,
      width: 960, height: 540, fps: 30, duration: 12, status: 'ready',
      cursor: Array.from({ length: 121 }, (_, i) => ({ time: i / 10, x: .2 + .6 * i / 120, y: .45, pressed: i === 25 || i === 32, visible: true })),
      edits: { trimStart: 0, trimEnd: 12, aspectRatio: 'source', background: '#283544', padding: 42, cornerRadius: 16, cursorSize: 1.5, cursorSmoothing: .5, showCursor: true, zooms: [{ id: 'zoom-test', start: 2, end: 5, scale: 1.8, x: .5, y: .5, followCursor: true }], camera: { enabled: true, x: .75, y: .64, width: .2, shape: 'circle', mirror: true }, crop: null, systemVolume: 1, microphoneVolume: 1 },
    }
    w.__test = { calls: [] as any[], saved: null, saveError: null, closeError: null, holdSave: false, finishSave: null, holdRegenerate: false, finishRegenerate: null, recordedProject: null, currentProject: () => structuredClone(project), patchProject: (patch: any) => { project = { ...project, ...patch, edits: { ...project.edits, ...patch.edits } } }, emit: (event: string, payload: unknown) => { for (const id of listeners.get(event) ?? []) callbacks.get(id)?.({ event, payload }) } }
    w.__test.addProject = (id: string, name: string) => additionalProjects.push({ ...structuredClone(project), id, name })
    w.__test.setExportStatus = (value: any) => { progress = structuredClone(value) }
    w.__test.recordingState = state
    w.__test.commandErrors = initialFailures
    w.__test.restoreProject = (id: string) => deletedProjects.delete(id)
    w.__TAURI_INTERNALS__ = {
      metadata: { currentWindow: { label: 'main' }, currentWebview: { label: 'main', windowLabel: 'main' } },
      transformCallback: (cb: (data: unknown) => void) => { const id = ++callbackId; callbacks.set(id, cb); return id },
      unregisterCallback: (id: number) => callbacks.delete(id),
      convertFileSrc: (value: string) => value,
      invoke: async (name: string, args: any) => {
        w.__test.calls.push({ name, args })
        if (w.__test.commandErrors[name]) throw new Error(w.__test.commandErrors[name])
        if (name === 'plugin:event|listen') { listeners.set(args.event, [...(listeners.get(args.event) ?? []), args.handler]); return args.handler }
        if (name === 'plugin:event|unlisten') return
        if (name === 'get_app_info') { if (w.__test.holdAppInfo) await new Promise<void>(resolve => { w.__test.finishAppInfo = resolve }); return { version: '0.4.0', projectsDirectory: 'D:\\UI-test', ffmpegAvailable: true, ffmpegPath: 'ffmpeg.exe' } }
        if (name === 'list_appearance_presets') { const result = structuredClone(w.__test.presets ?? []); if (w.__test.holdPresets) await new Promise<void>(resolve => { w.__test.finishPresets = resolve }); if (w.__test.presetsError) throw new Error(w.__test.presetsError); return result }
        if (name === 'save_appearance_preset') { if (w.__test.presetSaveError) throw new Error(w.__test.presetSaveError); const preset = { id: crypto.randomUUID(), name: args.name, settings: structuredClone(args.settings) }; w.__test.presets = [...(w.__test.presets ?? []), preset]; return preset }
        if (name === 'delete_appearance_preset') { if (w.__test.presetDeleteError) throw new Error(w.__test.presetDeleteError); w.__test.presets = w.__test.presets.filter((preset: any) => preset.id !== args.id); return }
        if (name === 'list_projects') {
          const result = structuredClone([project, ...additionalProjects].filter(item => !deletedProjects.has(item.id)))
          if (w.__test.holdList) await new Promise<void>(resolve => { w.__test.finishList = resolve })
          return result
        }
        if (name === 'open_project') return structuredClone([project, ...additionalProjects].find(item => item.id === args.id) ?? project)
        if (name === 'delete_project') {
          w.__test.mediaAtDelete = Array.from(document.querySelectorAll('video, audio')).map(element => ({ src: element.getAttribute('src'), paused: (element as HTMLMediaElement).paused }))
          if (w.__test.deleteError) throw new Error(w.__test.deleteError)
          deletedProjects.add(args.id); return
        }
        if (name === 'get_recording_state') { const snapshot = { ...state }; const failure = w.__test.recordingStateError; if (w.__test.holdRecordingState) { w.__test.holdRecordingState = false; await new Promise<void>(resolve => { w.__test.finishRecordingState = resolve }) }; if (failure) throw new Error(failure); return snapshot }
        if (name === 'get_export_progress') { const result = structuredClone(progress); if (w.__test.holdExportStatus) { w.__test.holdExportStatus = false; await new Promise<void>(resolve => { w.__test.finishExportStatus = resolve }) }; if (w.__test.exportStatusErrorOnce) { w.__test.exportStatusErrorOnce = false; if (w.__test.holdStatusRecovery) w.__test.holdExportStatus = true; throw new Error('Status channel temporarily unavailable.') }; return result }
        if (name === 'get_whatsapp_export_plan') {
          const result = structuredClone(w.__test.whatsappPlan ?? { width: 720, height: 1280, videoBitrate: 1_200_000, audioBitrate: 96_000 }); const failure = w.__test.whatsappError
          w.__test.planSavedEdits = structuredClone(project.edits)
          if (w.__test.holdWhatsAppPlan) await new Promise<void>(resolve => { (w.__test.planRequests ??= []).push({ id: args.id, finish: resolve }) })
          if (failure) throw new Error(failure)
          return result
        }
        if (name === 'get_local_transcription_status') { if (w.__test.localStatusError) throw new Error(w.__test.localStatusError); return w.__test.localStatus ?? { ready: true, runtimeReady: true, modelReady: true, runtimeVersion: '1.9.5', modelName: 'Whisper base multilingual', downloadBytes: 156880105, installedBytes: 156880105, error: null } }
        if (name === 'get_local_transcription_install_progress') return w.__test.localInstall ?? null
        if (name === 'start_local_transcription_install') { if (w.__test.holdLocalStart) await new Promise<void>(resolve => { w.__test.finishLocalStart = resolve }); if (w.__test.localStartError) throw new Error(w.__test.localStartError); w.__test.localInstall = { jobId: args.jobId, state: 'running', progress: .2, error: null, status: null }; return }
        if (name === 'cancel_local_transcription_install') { if (w.__test.holdLocalCancel) await new Promise<void>(resolve => { w.__test.finishLocalCancel = resolve }); w.__test.localInstall = { ...w.__test.localInstall, state: 'cancelled' }; w.__test.emit('local-transcription-install-progress', w.__test.localInstall); return }
        if (name === 'get_mask_tracking_progress') return w.__test.maskTracking ?? null
        if (name === 'start_mask_tracking') { if (w.__test.maskTrackingError) throw new Error(w.__test.maskTrackingError); w.__test.maskTracking = { jobId: args.jobId, projectId: args.id, regionId: args.regionId, state: 'running', progress: .4, error: null, tracking: null }; if (w.__test.holdMaskStart) await new Promise<void>(resolve => { w.__test.finishMaskStart = resolve }); return }
        if (name === 'cancel_mask_tracking') { if (w.__test.holdMaskCancel) await new Promise<void>(resolve => { w.__test.finishMaskCancel = resolve }); w.__test.maskTracking = { ...w.__test.maskTracking, state: 'cancelled' }; w.__test.emit('mask-tracking-progress', w.__test.maskTracking); return }
        if (name === 'get_transcription_progress') return transcription
        if (name === 'start_transcription') { if (w.__test.transcriptionError) throw new Error(w.__test.transcriptionError); if (w.__test.holdTranscription) await new Promise<void>(resolve => { w.__test.finishTranscription = resolve }); transcription = { state: 'running', projectId: project.id, jobId: args.jobId, progress: .4, error: null, cues: null }; return }
        if (name === 'cancel_transcription') { transcription = { ...transcription, state: 'cancelled' }; w.__test.emit('transcription-progress', transcription); return }
        if (name === 'render_subtitle_preview') {
          if (w.__test.holdSubtitle || w.__test.holdSubtitleWord && args.wordIndex !== null && args.wordIndex !== undefined) await new Promise<void>(resolve => { w.__test.finishSubtitle = resolve })
          if (w.__test.cancelSubtitleOnce) { w.__test.cancelSubtitleOnce = false; throw new Error('Subtitle preview cancelled.') }
          return '/test-media/subtitle.png'
        }
        if (name === 'import_subtitles') { if (w.__test.srtError) throw new Error(w.__test.srtError); return w.__test.importedCues ?? [{ id: 'srt', start: 1, end: 2, text: 'Imported caption', words: [], sourceRanges: [] }] }
        if (name === 'export_subtitles') return
        if (name === 'analyze_audio') {
          w.__test.analysisSnapshot = structuredClone(project.edits)
          if (w.__test.holdAnalysis) await new Promise<void>((resolve, reject) => { w.__test.finishAnalysis = resolve; w.__test.rejectAnalysis = reject })
          if (w.__test.analysisError) throw new Error(w.__test.analysisError)
          return w.__test.analysisResult ?? { sourceDuration: 12, bins: Array.from({ length: 120 }, (_, index) => ({ min: -(.1 + index % 9 / 20), max: .1 + index % 9 / 20 })), peak: .5, rms: .1, analyzedSeconds: 12, recommendedGain: 1.258925411794167 }
        }
        if (name === 'cancel_audio_analysis') { w.__test.rejectAnalysis?.(new Error('Audio analysis cancelled.')); w.__test.rejectAnalysis = null; return }
        if (name === 'start_microphone_meter') { meterSession = args.sessionId; if (w.__test.holdMeterStart) await new Promise<void>(resolve => { w.__test.finishMeterStart = resolve }); return }
        if (name === 'get_microphone_meter') return { sequence: ++meterSequence, peak: w.__test.meterPeak ?? .5, rms: .1, active: meterSession === args.sessionId && !w.__test.meterEnded, ready: true, error: w.__test.meterError ?? null }
        if (name === 'stop_microphone_meter') { if (w.__test.holdMeterStop) await new Promise<void>(resolve => { w.__test.finishMeterStop = resolve }); if (w.__test.meterStopError) throw new Error(w.__test.meterStopError); if (meterSession === args.sessionId) meterSession = null; return }
        if (name === 'plugin:dialog|open') return w.__test.importCancel ? null : 'D:\\Music\\music.wav'
        if (name === 'import_music') return { id: `music-${w.__test.calls.filter((call: any) => call.name === 'import_music').length}`, name: 'Music fixture', path: '/test-media/audio.wav', sourceDuration: 12, start: args.start, sourceStart: 0, duration: Math.min(6, project.duration - args.start), volume: .5, fadeIn: 0, fadeOut: 0 }
        if (name === 'get_recovery_issues') return w.__test.recoveryIssues ?? []
        if (name === 'get_stop_shortcut') return 'Ctrl+Shift+R'
        if (name === 'start_camera_preview') { previewSession = args.sessionId; return }
        if (name === 'get_camera_preview_frame') return { sequence: ++previewSequence, mimeType: 'image/jpeg', data: previewJpeg, error: w.__test.previewError ?? null, active: previewSession === args.sessionId }
        if (name === 'stop_camera_preview') { if (w.__test.holdPreviewStop) await new Promise<void>(resolve => { w.__test.finishPreviewStop = resolve }); if (w.__test.previewStopError) throw new Error(w.__test.previewStopError); if (previewSession === args.sessionId) previewSession = null; return }
        if (name === 'get_source_thumbnail') {
          const value = structuredClone(w.__test.thumbnail ?? { width: 1920, height: 1080, data: previewJpeg })
          if (w.__test.holdThumbnail) await new Promise<void>(resolve => { w.__test.finishThumbnail = resolve })
          if (w.__test.thumbnailError) throw new Error(w.__test.thumbnailError)
          return value
        }
        if (name === 'run_recording_test') {
          if (w.__test.holdTrial) await new Promise<void>((resolve, reject) => { w.__test.finishTrial = resolve; w.__test.rejectTrial = reject })
          if (w.__test.trialError) throw new Error(w.__test.trialError)
          return { id: w.__test.trialWrongId ? 'old-trial' : args.id, duration: 5, width: args.options.region?.width ?? 320, height: args.options.region?.height ?? 180, screenPath: '/test-media/recording-test.mp4', cameraPath: args.options.cameraId ? '/test-media/recording-test.mp4' : null, cameraOffsetSeconds: .2, microphonePath: args.options.microphoneId ? '/test-media/audio.wav' : null, systemAudioPath: args.options.systemAudio ? '/test-media/audio.wav' : null }
        }
        if (name === 'cancel_recording_test') { if (!w.__test.lateTrial) w.__test.rejectTrial?.(new Error('Test cancelled')); return }
        if (name === 'discard_recording_test') { if (w.__test.discardTrialError) throw new Error(w.__test.discardTrialError); return }
        if (name === 'list_capture_devices') {
          const result = structuredClone(w.__test.devices ?? { sources: [{ id: 'screen-test', title: 'Test display', kind: 'monitor', width: 1920, height: 1080 }, { id: 'window-test', title: 'Test window', kind: 'window', width: 960, height: 540 }], microphones: [{ id: 'mic-test', name: 'Test microphone' }], cameras: [{ id: 'cam-test', name: 'Test webcam' }] })
          const failure = w.__test.devicesError
          if (w.__test.holdDevices) await new Promise<void>(resolve => { w.__test.finishDevices = resolve })
          if (failure) throw new Error(failure)
          return result
        }
        if (name === 'save_project_edits') {
          if (w.__test.holdSave) await new Promise<void>(resolve => { w.__test.finishSave = resolve })
          if (w.__test.saveError) throw new Error(w.__test.saveError)
          if (deletedProjects.has(args.id)) throw new Error('Cannot save a deleted project')
          const item = args.id === project.id ? project : additionalProjects.find(item => item.id === args.id)
          Object.assign(item, { name: args.name.trim(), edits: structuredClone(args.edits) }); w.__test.saved = structuredClone(item); return structuredClone({ name: item.name, edits: item.edits })
        }
        if (name === 'start_recording') { if (w.__test.holdRecordingStart) await new Promise<void>(resolve => { w.__test.finishRecordingStart = resolve }); if (w.__test.recordingStartError) throw new Error(w.__test.recordingStartError); Object.assign(state, { active: true, paused: false, phase: 'recording', cameraAvailable: !!args.options.cameraId, projectId: w.__test.recordingProjectId ?? 'ui-test' }); const result = { ...state }; if (w.__test.endedBeforeStartAck) Object.assign(state, { active: false, paused: false, phase: 'idle', projectId: null }); return result }
        if (name === 'pause_recording' || name === 'resume_recording') { if (args.projectId !== state.projectId) throw new Error('Recording changed'); if (w.__test.pauseError) throw new Error(w.__test.pauseError); if (w.__test.holdPause) await new Promise<void>(resolve => { w.__test.finishPause = resolve }); state.paused = name === 'pause_recording'; state.phase = state.paused ? 'paused' : 'recording'; return { ...state } }
        if (name === 'stop_recording') {
          Object.assign(state, { active: true, phase: 'finalizing' })
          if (w.__test.holdRecordingStop) await new Promise<void>(resolve => { w.__test.finishRecordingStop = resolve })
          Object.assign(state, { active: false, paused: false, phase: 'idle', cameraAvailable: false })
          if (w.__test.recordingStopError) { w.__test.emit('recording-error', { projectId: state.projectId, error: w.__test.recordingStopError }); throw new Error(w.__test.recordingStopError) }
          project = structuredClone(w.__test.recordedProject ?? project)
          if (w.__test.emitRecordingStop) w.__test.emit('recording-stopped', structuredClone(project))
          return structuredClone(project)
        }
        if (name === 'regenerate_zooms') {
          const result = structuredClone(project)
          result.edits.zooms = [{ id: 'generated-zoom', start: 1, end: 4, scale: 2, x: .5, y: .5, followCursor: true }]
          if (w.__test.holdRegenerate) await new Promise<void>(resolve => { w.__test.finishRegenerate = resolve })
          project = result; return structuredClone(result)
        }
        if (name === 'plugin:dialog|save') { const result = w.__test.outputPath ?? 'D:\\UI-test\\export.mp4'; if (w.__test.holdOutputChoice) await new Promise<void>(resolve => { w.__test.finishOutputChoice = resolve }); return result }
        if (name === 'export_project') { if (w.__test.exportStartError) throw new Error(w.__test.exportStartError); if (w.__test.holdExportStart) await new Promise<void>(resolve => { w.__test.finishExportStart = resolve }); progress = { state: 'running', progress: .4, outputPath: args.options.outputPath, error: null }; return }
        if (name === 'cancel_export') { progress = { ...progress, state: 'cancelled' }; w.__test.emit('export-progress', progress); return }
        if (name === 'close_app') { if (w.__test.closeError) throw new Error(w.__test.closeError); return }
        if (name === 'reveal_file') return
        throw new Error(`Unmocked command ${name}`)
      },
    }
    w.__TAURI_EVENT_PLUGIN_INTERNALS__ = { unregisterListener: (_event: string, id: number) => callbacks.delete(id) }
  }, { previewJpeg: readFileSync(previewFixture).toString('base64'), initialFailures })
}
