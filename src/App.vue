<script setup lang="ts">
import { computed, markRaw, nextTick, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { open, save } from '@tauri-apps/plugin-dialog'
import { command, desktop, errorText } from './api'
import { addZoom, aspectRatio, clamp, even, normalizeEdits, timeLabel } from './editing'
import type { AppInfo, AppearanceSettings, CaptureApp, CaptureDevices, ExportOptions, ExportProgress, MusicClip, MaskTrackingProgress, Project, ProjectSummary, RecordedAudioSource, RecordingError, RecordingOptions, RecordingState, Redaction, SavedEdits, SubtitleCue, TrackSelection, TranscriptionOptions, TranscriptionProgress, WhatsAppExportPlan } from './types'
import { applyAppearance } from './appearance'
import { reframeAt, setReframeKey } from './reframe'
import { chooseApp, emptyAppSelection, filterApps, reconcileAppSelection } from './capture-apps'
import { projectedSubtitle } from './subtitle-projection'
import { manualCorrection, maskFingerprint, trackingBudgetError } from './mask-tracking'
import { audioSignature, audioSources, createAudioAnalysisStore, MAX_RECORDED_GAIN } from './audio-analysis'
import { auditionFromPlayhead, fitAuditionRange, type AudioMonitor, type AuditionRange } from './audition'
import { runCountdown } from './countdown'
import { initializeTracks, redactionsFit } from './tracks'
import { EditHistory } from './history'
import { changeSourceTrim, cutVideoRange, outputDuration, outputToSource, projectedRange, removeVideo, retainedSource, sourceToOutput, splitVideo } from './video-edits'
import Preview from './components/Preview.vue'
import Inspector from './components/Inspector.vue'
import EditorTimeline from './components/EditorTimeline.vue'
import CameraPreview from './components/CameraPreview.vue'
import MicrophoneMeter from './components/MicrophoneMeter.vue'
import RecordingTrial from './components/RecordingTrial.vue'
import SourceRegion from './components/SourceRegion.vue'
import LocalTranscriptionSetup from './components/LocalTranscriptionSetup.vue'

const info = ref<AppInfo | null>(null)
const projects = ref<ProjectSummary[]>([])
const project = ref<Project | null>(null)
const devices = ref<CaptureDevices>({ sources: [], microphones: [], cameras: [] })
const loading = ref(desktop)
const initializationError = ref('')
const devicesLoading = ref(false)
const devicesReady = ref(false)
let deviceRequest = 0
const opening = ref(false)
const busy = ref('')
const error = ref('')
const notice = ref('')
const recoveryIssues = ref<string[]>([])
const visibleRecoveryIssues = computed(() => [...new Set(recoveryIssues.value.map(issue => issue.trim()).filter(Boolean))])
const stopShortcut = ref<string | null>(null)
const savedSnapshot = ref('')
const saving = ref(false)
const closing = ref(false)
const time = ref(0)
const playing = ref(false)
const selectedZoom = ref<string | null>(null)
const selectedReframe = ref<string | null>(null)
const addingReframe = ref(false)
const selectedVideoClip = ref<string | null>(null)
const cutDialog = ref<HTMLDialogElement>()
const cutStart = ref(0)
const cutEnd = ref(0)
const cutError = ref('')
const editedDuration = computed(() => project.value ? outputDuration(project.value.edits) : 0)
const outputTime = computed(() => project.value ? sourceToOutput(project.value.edits, time.value) : 0)
const auditionOpen = ref(false)
const monitor = ref<AudioMonitor>('mix')
const loopEnabled = ref(false)
const auditionRange = ref<AuditionRange>({ start: 0, end: 0 })
const previewLoop = computed(() => auditionOpen.value && loopEnabled.value ? auditionRange.value : null)
watch(editedDuration, duration => {
  if (auditionOpen.value) auditionRange.value = fitAuditionRange(auditionRange.value, duration)
})
function resetAudition() {
  auditionOpen.value = false
  monitor.value = 'mix'
  loopEnabled.value = false
}
function toggleAudition() {
  if (auditionOpen.value) { resetAudition(); return }
  auditionRange.value = auditionFromPlayhead(outputTime.value, editedDuration.value)
  auditionOpen.value = true
}
const hiddenCaptions = computed(() => project.value?.edits.subtitles.some(cue => !projectedSubtitle(project.value!.edits, cue) && projectedRange(project.value!.edits, cue.start, cue.end).length) ?? false)
const libraryOpen = ref(localStorage.getItem('screenrecord-library') !== 'closed')
const recordingDialogOpen = ref(false)
const recordError = ref<{ title: string; message: string } | null>(null)
const recordErrorElement = ref<HTMLElement>()
const recordStartButton = ref<HTMLButtonElement>()
const recordingFailed = ref(false)
const cameraPreview = ref<InstanceType<typeof CameraPreview>>()
const microphoneMeter = ref<InstanceType<typeof MicrophoneMeter>>()
const recordingTrial = ref<InstanceType<typeof RecordingTrial>>()
const sourceRegion = ref<InstanceType<typeof SourceRegion>>()
const trialBusy = ref(false)
const audioAnalysis = createAudioAnalysisStore()
let waveformBatch = false
let waveformPending = false
let waveformRevision = 0
const selection = ref<TrackSelection | null>(null)
const drawing = ref(false)
const correctionRegion = ref<string | null>(null)
const maskProgress = ref<MaskTrackingProgress | null>(null)
const trackingMask = computed(() => maskProgress.value?.state === 'running')
let maskTarget: { projectId: string; regionId: string; jobId: string; cancelled: boolean } | null = null
let maskStartPromise: Promise<void> | null = null
let maskCancelPromise: Promise<void> | null = null
let previewRegionSnapshot: Redaction | null = null
let maskRollback = false
const unreviewedMasks = computed(() => project.value?.edits.redactions.filter(region => region.tracking && !region.tracking.reviewed && projectedRange(project.value!.edits, region.start, region.end).length) ?? [])
const activeTool = ref('canvas')
const previewReleased = ref(false)
const projectMenu = ref<{ item: ProjectSummary; x: number; y: number } | null>(null)
const projectMenuElement = ref<HTMLDivElement>()
const deleteDialog = ref<HTMLDialogElement>()
const deleteTarget = ref<ProjectSummary | null>(null)
const deleteError = ref('')
const deleting = ref(false)
let projectActionOrigin: HTMLElement | null = null
const history = new EditHistory()
const historyVersion = ref(0)
let historyGesture = false
let previewGesture = false
let previewGestureProject: Project | null = null
let timelineGesture = false
let historyApplying = false
let historyQueued = false
let historyGeneration = 0
let editRevision = 0
const editsPending = ref(false)
const editsSnapshot = ref('')
const pointerEditing = ref(false)
const gestureWaiters = new Set<() => void>()
const canUndo = computed(() => { void historyVersion.value; return !pointerEditing.value && (editsPending.value || history.canUndo) })
const canRedo = computed(() => { void historyVersion.value; return !pointerEditing.value && !editsPending.value && history.canRedo })
const transcription = ref<TranscriptionProgress | null>(null)
const transcribing = computed(() => transcription.value?.state === 'running')
const transcriptionDialog = ref<HTMLDialogElement>()
const transcriptionDialogOpen = ref(false)
const localSetup = ref<InstanceType<typeof LocalTranscriptionSetup>>()
const localReady = ref(false)
const localInstallBusy = ref(false)
const transcriptionDialogClosing = ref(false)
const transcriptionOptions = ref<TranscriptionOptions>({ provider: 'local', apiKey: '', source: 'microphone', language: 'id' })
let transcriptionTarget: { projectId: string; jobId: string } | null = null
const idleRecording = (): RecordingState => ({ active: false, paused: false, phase: 'idle', cameraAvailable: false, elapsed: 0, projectId: null })
const recording = ref<RecordingState>(idleRecording())
const countdownSeconds = ref([0, 3, 5].includes(Number(localStorage.getItem('screenrecord-countdown'))) && localStorage.getItem('screenrecord-countdown') !== null ? Number(localStorage.getItem('screenrecord-countdown')) : 3)
const countdown = ref<number | null>(null)
const countdownCancel = ref<HTMLButtonElement>()
let countdownController: AbortController | null = null
let recordingStartRequest = 0
let recordingRevision = 0
type RecordingTerminal = { project?: Project; error?: string }
let pendingRecordingStart: { request: number; terminals: Map<string, RecordingTerminal> } | null = null
const exportProgress = ref<ExportProgress | null>(null)
const exportStatusError = ref('')
let exportTargetPath: string | null = null
let exportAttempt = 0
function acceptExportProgress(value: ExportProgress | null, event = false) {
  if (!value) return
  const previous = exportProgress.value
  if (event && value.state === 'running' && previous?.state !== 'running' && busy.value !== 'Starting export') { exportAttempt++; exportTargetPath = value.outputPath }
  if (exportTargetPath && value.outputPath !== exportTargetPath) return
  if (!event && previous?.outputPath === value.outputPath && previous.state !== 'running' && value.state === 'running') return
  exportStatusError.value = ''; exportProgress.value = value
}
function rememberedRecordingOptions(): Omit<RecordingOptions, 'sourceId'> {
  try {
    const saved = JSON.parse(localStorage.getItem('screenrecord-recording-options') ?? '{}')
    return { fps: saved.fps === 60 ? 60 : 30, microphoneId: typeof saved.microphoneId === 'string' ? saved.microphoneId : null, systemAudio: typeof saved.systemAudio === 'boolean' ? saved.systemAudio : true, cameraId: typeof saved.cameraId === 'string' ? saved.cameraId : null }
  } catch { return { fps: 30, microphoneId: null, systemAudio: true, cameraId: null } }
}
const options = ref<RecordingOptions>({ sourceId: '', ...rememberedRecordingOptions(), systemAudioScope: 'system', region: null })
const sourceKind = ref<'monitor' | 'window' | 'apps'>('monitor')
const appAudioScope = ref<'system' | 'application'>('system')
const appSelection = ref(emptyAppSelection())
const appSearch = ref('')
const exportOptions = ref<ExportOptions>({ outputPath: '', resolution: '1080p', fps: 30 })
const exportPreset = ref<'standard' | 'whatsapp'>('standard')
const exportDialogOpen = ref(false)
const whatsappPlan = ref<WhatsAppExportPlan | null>(null)
const whatsappPlanSnapshot = ref('')
const exportPlanLoading = ref(false)
const exportPlanError = ref('')
const choosingOutput = ref(false)
let exportSession = 0
let exportPlanRequest = 0
let outputRequest = 0
const validWhatsAppPlan = computed(() => !!whatsappPlan.value && !exportPlanLoading.value && !dirty.value && whatsappPlanSnapshot.value === savedSnapshot.value)
const exportDimensions = computed(() => { if (!project.value) return ''; if (exportPreset.value === 'whatsapp') return validWhatsAppPlan.value ? `${whatsappPlan.value!.width} x ${whatsappPlan.value!.height}` : 'Auto size'; const ratio = aspectRatio(project.value); const edge = exportOptions.value.resolution === '2160p' ? 2160 : exportOptions.value.resolution === '1440p' ? 1440 : 1080; return `${even(edge * Math.max(1, ratio))} x ${even(edge * Math.max(1, 1 / ratio))}` })
const completedExportSize = computed(() => exportProgress.value?.state === 'completed' && typeof exportProgress.value.outputBytes === 'number' && Number.isFinite(exportProgress.value.outputBytes) && exportProgress.value.outputBytes >= 0 ? `${(exportProgress.value.outputBytes / 1_000_000).toFixed(2)} MB` : '')
const recordDialog = ref<HTMLDialogElement>()
const exportDialog = ref<HTMLDialogElement>()
const preview = ref<InstanceType<typeof Preview>>()
const inspector = ref<InstanceType<typeof Inspector>>()
const theme = ref(localStorage.getItem('screenrecord-theme') ?? (matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'))
const filteredSources = computed(() => devices.value.sources.filter(item => item.kind === sourceKind.value))
const captureApps = computed(() => (devices.value.apps ?? []).filter(app => app.windows.length > 0))
const filteredApps = computed(() => filterApps(captureApps.value, appSearch.value))
const selectedApp = computed(() => captureApps.value.find(app => app.id === appSelection.value.appId))
const selectedSource = computed(() => (sourceKind.value === 'apps' ? selectedApp.value?.windows ?? [] : filteredSources.value).find(item => item.id === options.value.sourceId))
const sourceSummary = computed(() => selectedSource.value ? sourceKind.value === 'apps' ? `${selectedApp.value?.name}: ${selectedSource.value.title}` : selectedSource.value.title : sourceKind.value === 'apps' && selectedApp.value ? 'Choose an app window' : 'Select a source')
const editSnapshot = () => project.value ? `{"name":${JSON.stringify(project.value.name)},"edits":${editsSnapshot.value}}` : ''
const dirty = computed(() => editsPending.value || editSnapshot() !== savedSnapshot.value)
const exporting = computed(() => exportProgress.value?.state === 'running')
const locked = computed(() => loading.value || closing.value || opening.value || !!busy.value || trialBusy.value || countdown.value !== null || recording.value.active || exporting.value || transcribing.value || trackingMask.value)
let autoSaveTimer: ReturnType<typeof setTimeout> | undefined
let polling: ReturnType<typeof setInterval> | undefined
let savePromise: Promise<void> | null = null
let lastSaveFailure: unknown = null
let pollPending = false
let projectListRequest = 0
let unlisteners: UnlistenFn[] = []

watch(theme, value => { document.documentElement.dataset.theme = value; localStorage.setItem('screenrecord-theme', value) }, { immediate: true })
watch(() => project.value?.edits, edits => {
  if (!edits || historyApplying) return
  editsPending.value = true; editRevision++
  if (!historyGesture && !historyQueued) {
    historyQueued = true; const generation = historyGeneration; const current = project.value
    queueMicrotask(() => { if (generation !== historyGeneration || current !== project.value) return; historyQueued = false; if (!historyGesture) commitHistory() })
  }
}, { deep: true, flush: 'sync' })
function scheduleAutoSave() {
  clearTimeout(autoSaveTimer)
  if (dirty.value && !historyGesture) autoSaveTimer = setTimeout(() => { if (!locked.value && !historyGesture) void persist().catch(showError) }, 800)
}
watch(locked, value => { if (value) { stopCorrection(); commitHistory(); closeProjectMenu() } else if (dirty.value) void persist().catch(showError) }, { flush: 'sync' })
let maskFingerprints = new Map<string, string>()
watch(() => project.value?.edits.redactions, regions => {
  const next = new Map<string, string>()
  for (const region of regions ?? []) { const key = `${project.value?.id}:${region.id}`; const fingerprint = maskFingerprint(region); next.set(key, fingerprint); if (!historyApplying && !maskRollback && maskFingerprints.has(key) && maskFingerprints.get(key) !== fingerprint && region.tracking?.reviewed) region.tracking.reviewed = false }
  maskFingerprints = next
}, { deep: true, flush: 'sync' })
watch([() => project.value?.id, () => project.value?.edits.camera.layout, () => selection.value?.id], () => { if (correctionRegion.value && (selection.value?.id !== correctionRegion.value || !project.value?.edits.redactions.some(region => region.id === correctionRegion.value) || project.value.edits.camera.layout === 'fullscreen')) stopCorrection() }, { flush: 'sync' })
watch(time, value => { const region = project.value?.edits.redactions.find(item => item.id === correctionRegion.value); if (correctionRegion.value && (!region || value < region.start || value >= region.end)) stopCorrection() }, { flush: 'sync' })
watch(sourceKind, () => {
  if (sourceKind.value !== 'apps') appAudioScope.value = 'system'
  if (sourceKind.value === 'apps') { appSelection.value = reconcileAppSelection(captureApps.value, appSelection.value, true); options.value.sourceId = appSelection.value.windowId }
  else if (!filteredSources.value.some(s => s.id === options.value.sourceId)) options.value.sourceId = filteredSources.value[0]?.id ?? ''
})
watch([() => options.value.systemAudio, sourceKind, appAudioScope], ([enabled, kind, scope]) => {
  options.value.systemAudioScope = enabled && kind === 'apps' ? scope : 'system'
}, { flush: 'sync' })
watch([() => project.value?.id, savedSnapshot, locked, recordingDialogOpen], () => { void loadWaveforms() })
async function stopAudioAnalysis() { waveformRevision++; await audioAnalysis.cancel() }
async function loadWaveforms() {
  const current = project.value
  if (!desktop || !current || locked.value || recordingDialogOpen.value || dirty.value) return
  if (waveformBatch) { waveformPending = true; return }
  const revision = waveformRevision; waveformBatch = true
  const snapshot = savedSnapshot.value
  const saved = JSON.parse(snapshot) as SavedEdits
  try {
    for (const source of audioSources(current)) {
      if (revision !== waveformRevision || current !== project.value || snapshot !== savedSnapshot.value || locked.value || dirty.value || recordingDialogOpen.value) break
      const signature = audioSignature(saved.edits, source)
      const state = audioAnalysis.states[source]
      if (state?.projectId === current.id && state.attemptedSignature === signature) continue
      try { await audioAnalysis.analyze(current, source, signature) } catch { /* The track shows its analysis error and a retry action. */ }
    }
  } finally {
    waveformBatch = false
    if (waveformPending) { waveformPending = false; void loadWaveforms() }
  }
}
async function analyzeRecordedAudio(source: string, forNormalization = false) {
  const current = project.value; if (!current || locked.value) return
  busy.value = forNormalization ? 'Normalizing audio' : 'Analyzing audio'; preview.value?.pause(); error.value = ''
  try {
    const pending = forNormalization && audioAnalysis.pending(current.id, source, audioSignature(current.edits, source))
    if (pending) await pending
    else {
      const stopped = stopAudioAnalysis()
      const revision = waveformRevision
      await stopped
      await persist()
      if (current !== project.value || revision !== waveformRevision) return
      await audioAnalysis.analyze(current, source, audioSignature(current.edits, source), !forNormalization)
    }
    if (current === project.value) return audioAnalysis.cached(current.id, source, audioSignature(current.edits, source))
  } catch (failure) { showError(failure) }
  finally { busy.value = '' }
}
async function normalizeRecordedAudio(source: RecordedAudioSource) {
  const current = project.value
  if (!current || locked.value) return
  error.value = ''
  const signature = audioSignature(current.edits, source)
  const data = audioAnalysis.cached(current.id, source, signature) ?? await analyzeRecordedAudio(source, true)
  if (!data || current !== project.value) return
  if (!data.peak || !data.analyzedSeconds) { notice.value = 'No audible signal in the kept video.'; return }
  const gainKey = source === 'microphone' ? 'microphoneGain' : 'systemGain'
  commitHistory()
  current.edits.audioProcessing[gainKey] = Math.min(MAX_RECORDED_GAIN, data.recommendedGain)
  commitHistory()
}
function showError(value: unknown) {
  if (value !== lastSaveFailure) lastSaveFailure = null
  error.value = errorText(value); notice.value = ''
  if (recordingDialogOpen.value) recordError.value = null
}
function showRecordingError(value: unknown, title: string) {
  recordError.value = { title, message: errorText(value) }
  void nextTick(() => recordErrorElement.value?.focus())
}
function toggleLibrary() { libraryOpen.value = !libraryOpen.value; localStorage.setItem('screenrecord-library', libraryOpen.value ? 'open' : 'closed') }
function rememberRecordingOptions() { const { fps, microphoneId, systemAudio, cameraId } = options.value; localStorage.setItem('screenrecord-recording-options', JSON.stringify({ fps, microphoneId, systemAudio, cameraId })) }
function rememberCountdown() { localStorage.setItem('screenrecord-countdown', String(countdownSeconds.value)) }
function flushHistorySnapshot() { if (project.value && editsPending.value) { editsSnapshot.value = JSON.stringify(project.value.edits); history.observe(editsSnapshot.value); editsPending.value = false } }
function commitHistory() {
  if (previewGesture || timelineGesture) return
  const changed = editsPending.value; const ended = historyGesture
  historyGesture = false; flushHistorySnapshot(); history.commit(); historyVersion.value++
  if (changed || ended) scheduleAutoSave()
  for (const resolve of gestureWaiters) resolve(); gestureWaiters.clear()
}
function beginTransaction() { if (historyGesture) return; commitHistory(); historyGesture = true; clearTimeout(autoSaveTimer) }
function beginPreviewEdit() { commitHistory(); beginTransaction(); const region = project.value?.edits.redactions.find(item => item.id === selection.value?.id); previewRegionSnapshot = region ? JSON.parse(JSON.stringify(region)) : null; previewGestureProject = project.value; previewGesture = true; pointerEditing.value = true }
function endPreviewEdit(commit = true) { if (!previewGesture || previewGestureProject !== project.value) return; if (!commit && previewRegionSnapshot && project.value) { const index = project.value.edits.redactions.findIndex(item => item.id === previewRegionSnapshot!.id); if (index >= 0) { maskRollback = true; project.value.edits.redactions[index] = previewRegionSnapshot; maskRollback = false } }; previewRegionSnapshot = null; previewGestureProject = null; previewGesture = false; pointerEditing.value = false; commitHistory() }
function beginTimelineEdit() { commitHistory(); beginTransaction(); timelineGesture = true; pointerEditing.value = true }
function endTimelineEdit(commit = true) {
  if (!timelineGesture) return
  if (!commit && project.value) { historyApplying = true; project.value.edits = JSON.parse(editsSnapshot.value); historyApplying = false; editsPending.value = false; editRevision++ }
  timelineGesture = false; pointerEditing.value = false; commitHistory()
}
function beginHistory(event: Event) { if ((event.target as HTMLElement).closest('input, textarea, select')) beginTransaction() }
function restoreHistory(direction: 'undo' | 'redo') {
  if (!project.value || locked.value || previewGesture || timelineGesture) return
  commitHistory()
  const snapshot = direction === 'undo' ? history.undo() : history.redo()
  if (!snapshot) return
  historyApplying = true; project.value.edits = JSON.parse(snapshot); historyApplying = false
  editsSnapshot.value = snapshot; editsPending.value = false; editRevision++; scheduleAutoSave()
  historyVersion.value++; selectedZoom.value = null; selectedReframe.value = null; selectedVideoClip.value = null; selection.value = null; drawing.value = false
}
function setProject(value: Project) {
  stopCorrection()
  resetAudition()
  clearTimeout(autoSaveTimer); historyGeneration++; historyQueued = false; editRevision++
  waveformRevision++; audioAnalysis.clear()
  previewReleased.value = false
  value.cursor = markRaw(value.cursor); value.cameraOffsetSeconds ??= 0; initializeTracks(value.edits)
  savedSnapshot.value = JSON.stringify({ name: value.name, edits: value.edits })
  historyApplying = true; project.value = value; historyApplying = false
  editsSnapshot.value = JSON.stringify(value.edits); editsPending.value = false
  history.reset(editsSnapshot.value); historyVersion.value++; historyGesture = false; timelineGesture = false; previewGesture = false
  previewGestureProject = null; previewRegionSnapshot = null; pointerEditing.value = false
  for (const resolve of gestureWaiters) resolve(); gestureWaiters.clear()
  time.value = outputToSource(value.edits, 0); selectedZoom.value = null; selectedReframe.value = null; selectedVideoClip.value = null; selection.value = null; drawing.value = false; playing.value = false
  if (localStorage.getItem('screenrecord-library') === null || window.innerWidth < 1100) libraryOpen.value = false
}
async function addReframe() {
  const current = project.value; const player = preview.value
  if (!current || !player || locked.value || addingReframe.value || current.edits.camera.layout === 'fullscreen') return
  addingReframe.value = true; stopCorrection()
  try {
    const frame = await player.pauseAtFrame()
    if (current !== project.value || locked.value) return
    if (frame === null) { showError('Wait for the preview frame to finish loading before setting a keyframe.'); return }
    const sample = reframeAt(current.edits.reframeKeyframes ?? [], frame.time) ?? { x: frame.x, y: frame.y, scale: 1 }
    const result = setReframeKey(current.edits.reframeKeyframes ?? [], frame.time, sample, current.duration)
    if (typeof result === 'string') { showError(result); return }
    commitHistory(); current.edits.reframeKeyframes = result.keys; commitHistory(); selectedReframe.value = result.selected
  } finally { addingReframe.value = false }
}
function selectReframe(id: string) {
  const key = project.value?.edits.reframeKeyframes?.find(item => item.id === id)
  if (!key || locked.value) return
  preview.value?.pause(); selectedReframe.value = id; seek(key.time)
}
function updateReframe(value: { id: string; field: 'x' | 'y' | 'scale'; value: number }) {
  const key = project.value?.edits.reframeKeyframes?.find(item => item.id === value.id)
  if (!key || locked.value || !Number.isFinite(value.value)) return
  preview.value?.pause(); commitHistory(); key[value.field] = clamp(value.value, value.field === 'scale' ? 1 : 0, value.field === 'scale' ? 3 : 1); commitHistory()
}
function removeReframe(id: string) {
  if (!project.value || locked.value) return
  preview.value?.pause(); commitHistory(); project.value.edits.reframeKeyframes = project.value.edits.reframeKeyframes?.filter(key => key.id !== id) ?? []; commitHistory(); selectedReframe.value = null
}
function applyPreset(settings: AppearanceSettings) {
  if (!project.value || locked.value) return
  try {
    const edits = applyAppearance(project.value.edits, settings, !!project.value.cameraPath)
    preview.value?.pause(); commitHistory(); drawing.value = false; project.value.edits = edits; commitHistory()
  } catch (failure) { showError(failure) }
}
watch(() => project.value?.edits.camera.layout, layout => { if (layout === 'fullscreen') drawing.value = false })
async function refreshProjects() {
  const request = ++projectListRequest
  const list = await command<ProjectSummary[]>('list_projects')
  if (request === projectListRequest) projects.value = list
}
function restoreProjectActionFocus() {
  void nextTick(() => {
    const origin = projectActionOrigin
    if (origin?.isConnected && !origin.closest('[inert]')) origin.focus()
    else document.querySelector<HTMLButtonElement>('.library-toggle')?.focus()
  })
}
function closeProjectMenu(restoreFocus = true) {
  if (!projectMenu.value) return
  projectMenu.value = null
  if (restoreFocus) restoreProjectActionFocus()
}
async function openProjectMenu(event: MouseEvent | KeyboardEvent, item: ProjectSummary) {
  event.preventDefault()
  if (locked.value || !desktop) return
  const element = event.currentTarget as HTMLElement
  projectActionOrigin = element.matches('button') ? element : element.querySelector<HTMLElement>('button')
  const bounds = element.getBoundingClientRect()
  const pointer = event instanceof MouseEvent && event.type === 'contextmenu' && (event.clientX !== 0 || event.clientY !== 0)
  const menu = { item, x: pointer ? event.clientX : bounds.left, y: pointer ? event.clientY : bounds.bottom }
  projectMenu.value = menu
  await nextTick()
  if (projectMenu.value?.item.id !== item.id) return
  const size = projectMenuElement.value?.getBoundingClientRect()
  projectMenu.value = { item, x: clamp(menu.x, 8, Math.max(8, innerWidth - (size?.width ?? 190) - 8)), y: clamp(menu.y, 8, Math.max(8, innerHeight - (size?.height ?? 46) - 8)) }
  projectMenuElement.value?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus()
}
function projectMenuKeyboard(event: KeyboardEvent, item: ProjectSummary) {
  if (event.key === 'ContextMenu' || event.key === 'F10' && event.shiftKey) { event.stopPropagation(); void openProjectMenu(event, item) }
}
function menuKeyboard(event: KeyboardEvent) {
  if (event.key === 'Escape' || event.key === 'Tab') { event.preventDefault(); closeProjectMenu() }
  else if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(event.key)) { event.preventDefault(); projectMenuElement.value?.querySelector<HTMLButtonElement>('[role="menuitem"]')?.focus() }
}
function outsideProjectMenu(event: PointerEvent) { if (projectMenu.value && !projectMenuElement.value?.contains(event.target as Node)) closeProjectMenu(!(event.target as HTMLElement).closest('button, input, select, textarea, a, [tabindex]')) }
function closeMenuForResize() { closeProjectMenu() }
async function confirmProjectDeletion() {
  const item = projectMenu.value?.item
  if (!item || locked.value) return
  deleteTarget.value = { ...item, name: project.value?.id === item.id ? project.value.name : item.name }; deleteError.value = ''; closeProjectMenu(false); preview.value?.pause()
  await nextTick(); deleteDialog.value?.showModal()
  deleteDialog.value?.querySelector<HTMLButtonElement>('[data-cancel-delete]')?.focus()
}
function cancelProjectDeletion() {
  if (deleting.value) return
  deleteDialog.value?.close(); deleteTarget.value = null; deleteError.value = ''; restoreProjectActionFocus()
}
function clearProject() {
  resetAudition()
  clearTimeout(autoSaveTimer); historyGeneration++; historyQueued = false; editRevision++
  historyApplying = true; project.value = null; historyApplying = false
  savedSnapshot.value = ''; editsSnapshot.value = ''; editsPending.value = false; history.reset(''); historyVersion.value++; historyGesture = false; timelineGesture = false; previewGesture = false
  previewGestureProject = null; previewRegionSnapshot = null; pointerEditing.value = false
  for (const resolve of gestureWaiters) resolve(); gestureWaiters.clear()
  selectedZoom.value = null; selectedReframe.value = null; selectedVideoClip.value = null; selection.value = null; drawing.value = false; time.value = 0; playing.value = false
}
async function deleteProject() {
  const target = deleteTarget.value
  if (!target || locked.value || deleting.value) return
  deleting.value = true; busy.value = 'Moving project to Recycle Bin'; deleteError.value = ''
  const wasCurrent = project.value?.id === target.id
  try {
    preview.value?.pause()
    await stopAudioAnalysis()
    await persist()
    if (wasCurrent) { try { await preview.value?.release() } finally { previewReleased.value = true; await nextTick() } }
    await command('delete_project', { id: target.id })
    projectListRequest++
    projects.value = projects.value.filter(item => item.id !== target.id)
    if (wasCurrent) {
      clearProject()
      const next = projects.value[0]
      if (next) { try { setProject(await command<Project>('open_project', { id: next.id })) } catch (e) { showError(e); libraryOpen.value = true } }
    }
    deleteDialog.value?.close(); deleteTarget.value = null
    notice.value = `Moved "${target.name}" to the Recycle Bin.`
  } catch (e) {
    previewReleased.value = false
    deleteError.value = `Could not delete "${target.name}". ${errorText(e)}${project.value ? ' Your edits remain open.' : ''} You can retry.`
  } finally {
    deleting.value = false; busy.value = ''
    if (!deleteTarget.value) restoreProjectActionFocus()
  }
}
async function refreshDevices(withCamera = false) {
  const request = ++deviceRequest
  devicesLoading.value = true; devicesReady.value = false
  if (recordError.value?.title === 'Could not refresh devices') recordError.value = null
  try {
    const result = await command<CaptureDevices>('list_capture_devices')
    if (request !== deviceRequest || !recordingDialogOpen.value) return
    devices.value = result
    appSelection.value = reconcileAppSelection(captureApps.value, appSelection.value, sourceKind.value === 'apps')
    if (sourceKind.value === 'apps') options.value.sourceId = appSelection.value.windowId
    else if (!filteredSources.value.some(s => s.id === options.value.sourceId)) options.value.sourceId = filteredSources.value[0]?.id ?? ''
    if (!devices.value.microphones.some(d => d.id === options.value.microphoneId)) options.value.microphoneId = null
    if (!devices.value.cameras.some(d => d.id === options.value.cameraId)) options.value.cameraId = null
    if (withCamera) {
      options.value.cameraId = options.value.cameraId ?? devices.value.cameras[0]?.id ?? null
      if (!options.value.cameraId) showRecordingError('Connect a webcam and refresh devices.', 'No camera is available')
      else rememberRecordingOptions()
    }
    devicesReady.value = true
  } catch (e) { if (request === deviceRequest && recordingDialogOpen.value) showRecordingError(e, 'Could not refresh devices') }
  finally { if (request === deviceRequest) devicesLoading.value = false }
}
function selectCaptureApp(app: CaptureApp) { appSelection.value = chooseApp(app); options.value.sourceId = appSelection.value.windowId }
function selectAppWindow(event: Event) {
  const id = (event.target as HTMLSelectElement).value
  if (!selectedApp.value?.windows.some(window => window.id === id)) return
  appSelection.value = { appId: selectedApp.value.id, windowId: id, needsReselection: false }; options.value.sourceId = id
}
async function initialize() {
  if (!desktop) return
  loading.value = true; error.value = ''; initializationError.value = ''
  const request = ++projectListRequest
  const revision = recordingRevision
  const exportRevision = exportAttempt
  try {
    const results = await Promise.allSettled([command<AppInfo>('get_app_info'), command<ProjectSummary[]>('list_projects'), command<RecordingState>('get_recording_state'), command<ExportProgress | null>('get_export_progress'), command<string | null>('get_stop_shortcut'), command<string[]>('get_recovery_issues')])
    const [appInfo, list, state, progress, shortcut, issues] = results
    if (appInfo.status === 'fulfilled') info.value = appInfo.value
    if (list.status === 'fulfilled' && request === projectListRequest) projects.value = list.value
    if (state.status === 'fulfilled' && revision === recordingRevision) recording.value = state.value
    if (progress.status === 'fulfilled' && exportRevision === exportAttempt) acceptExportProgress(progress.value)
    if (shortcut.status === 'fulfilled') stopShortcut.value = shortcut.value
    if (issues.status === 'fulfilled') recoveryIssues.value = issues.value
    initializationError.value = results.filter(result => result.status === 'rejected').map(result => errorText(result.reason)).join('\n')
  } catch (e) { initializationError.value = errorText(e) } finally { loading.value = false }
}
async function persist(): Promise<void> {
  clearTimeout(autoSaveTimer)
  const current = project.value; const generation = historyGeneration
  if (previewGesture || timelineGesture) await new Promise<void>(resolve => gestureWaiters.add(resolve))
  if (current !== project.value || generation !== historyGeneration) return
  commitHistory(); clearTimeout(autoSaveTimer)
  if (savePromise) { await savePromise; if (current !== project.value || generation !== historyGeneration) return; if (dirty.value) return persist(); return }
  if (!project.value || !dirty.value) return
  project.value.edits = normalizeEdits(project.value.edits, project.value.duration)
  commitHistory(); clearTimeout(autoSaveTimer)
  const revision = editRevision
  const payload = { id: project.value.id, name: project.value.name, edits: JSON.parse(editsSnapshot.value) as Project['edits'] }
  saving.value = true
  savePromise = (async () => {
    const saved = await command<SavedEdits>('save_project_edits', payload)
    if (project.value === current && generation === historyGeneration) {
      if (editRevision === revision && project.value!.name === payload.name && !historyGesture) {
        historyApplying = true; project.value!.name = saved.name; project.value!.edits = saved.edits; historyApplying = false
        editsSnapshot.value = JSON.stringify(saved.edits); editsPending.value = false; history.observe(editsSnapshot.value); history.commit(); historyVersion.value++
      }
      savedSnapshot.value = JSON.stringify({ name: saved.name, edits: saved.edits })
    }
    const summary = projects.value.find(p => p.id === payload.id); if (summary) { summary.name = saved.name; summary.duration = outputDuration(saved.edits) }
  })()
  try {
    await savePromise
    if (lastSaveFailure !== null && error.value === errorText(lastSaveFailure)) error.value = ''
    lastSaveFailure = null
    if (dirty.value) scheduleAutoSave()
  } catch (failure) {
    lastSaveFailure = failure
    throw failure
  } finally { saving.value = false; savePromise = null }
}
async function requestClose() {
  if (closing.value) return
  cancelCountdown(); stopCorrection()
  if (recordingDialogOpen.value) { try { await recordingTrial.value?.discardAndWait(); await sourceRegion.value?.closeAndWait() } catch (failure) { showRecordingError(failure, 'Test cleanup needed'); return } }
  if (maskTarget) { try { await cancelMaskTracking() } catch (failure) { showError(failure); return } }
  if (locked.value) { showError(recording.value.phase === 'finalizing' || busy.value === 'Finalizing recording' ? 'Wait for recording finalization to finish before closing.' : busy.value || opening.value ? 'Wait for the current operation to finish before closing.' : transcribing.value ? 'Cancel subtitle generation before closing.' : 'Stop recording or cancel export before closing.'); return }
  closing.value = true; error.value = ''; notice.value = ''; preview.value?.pause()
  let timeout: ReturnType<typeof setTimeout> | undefined
  try {
    await Promise.all([cameraPreview.value?.stop(), microphoneMeter.value?.stop(), stopAudioAnalysis(), localSetup.value?.cancelAndWait()])
    await Promise.race([
      persist(),
      new Promise<never>((_, reject) => { timeout = setTimeout(() => reject(new Error('Closing is taking too long. Your changes are still open. Wait for saving to finish, then try closing again.')), 10_000) }),
    ])
    clearTimeout(timeout)
    await command('close_app')
  } catch (e) { showError(e) }
  finally { clearTimeout(timeout); closing.value = false }
}
async function openProject(id: string, preserveError = false) {
  if (locked.value || opening.value) return
  stopCorrection()
  opening.value = true; if (!preserveError) error.value = ''; preview.value?.pause()
  try { await stopAudioAnalysis(); await persist(); setProject(await command<Project>('open_project', { id })) } catch (e) { showError(e) } finally { opening.value = false }
}
async function newRecording(withCamera = false) {
  if (locked.value) return
  busy.value = 'Preparing recording'; error.value = ''; preview.value?.pause()
  try {
    await stopAudioAnalysis(); await persist()
    recordError.value = null; recordingFailed.value = false; devicesReady.value = false; appSearch.value = ''
    recordingDialogOpen.value = true; recordDialog.value?.showModal(); busy.value = ''
    await refreshDevices(withCamera)
  } catch (e) { showError(e) }
  finally { if (busy.value === 'Preparing recording') busy.value = '' }
}
async function closeRecordingDialog() {
  if (busy.value) return
  cancelCountdown()
  busy.value = 'Closing recording setup'
  try { await Promise.all([cameraPreview.value?.stop(), microphoneMeter.value?.stop(), recordingTrial.value?.discardAndWait(), sourceRegion.value?.closeAndWait()]); recordDialog.value?.close(); recordingDialogOpen.value = false }
  catch (failure) { showRecordingError(`Use the cleanup retry controls, then close this dialog. ${errorText(failure)}`, 'Device cleanup needed') }
  finally { busy.value = '' }
}
function cancelCountdown() { if (!countdownController && countdown.value === null) return; recordingStartRequest++; countdownController?.abort(); countdownController = null; countdown.value = null; void nextTick(() => { if (recordingDialogOpen.value && !busy.value) recordStartButton.value?.focus() }) }
function recordingDialogClosed() { if (recordDialog.value?.open) return; cancelCountdown(); recordingDialogOpen.value = false; deviceRequest++; devicesLoading.value = false; devicesReady.value = false; recordError.value = null; void Promise.all([cameraPreview.value?.stop(), microphoneMeter.value?.stop(), recordingTrial.value?.discardAndWait(), sourceRegion.value?.closeAndWait()]).catch(showError) }
async function prepareRecordingTest() { await stopAudioAnalysis(); await Promise.all([cameraPreview.value?.stop(), microphoneMeter.value?.stop(), sourceRegion.value?.closeAndWait()]) }
async function startRecording() {
  if (!selectedSource.value || !devicesReady.value || devicesLoading.value || busy.value || trialBusy.value || closing.value || countdown.value !== null) return
  const request = ++recordingStartRequest
  const selected = JSON.parse(JSON.stringify(options.value)) as RecordingOptions
  let terminal: RecordingTerminal | undefined
  let startedProjectId: string | null = null
  let phase: 'analysis' | 'cleanup' | 'save' | 'capture' = 'analysis'
  busy.value = 'Preparing recording'; error.value = ''; notice.value = ''; recordError.value = null
  try {
    await stopAudioAnalysis()
    phase = 'cleanup'
    await Promise.all([cameraPreview.value?.stop(), microphoneMeter.value?.stop(), recordingTrial.value?.discardAndWait(), sourceRegion.value?.closeAndWait()])
    phase = 'save'
    await persist()
    const controller = new AbortController(); countdownController = controller
    if (countdownSeconds.value > 0) { countdown.value = countdownSeconds.value; busy.value = ''; await nextTick(); countdownCancel.value?.focus() }
    const start = await runCountdown(countdownSeconds.value, controller.signal, value => { countdown.value = value })
    if (!start || controller.signal.aborted || request !== recordingStartRequest) return
    countdownController = null; countdown.value = null; busy.value = 'Starting recording'; recordingRevision++
    phase = 'capture'
    pendingRecordingStart = { request, terminals: new Map() }
    recording.value = await command<RecordingState>('start_recording', { options: selected })
    startedProjectId = recording.value.projectId
    if (startedProjectId) terminal = pendingRecordingStart.terminals.get(startedProjectId)
    pendingRecordingStart = null
    recordDialog.value?.close()
  } catch (e) {
    if (phase === 'capture') { recordingFailed.value = true; showRecordingError(e, 'Recording did not start') }
    else if (phase === 'cleanup') showRecordingError('Use Retry stop preview or Retry stop test to release the device, then try again.', 'Device cleanup needed')
    else if (phase === 'analysis') showRecordingError(e, 'Could not finish audio analysis')
    else showRecordingError(e, 'Could not save the current project')
  } finally {
    if (pendingRecordingStart?.request === request) pendingRecordingStart = null
    if (request === recordingStartRequest) { busy.value = ''; countdown.value = null; countdownController = null }
  }
  if (terminal?.project) try { await recordingStopped(terminal.project) } catch (failure) { showError(failure) }
  if (terminal?.error && startedProjectId) recordingError({ projectId: startedProjectId, error: terminal.error })
}
async function pauseRecording() {
  if (busy.value || !recording.value.active || !recording.value.projectId || recording.value.phase === 'finalizing') return
  const resume = recording.value.paused
  const projectId = recording.value.projectId
  busy.value = resume ? 'Resuming recording' : 'Pausing recording'; error.value = ''; recordingRevision++
  try { recording.value = await command<RecordingState>(resume ? 'resume_recording' : 'pause_recording', { projectId }) }
  catch (failure) { showError(failure); try { recording.value = await command<RecordingState>('get_recording_state') } catch { /* Keep Stop available when status cannot be refreshed. */ } }
  finally { busy.value = '' }
}
async function stopRecording() {
  if (busy.value || !recording.value.active || !recording.value.projectId || recording.value.phase === 'finalizing') return
  busy.value = 'Finalizing recording'; error.value = ''; recordingRevision++
  try {
    const recorded = await command<Project>('stop_recording', { projectId: recording.value.projectId })
    await displayRecording(recorded)
  } catch (e) { showError(e); try { recording.value = await command<RecordingState>('get_recording_state'); await refreshProjects() } catch { /* The original failure remains visible. */ } }
  finally { busy.value = '' }
}
async function displayRecording(recorded: Project) {
  recording.value = idleRecording()
  await refreshProjects()
  if (project.value?.id === recorded.id) return
  await persist()
  setProject(recorded)
}
async function recordingStopped(recorded: Project) {
  if (queueRecordingTerminal(recorded.id, { project: recorded })) return
  if (recording.value.active && recording.value.projectId !== recorded.id) { await refreshProjects(); return }
  recordingRevision++; recording.value = idleRecording()
  if (locked.value) { await refreshProjects(); return }
  opening.value = true
  try { await displayRecording(recorded) } finally { opening.value = false }
}
function recordingError(value: RecordingError) {
  if (typeof value === 'string') { showError(value); return }
  if (queueRecordingTerminal(value.projectId, { error: value.error })) return
  if (recording.value.projectId && recording.value.projectId !== value.projectId) return
  showError(value.error)
}
function queueRecordingTerminal(projectId: string, terminal: RecordingTerminal) {
  if (!pendingRecordingStart) return false
  const terminals = pendingRecordingStart.terminals
  terminals.set(projectId, { ...terminals.get(projectId), ...terminal })
  if (terminals.size > 4) terminals.delete(terminals.keys().next().value!)
  return true
}
async function poll() {
  if (pollPending || !desktop || ['Starting recording', 'Pausing recording', 'Resuming recording', 'Finalizing recording'].includes(busy.value)) return
  pollPending = true
  const revision = recordingRevision
  try {
    const state = await command<RecordingState>('get_recording_state')
    if (revision !== recordingRevision) return
    const previousProjectId = recording.value.active ? recording.value.projectId : null
    recording.value = state
    if (previousProjectId && !state.active && !busy.value) {
      await refreshProjects()
      if (revision !== recordingRevision) return
      if (projects.value.some(item => item.id === previousProjectId)) await openProject(previousProjectId, true)
    }
    if (exporting.value && busy.value !== 'Starting export') {
      const attempt = exportAttempt
      try { const progress = await command<ExportProgress | null>('get_export_progress'); if (attempt === exportAttempt && busy.value !== 'Starting export') acceptExportProgress(progress) }
      catch (failure) { if (attempt === exportAttempt && exporting.value && busy.value !== 'Starting export') exportStatusError.value = `Status check failed: ${errorText(failure)} Retrying automatically.` }
    }
    if (trackingMask.value) acceptMaskTracking(await command<MaskTrackingProgress | null>('get_mask_tracking_progress'))
    if (transcribing.value) acceptTranscription(await command<TranscriptionProgress | null>('get_transcription_progress'))
  } catch (e) { if (revision === recordingRevision && (recording.value.active || exporting.value || transcribing.value || trackingMask.value)) showError(e) }
  finally { pollPending = false }
}
function seek(value: number) { if (!project.value) return; if (correctionRegion.value) stopCorrection(); time.value = retainedSource(project.value.edits, value); preview.value?.seek(time.value) }
function selectVideo(id: string) { selectedVideoClip.value = id; selectedZoom.value = null; selection.value = null; showTool('canvas') }
function splitAtPlayhead() {
  if (!project.value || locked.value) return
  const result = splitVideo(project.value.edits, Math.round(time.value * project.value.fps) / project.value.fps)
  if (typeof result === 'string') { notice.value = result; return }
  preview.value?.pause(); commitHistory(); project.value.edits.videoClips = result.clips; commitHistory(); selectVideo(result.selected)
}
function deleteVideoClip() {
  if (!project.value || !selectedVideoClip.value || locked.value) return
  const clips = removeVideo(project.value.edits, selectedVideoClip.value)
  if (typeof clips === 'string') { notice.value = clips; return }
  preview.value?.pause(); commitHistory(); project.value.edits.videoClips = clips; commitHistory(); selectedVideoClip.value = null; seek(time.value)
}
function openCutRange() {
  if (!project.value || locked.value) return
  preview.value?.pause(); cutError.value = ''; cutStart.value = Math.max(0, Math.min(outputTime.value, editedDuration.value - .1)); cutEnd.value = Math.min(editedDuration.value, cutStart.value + 1); cutDialog.value?.showModal()
}
function applyCutRange() {
  if (!project.value || locked.value) return
  const clips = cutVideoRange(project.value.edits, cutStart.value, cutEnd.value)
  if (typeof clips === 'string') { cutError.value = clips; return }
  commitHistory(); project.value.edits.videoClips = clips; commitHistory(); selectedVideoClip.value = null; cutDialog.value?.close(); seek(outputToSource(project.value.edits, cutStart.value))
}
function setSourceTrim(field: 'trimStart' | 'trimEnd', value: number) {
  if (!project.value || locked.value) return
  const message = changeSourceTrim(project.value.edits, field, value, project.value.duration)
  if (message) showError(message)
}
function insertZoom() {
  if (!project.value) return
  const added = addZoom(project.value.edits, time.value, project.value.duration)
  if (!added) { notice.value = 'There is no room for a zoom here. Move the playhead outside the existing segments.'; return }
  project.value.edits.zooms.push(added); project.value.edits.zooms.sort((a, b) => a.start - b.start); selectZoom(added.id)
}
function selectZoom(id: string | null) { selectedZoom.value = id; selectedVideoClip.value = null; selection.value = null; inspector.value?.showZoom() }
function selectTrack(value: TrackSelection | null) { selection.value = value; selectedVideoClip.value = null; selectedZoom.value = null }
function selectCamera() { selection.value = null; selectedVideoClip.value = null; selectedZoom.value = null; showTool('camera') }
function selectRegion(id: string) { selectTrack({ kind: 'redactions', id }); showTool('redactions') }
function updateRegion(value: Pick<Redaction, 'id' | 'x' | 'y' | 'width' | 'height'>) {
  if (!previewGesture || previewGestureProject !== project.value) return
  const region = project.value?.edits.redactions.find(item => item.id === value.id); if (!region) return
  const changed = region.tracking ? manualCorrection(region, time.value, value) : { ...region, ...value }
  if (maskWithinLimit(changed)) Object.assign(region, changed)
}
function maskWithinLimit(region: Redaction) { const message = trackingBudgetError(project.value?.edits.redactions ?? [], region); if (message) showError(message); return !message }
function updateMask(region: Redaction) { if (!project.value || locked.value || !maskWithinLimit(region)) return; commitHistory(); const index = project.value.edits.redactions.findIndex(item => item.id === region.id); if (index >= 0) project.value.edits.redactions[index] = region; commitHistory() }
function stopCorrection() { if (correctionRegion.value) preview.value?.hideOriginal(); correctionRegion.value = null; drawing.value = false }
function correctMask(id: string) { const region = project.value?.edits.redactions.find(item => item.id === id); if (!region || locked.value || time.value < region.start || time.value >= region.end || project.value?.edits.camera.layout === 'fullscreen') return; preview.value?.pause(); correctionRegion.value = id; drawing.value = true }
async function startMaskTracking(value: { regionId: string; start?: number }) {
  const current = project.value; const region = current?.edits.redactions.find(item => item.id === value.regionId)
  if (!current || !region || locked.value) return
  if (region.end - region.start > 300 + 1e-9) { showError('Shorten this region to 300 seconds or less before tracking.'); return }
  stopCorrection(); preview.value?.pause(); busy.value = 'Starting mask tracking'; error.value = ''
  try {
    await stopAudioAnalysis(); await persist()
    const target = { projectId: current.id, regionId: region.id, jobId: crypto.randomUUID(), cancelled: false }; maskTarget = target
    maskProgress.value = { ...target, state: 'running', progress: 0, error: null, tracking: null }
    const task = command<void>('start_mask_tracking', { id: current.id, regionId: region.id, jobId: target.jobId, ...(value.start === undefined ? {} : { start: value.start }) }); maskStartPromise = task
    try { await task } finally { if (maskStartPromise === task) maskStartPromise = null }
  } catch (failure) { maskTarget = null; maskProgress.value = null; showError(failure) } finally { busy.value = '' }
}
function acceptMaskTracking(value: MaskTrackingProgress | null) {
  const target = maskTarget
  if (!value || !target || value.jobId !== target.jobId || value.projectId !== target.projectId || value.regionId !== target.regionId) return
  maskProgress.value = value
  if (value.state === 'running') return
  maskTarget = null
  if (target.cancelled || value.state === 'cancelled') { notice.value = 'Mask tracking cancelled. The previous mask is unchanged.'; return }
  if (value.state === 'failed') { showError(value.error ?? 'Mask tracking failed. The previous mask is unchanged.'); return }
  const region = project.value?.edits.redactions.find(item => item.id === target.regionId)
  if (project.value?.id !== target.projectId || !region || !value.tracking) return
  const changed = { ...region, tracking: { ...value.tracking, reviewed: false } }; if (!maskWithinLimit(changed)) return
  commitHistory(); Object.assign(region, changed); commitHistory(); selection.value = { kind: 'redactions', id: region.id }; notice.value = 'Tracking finished. Review the mask and correct any lost sections before export.'
}
async function cancelMaskTracking(): Promise<void> {
  if (maskCancelPromise) return maskCancelPromise
  const task = (async () => {
    const target = maskTarget; if (!target) return
    target.cancelled = true; await maskStartPromise?.catch(() => undefined)
    if (maskTarget?.jobId !== target.jobId) return
    await command('cancel_mask_tracking', { jobId: target.jobId })
    while (maskTarget?.jobId === target.jobId) { acceptMaskTracking(await command<MaskTrackingProgress | null>('get_mask_tracking_progress')); if (maskTarget?.jobId === target.jobId) await new Promise(resolve => setTimeout(resolve, 100)) }
  })(); maskCancelPromise = task
  try { await task } finally { if (maskCancelPromise === task) maskCancelPromise = null }
}
function updateCamera(value: Pick<Project['edits']['camera'], 'x' | 'y' | 'width'>) { if (previewGesture && previewGestureProject === project.value && project.value) Object.assign(project.value.edits.camera, value) }
function showTool(tool: string) { inspector.value?.showTool(tool) }
function setActiveTool(tool: string) { if (tool !== 'redactions' && correctionRegion.value) stopCorrection(); activeTool.value = tool; if (tool !== 'canvas') selectedVideoClip.value = null }
function removeZoom(id: string) { if (!project.value) return; project.value.edits.zooms = project.value.edits.zooms.filter(z => z.id !== id); selectedZoom.value = null }
async function regenerate() {
  if (!project.value || locked.value) return
  const id = project.value.id
  busy.value = 'Generating zooms'; error.value = ''
  try { await stopAudioAnalysis(); await persist(); const regenerated = await command<Project>('regenerate_zooms', { id }); if (project.value?.id === id) { project.value.edits.zooms = regenerated.edits.zooms; commitHistory() } } catch (e) { showError(e) } finally { busy.value = '' }
}
async function importMusic() {
  if (!project.value || locked.value) return
  busy.value = 'Importing music'; error.value = ''; preview.value?.pause()
  try {
    await stopAudioAnalysis()
    const path = await open({ multiple: false, filters: [{ name: 'Audio', extensions: ['mp3', 'wav', 'm4a', 'aac', 'flac', 'ogg', 'opus'] }] })
    if (!path || typeof path !== 'string') return
    await persist()
    const clip = await command<MusicClip>('import_music', { id: project.value.id, path, start: clamp(time.value, project.value.edits.trimStart, project.value.edits.trimEnd - .1) })
    project.value.edits.music.push(clip); selectTrack({ kind: 'music', id: clip.id }); commitHistory()
  } catch (e) { showError(e) } finally { busy.value = '' }
}
function drawRegion(rectangle: Pick<Redaction, 'x' | 'y' | 'width' | 'height'>) {
  if (!project.value || locked.value || project.value.edits.camera.layout === 'fullscreen') return
  if (correctionRegion.value) { const region = project.value.edits.redactions.find(item => item.id === correctionRegion.value); stopCorrection(); if (region) updateMask(manualCorrection(region, time.value, rectangle)); return }
  const start = clamp(time.value, project.value.edits.trimStart, project.value.edits.trimEnd - .1)
  const end = Math.min(start + 3, project.value.edits.trimEnd)
  const region: Redaction = { id: crypto.randomUUID(), start, end, ...rectangle, mode: 'solid', strength: 16 }
  if (project.value.edits.redactions.length >= 100 || !redactionsFit([...project.value.edits.redactions, region])) { showError('At most 100 regions and 16 overlapping redactions are supported.'); return }
  project.value.edits.redactions.push(region); drawing.value = false; selectTrack({ kind: 'redactions', id: region.id }); commitHistory()
}
function toggleDrawing() { if (correctionRegion.value) { stopCorrection(); return }; if (project.value?.edits.camera.layout === 'fullscreen') return; preview.value?.pause(); drawing.value = !drawing.value }
async function openTranscription() {
  if (!project.value || locked.value) return
  preview.value?.pause(); error.value = ''; busy.value = 'Preparing subtitles'
  try {
    await stopAudioAnalysis()
    transcriptionOptions.value.apiKey = ''; transcriptionOptions.value.provider = 'local'; localReady.value = false; transcriptionDialogOpen.value = true
    transcriptionOptions.value.source = project.value.microphonePath ? 'microphone' : 'system'
    transcriptionDialog.value?.showModal()
  } catch (failure) { showError(failure) } finally { busy.value = '' }
}
async function closeTranscriptionDialog() {
  if (busy.value || transcriptionDialogClosing.value) return
  transcriptionDialogClosing.value = true
  try { await localSetup.value?.cancelAndWait(); transcriptionDialog.value?.close() } catch (failure) { showError(failure) }
  finally { transcriptionDialogClosing.value = false }
}
async function changeTranscriptionProvider(event: Event) {
  const input = event.target as HTMLSelectElement; const provider = input.value as TranscriptionOptions['provider']
  input.value = transcriptionOptions.value.provider
  if (busy.value || transcriptionDialogClosing.value) return
  transcriptionDialogClosing.value = true
  try { if (provider !== 'local') await localSetup.value?.cancelAndWait(); transcriptionOptions.value.provider = provider; transcriptionOptions.value.apiKey = '' } catch (failure) { showError(failure) }
  finally { transcriptionDialogClosing.value = false }
}
async function startTranscription() {
  if (!project.value || locked.value || transcriptionDialogClosing.value || (transcriptionOptions.value.provider === 'local' ? !localReady.value || localInstallBusy.value : !transcriptionOptions.value.apiKey.trim())) return
  busy.value = 'Starting transcription'; error.value = ''; transcriptionTarget = { projectId: project.value.id, jobId: crypto.randomUUID() }
  try {
    await stopAudioAnalysis()
    await persist()
    transcription.value = { state: 'running', projectId: project.value.id, jobId: transcriptionTarget.jobId, progress: 0, error: null, cues: null }
    const { provider, source, language, apiKey } = transcriptionOptions.value
    await command('start_transcription', { id: project.value.id, jobId: transcriptionTarget.jobId, options: { provider, source, language, ...(provider === 'cloud' ? { apiKey: apiKey.trim() } : {}) } })
    transcriptionDialog.value?.close()
  } catch (e) { transcriptionTarget = null; transcription.value = null; showError(e) }
  finally { transcriptionOptions.value.apiKey = ''; busy.value = '' }
}
function acceptTranscription(value: TranscriptionProgress | null) {
  if (!value || !transcriptionTarget || value.jobId !== transcriptionTarget.jobId || value.projectId !== transcriptionTarget.projectId) return
  transcription.value = value
  if (value.state === 'running') return
  transcriptionTarget = null
  if (value.state === 'completed' && value.cues && project.value?.id === value.projectId) {
    project.value.edits.subtitles = value.cues; commitHistory()
    selection.value = value.cues[0] ? { kind: 'subtitles', id: value.cues[0].id } : null
    notice.value = value.cues.length ? `Generated ${value.cues.length} subtitle cues. Review the text before exporting.` : 'No speech was found in the selected audio.'
  } else if (value.state === 'failed') showError(value.error ?? 'Subtitle generation failed.')
  else if (value.state === 'cancelled') notice.value = 'Subtitle generation cancelled.'
}
async function cancelTranscription() { if (!transcriptionTarget || busy.value) return; try { await command('cancel_transcription', { jobId: transcriptionTarget.jobId }); acceptTranscription(await command<TranscriptionProgress | null>('get_transcription_progress')) } catch (e) { showError(e) } }
async function exportSrt() {
  if (!project.value || locked.value) return
  busy.value = 'Saving subtitles'; error.value = ''
  try {
    const path = await save({ defaultPath: `${project.value.name.replace(/[<>:"/\\|?*]/g, '_')}.srt`, filters: [{ name: 'SubRip subtitles', extensions: ['srt'] }] })
    if (!path) return
    await stopAudioAnalysis(); await persist(); await command('export_subtitles', { id: project.value.id, outputPath: path.toLowerCase().endsWith('.srt') ? path : `${path}.srt` })
    notice.value = 'Subtitles exported.'
  } catch (e) { showError(e) } finally { busy.value = '' }
}
async function importSrt() {
  const current = project.value; if (!current || locked.value) return
  busy.value = 'Importing subtitles'; error.value = ''; preview.value?.pause()
  try {
    const path = await open({ multiple: false, filters: [{ name: 'SubRip subtitles', extensions: ['srt'] }] })
    if (!path) return
    await stopAudioAnalysis(); await persist()
    const cues = await command<SubtitleCue[]>('import_subtitles', { id: current.id, path })
    if (project.value?.id !== current.id) return
    commitHistory(); current.edits.subtitles = cues; commitHistory(); selection.value = cues[0] ? { kind: 'subtitles', id: cues[0].id } : null
    notice.value = `Imported ${cues.length} subtitle cues. SRT has cue timing only; words are not timed.`
  } catch (failure) { showError(failure) } finally { busy.value = '' }
}
async function openExport() {
  if (!project.value || locked.value) return
  stopCorrection()
  if (unreviewedMasks.value.length) { selectRegion(unreviewedMasks.value[0].id); showError('Review tracked masks before exporting. Use Mark tracking reviewed after inspecting each region.'); return }
  busy.value = 'Preparing export'; error.value = ''; preview.value?.pause()
  try { await stopAudioAnalysis(); await persist(); exportSession++; exportAttempt++; exportOptions.value.fps = project.value.fps; exportOptions.value.outputPath = ''; exportProgress.value = null; exportStatusError.value = ''; exportTargetPath = null; exportDialog.value?.showModal(); exportDialogOpen.value = true } catch (e) { showError(e) }
  finally { busy.value = '' }
}
function exportClosed() { exportDialogOpen.value = false; exportSession++; exportPlanRequest++; outputRequest++; whatsappPlan.value = null; whatsappPlanSnapshot.value = ''; exportPlanLoading.value = false; exportPlanError.value = ''; choosingOutput.value = false }
function closeExportDialog() { exportDialog.value?.close(); exportClosed() }
watch(() => project.value, (current, previous) => { if (current !== previous && exportDialogOpen.value) closeExportDialog() })
watch([exportPreset, exportDialogOpen], () => { if (exportDialogOpen.value && exportPreset.value === 'whatsapp') void loadWhatsAppPlan(); else { exportPlanRequest++; whatsappPlan.value = null; exportPlanLoading.value = false; exportPlanError.value = '' } })
async function loadWhatsAppPlan() {
  const current = project.value; const session = exportSession; const request = ++exportPlanRequest
  if (!current || !exportDialogOpen.value || exportPreset.value !== 'whatsapp') return
  const matches = () => request === exportPlanRequest && session === exportSession && current === project.value && exportDialogOpen.value && exportPreset.value === 'whatsapp'
  whatsappPlan.value = null; whatsappPlanSnapshot.value = ''; exportPlanError.value = ''; exportPlanLoading.value = true
  try {
    await persist(); if (!matches()) return
    const snapshot = savedSnapshot.value
    const plan = await command<WhatsAppExportPlan>('get_whatsapp_export_plan', { id: current.id })
    if (!matches()) return
    if (snapshot !== savedSnapshot.value || dirty.value) throw new Error('The project changed. Calculate the export size again.')
    if (![plan.width, plan.height, plan.videoBitrate, plan.audioBitrate].every(value => Number.isFinite(value) && value > 0)) throw new Error('Export dimensions are unavailable. Try calculating the size again.')
    whatsappPlan.value = plan; whatsappPlanSnapshot.value = snapshot
  } catch (failure) { if (matches()) exportPlanError.value = errorText(failure) }
  finally { if (matches()) exportPlanLoading.value = false }
}
async function chooseOutput() {
  const current = project.value; const session = exportSession; const preset = exportPreset.value
  if (!current || !exportDialogOpen.value || choosingOutput.value || busy.value || exporting.value) return
  const request = ++outputRequest; choosingOutput.value = true
  try {
    const name = current.name.replace(/[<>:"/\\|?*]/g, '_')
    const path = await save({ defaultPath: `${name}${preset === 'whatsapp' ? '-whatsapp' : ''}.mp4`, filters: [{ name: 'MP4 video', extensions: ['mp4'] }] })
    if (path && request === outputRequest && session === exportSession && current === project.value && preset === exportPreset.value && exportDialogOpen.value) exportOptions.value.outputPath = path.toLowerCase().endsWith('.mp4') ? path : `${path}.mp4`
  } catch (e) { if (session === exportSession) showError(e) }
  finally { if (request === outputRequest) choosingOutput.value = false }
}
async function startExport() {
  const current = project.value; const session = exportSession; const preset = exportPreset.value; const path = exportOptions.value.outputPath
  if (!current || !path || exporting.value || busy.value || choosingOutput.value || !exportDialogOpen.value || preset === 'whatsapp' && !validWhatsAppPlan.value) return
  stopCorrection(); if (unreviewedMasks.value.length) { showError('Review tracked masks before exporting.'); return }
  const attempt = ++exportAttempt
  busy.value = 'Starting export'; error.value = ''
  let admitted = false
  try {
    await stopAudioAnalysis()
    await persist()
    if (current !== project.value || session !== exportSession || !exportDialogOpen.value || preset !== exportPreset.value || path !== exportOptions.value.outputPath) return
    if (preset === 'whatsapp' && !validWhatsAppPlan.value) { exportPlanError.value = 'The project changed. Calculate the export size again.'; return }
    const options: ExportOptions = preset === 'whatsapp' ? { outputPath: path, preset, resolution: 'auto', fps: 30 } : { ...exportOptions.value, preset: 'standard' }
    exportTargetPath = path; exportProgress.value = { state: 'running', progress: 0, outputPath: path, error: null }
    await command('export_project', { id: current.id, options })
    admitted = true
    if (attempt !== exportAttempt) return
    if (busy.value === 'Starting export') busy.value = ''
    const progress = await command<ExportProgress | null>('get_export_progress')
    if (attempt === exportAttempt) acceptExportProgress(progress)
  } catch (e) {
    if (attempt !== exportAttempt) return
    if (admitted) { if (exportProgress.value?.state === 'running') exportStatusError.value = `Export started. Status check failed: ${errorText(e)} Retrying status automatically.` }
    else { if (exportProgress.value?.state === 'running') exportProgress.value = { state: 'failed', progress: exportProgress.value.progress, outputPath: path, error: errorText(e) }; showError(e) }
  } finally { if (attempt === exportAttempt && busy.value === 'Starting export') busy.value = '' }
}
async function cancelExport() { if (busy.value === 'Starting export') return; try { await command('cancel_export') } catch (e) { showError(e) } }
async function reveal(path: string) { try { await command('reveal_file', { path }) } catch (e) { showError(e) } }
function keyboard(event: KeyboardEvent) {
  const target = event.target as HTMLElement
  if (event.defaultPrevented || event.isComposing) return
  if (projectMenu.value || deleteDialog.value?.open || cutDialog.value?.open) return
  if (!pointerEditing.value && !event.ctrlKey && !event.metaKey && !event.altKey && !target.closest('input, select, textarea, [contenteditable]') && !target.isContentEditable && project.value && !locked.value && !recordDialog.value?.open && !exportDialogOpen.value && !transcriptionDialog.value?.open) {
    if (event.key.toLowerCase() === 's') { event.preventDefault(); splitAtPlayhead(); return }
    if (event.key === 'Delete' && selectedVideoClip.value) { event.preventDefault(); deleteVideoClip(); return }
  }
  if (event.key === 'Escape' && drawing.value && !recordDialog.value?.open && !exportDialogOpen.value && !transcriptionDialog.value?.open) { event.preventDefault(); stopCorrection(); return }
  if ((event.ctrlKey || event.metaKey) && !target.closest('input, textarea, [contenteditable]') && !locked.value && !recordDialog.value?.open && !exportDialogOpen.value && !transcriptionDialog.value?.open) {
    if (event.key.toLowerCase() === 'z') { event.preventDefault(); restoreHistory(event.shiftKey ? 'redo' : 'undo'); return }
    if (event.key.toLowerCase() === 'y') { event.preventDefault(); restoreHistory('redo'); return }
  }
  if ((event.ctrlKey || event.metaKey) && event.key.toLowerCase() === 's') { event.preventDefault(); void persist().catch(showError); return }
  if (pointerEditing.value || event.ctrlKey || event.metaKey || event.altKey || target.closest('input, select, textarea, button, a, summary, [role="tab"], [contenteditable]') || target.isContentEditable || recordDialog.value?.open || exportDialogOpen.value || transcriptionDialog.value?.open || !project.value || locked.value) return
  if (event.code === 'Space') { event.preventDefault(); preview.value?.toggle() }
  if (event.key === 'ArrowLeft') { event.preventDefault(); seek(outputToSource(project.value.edits, outputTime.value - (event.shiftKey ? 1 : 1 / project.value.fps))) }
  if (event.key === 'ArrowRight') { event.preventDefault(); seek(outputToSource(project.value.edits, outputTime.value + (event.shiftKey ? 1 : 1 / project.value.fps))) }
}
onMounted(async () => {
  window.addEventListener('keydown', keyboard)
  window.addEventListener('pointerdown', outsideProjectMenu)
  window.addEventListener('resize', closeMenuForResize)
  if (desktop) {
    try {
      unlisteners.push(await getCurrentWindow().onCloseRequested(event => {
        event.preventDefault()
        return requestClose()
      }))
    } catch (e) { showError(e) }
  }
  await initialize()
  if (desktop) {
    try {
      unlisteners.push(...await Promise.all([
        listen<ExportProgress>('export-progress', e => acceptExportProgress(e.payload, true)),
        listen<{ jobId: string; projectId: string; source: string; progress: number }>('audio-analysis-progress', e => audioAnalysis.progress(e.payload)),
        listen<MaskTrackingProgress>('mask-tracking-progress', e => acceptMaskTracking(e.payload)),
        listen<TranscriptionProgress>('transcription-progress', e => { acceptTranscription(e.payload) }),
        listen<string>('close-blocked', e => { if (maskTarget || recordingDialogOpen.value) void requestClose(); else if (!locked.value && (localInstallBusy.value || Object.values(audioAnalysis.states).some(state => state.state === 'loading'))) void requestClose(); else showError(e.payload || 'Stop recording or cancel export before closing.') }),
        listen<Project>('recording-stopped', e => { void recordingStopped(e.payload).catch(showError) }),
        listen<RecordingError>('recording-error', e => { recordingError(e.payload) }),
        listen('library-updated', () => { void refreshProjects().catch(showError) }),
        listen('recovery-issues-updated', () => { void command<string[]>('get_recovery_issues').then(issues => { recoveryIssues.value = issues }).catch(showError) }),
      ]))
      await Promise.all([
        refreshProjects(),
        command<string[]>('get_recovery_issues').then(issues => { recoveryIssues.value = issues }),
      ])
    } catch (e) { showError(e) }
    polling = setInterval(() => void poll(), 600)
  }
})
onBeforeUnmount(() => { cancelCountdown(); void stopAudioAnalysis().catch(() => undefined); clearTimeout(autoSaveTimer); clearInterval(polling); unlisteners.forEach(f => f()); window.removeEventListener('keydown', keyboard); window.removeEventListener('pointerdown', outsideProjectMenu); window.removeEventListener('resize', closeMenuForResize) })
</script>

<template>
  <div class="app-shell">
    <header class="app-header">
      <h1 :class="{ 'sr-only': project }">Screenrecord</h1>
      <button type="button" class="library-toggle" :aria-expanded="libraryOpen" aria-controls="project-library" :disabled="locked" @click="toggleLibrary"><svg width="17" height="17" viewBox="0 0 20 20" fill="none" aria-hidden="true"><rect x="2" y="3" width="16" height="14" rx="2" stroke="currentColor" stroke-width="1.5"/><path d="M7 3v14" stroke="currentColor" stroke-width="1.5"/></svg>Projects</button>
      <div v-if="project" class="project-title-group"><label class="sr-only" for="project-name">Project name</label><input id="project-name" v-model="project.name" maxlength="120" class="title-input" :disabled="locked" @change="persist().catch(showError)" /><span class="save-state" role="status">{{ saving ? 'Saving...' : dirty ? 'Unsaved changes' : 'Saved' }}</span></div>
      <div v-if="project" class="history-actions"><button type="button" class="icon-button" :disabled="!canUndo || locked" aria-label="Undo" title="Undo (Ctrl+Z)" @click="restoreHistory('undo')"><svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m7 4-4 4 4 4M3 8h8a5 5 0 0 1 0 10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg></button><button type="button" class="icon-button" :disabled="!canRedo || locked" aria-label="Redo" title="Redo (Ctrl+Shift+Z)" @click="restoreHistory('redo')"><svg width="18" height="18" viewBox="0 0 20 20" fill="none" aria-hidden="true"><path d="m13 4 4 4-4 4m4-4H9a5 5 0 0 0 0 10" stroke="currentColor" stroke-width="1.7" stroke-linecap="round" stroke-linejoin="round"/></svg></button><button type="button" class="quiet-button save-button" :disabled="locked || !dirty || saving" @click="persist().catch(showError)">Save</button></div>
      <div class="header-actions">
        <span v-if="info && !project" class="version">v{{ info.version }}</span>
        <button type="button" class="quiet-button theme-button" :aria-label="theme === 'dark' ? 'Switch to light theme' : 'Switch to dark theme'" @click="theme = theme === 'dark' ? 'light' : 'dark'">{{ theme === 'dark' ? 'Light' : 'Dark' }}</button>
        <button type="button" :class="project ? 'record-button' : 'primary-button'" :disabled="!desktop || locked || loading || !info?.ffmpegAvailable" @click="newRecording()">New recording</button>
        <button v-if="project" type="button" class="primary-button" :disabled="locked || !info?.ffmpegAvailable" @click="openExport">Export MP4</button>
      </div>
    </header>
    <div v-if="!desktop" class="runtime-notice" role="status">Recording and editing require the Windows desktop app. This browser view cannot access your devices or projects.</div>
    <div v-if="info && !info.ffmpegAvailable" class="error-banner" role="alert"><span>The video engine is unavailable. Reinstall Screenrecord or restore the bundled FFmpeg executable.</span><button class="quiet-button" type="button" :disabled="loading || locked" @click="initialize">Retry</button></div>
    <div v-if="(error || initializationError) && !recordingDialogOpen" class="error-banner" role="alert"><span>{{ error || initializationError }}</span><button v-if="initializationError" type="button" class="quiet-button" :disabled="loading || locked" @click="initialize">Retry initialization</button><button v-if="error" type="button" class="quiet-button" aria-label="Dismiss error" @click="error = ''">Dismiss</button></div>
    <div v-if="notice" class="notice-banner" role="status"><span>{{ notice }}</span><button type="button" class="quiet-button" aria-label="Dismiss notice" @click="notice = ''">Dismiss</button></div>
    <div v-if="closing" class="notice-banner" role="status">{{ saving || dirty ? 'Saving changes before closing...' : 'Closing...' }}</div>
    <div v-if="busy && !recordDialog?.open && !exportDialogOpen && busy !== 'Finalizing recording'" class="notice-banner" role="status">{{ busy }}... <button v-if="busy === 'Analyzing audio' || busy === 'Normalizing audio'" type="button" class="text-button" @click="stopAudioAnalysis().catch(showError)">Cancel analysis</button></div>
    <details v-if="visibleRecoveryIssues.length" class="recovery-issues"><summary>{{ visibleRecoveryIssues.length }} project recovery {{ visibleRecoveryIssues.length === 1 ? 'issue' : 'issues' }}</summary><ul><li v-for="issue in visibleRecoveryIssues" :key="issue">{{ issue }}</li></ul></details>
    <div v-if="recording.active || busy === 'Finalizing recording'" class="recording-banner"><div><strong role="status">{{ recording.phase === 'finalizing' || busy === 'Finalizing recording' ? 'Finalizing recording' : recording.paused ? 'Recording paused' : 'Recording' }}</strong><span aria-label="Recorded duration">{{ timeLabel(recording.elapsed) }}</span></div><div class="recording-actions"><button type="button" :disabled="!!busy || recording.phase === 'finalizing'" @click="pauseRecording">{{ recording.paused ? 'Resume recording' : 'Pause recording' }}</button><button type="button" class="stop-button" :disabled="!!busy || recording.phase === 'finalizing'" @click="stopRecording">{{ busy === 'Finalizing recording' ? busy : 'Stop recording' }}</button></div></div>
    <div v-if="transcribing" class="notice-banner" role="status"><span>Generating subtitles {{ transcriptionOptions.provider === 'local' ? 'on this computer' : 'with Deepgram' }}. {{ Math.round((transcription?.progress ?? 0) * 100) }}%</span><button type="button" :disabled="!!busy" @click="cancelTranscription">Cancel transcription</button></div>
    <div v-if="trackingMask" class="notice-banner" role="status"><span>Tracking mask {{ Math.round((maskProgress?.progress ?? 0) * 100) }}%</span><button type="button" :disabled="!!busy" @click="cancelMaskTracking().catch(showError)">Cancel tracking</button></div>
    <div class="workspace" :class="{ 'library-open': libraryOpen, 'has-project': !!project }" :inert="locked" @focusin="beginHistory" @pointerdown="beginHistory" @focusout="commitHistory" @change="commitHistory" @pointerup="commitHistory">
      <nav v-show="libraryOpen" id="project-library" class="project-sidebar" aria-label="Saved projects">
        <div class="sidebar-heading"><h2>Projects</h2><button type="button" class="text-button" :disabled="!desktop || loading || locked" @click="initialize">Refresh</button></div>
        <p v-if="loading" class="sidebar-state" role="status">Loading projects...</p>
        <p v-else-if="!projects.length" class="sidebar-state">Your recordings will appear here.</p>
        <ul v-else class="project-list"><li v-for="item in projects" :key="item.id" :class="{ selected: project?.id === item.id }" @contextmenu.prevent="openProjectMenu($event, item)" @keydown="projectMenuKeyboard($event, item)"><button type="button" class="project-open" :disabled="locked || opening" :aria-current="project?.id === item.id ? 'true' : undefined" @click="openProject(item.id)"><span :id="`project-name-${item.id}`" class="project-name">{{ item.name }}</span><span class="project-meta">{{ timeLabel(item.duration) }}<span v-if="item.status === 'interrupted'">Recovered</span><span v-else>{{ new Date(item.createdAt).toLocaleDateString(undefined, { month: 'short', day: 'numeric' }) }}</span></span></button><button type="button" class="project-actions" aria-label="Project actions" :aria-describedby="`project-name-${item.id}`" aria-haspopup="menu" :aria-expanded="projectMenu?.item.id === item.id" :disabled="locked" @click="openProjectMenu($event, item)"><svg width="18" height="18" viewBox="0 0 18 18" fill="currentColor" aria-hidden="true"><circle cx="9" cy="4" r="1.5"/><circle cx="9" cy="9" r="1.5"/><circle cx="9" cy="14" r="1.5"/></svg></button></li></ul>
        <div class="sidebar-footer"><button v-if="info" type="button" class="text-button" @click="reveal(info.projectsDirectory)">Open projects folder</button></div>
      </nav>
      <main v-if="!project || opening" class="empty-workspace">
        <template v-if="opening"><h2>Opening project...</h2><p role="status">Loading the recording and its edits.</p></template>
        <template v-else><div class="empty-frame" aria-hidden="true"><svg viewBox="0 0 64 48" width="64" height="48" fill="none"><rect x="3" y="3" width="58" height="36" rx="3" stroke="currentColor" stroke-width="2"/><path d="M23 45h18M32 39v6M27 14l13 8-13 8V14Z" stroke="currentColor" stroke-width="2" stroke-linejoin="round"/></svg></div><h2>{{ projects.length ? 'Open a recording' : 'Record your screen' }}</h2><p>{{ projects.length ? 'Choose a saved project to continue editing.' : 'Choose a screen or window, then add a microphone and webcam.' }}</p><button type="button" class="primary-button" :disabled="!desktop || locked || loading || !info?.ffmpegAvailable" @click="newRecording()">New recording</button><button v-if="projects.length && !libraryOpen" type="button" @click="toggleLibrary">Browse projects</button></template>
      </main>
      <template v-else>
        <main class="editor-main">
          <div v-if="project.status === 'interrupted'" class="recovery-note">Recovered after an interrupted recording. Check playback before exporting.</div>
          <div class="preview-status"><span>Screen {{ project.width }} x {{ project.height }}</span><button type="button" class="source-status" @click="showTool('camera')">{{ project.cameraPath ? project.edits.camera.enabled ? 'Camera visible' : 'Camera hidden' : 'No camera recorded' }}</button><button type="button" class="source-status" @click="showTool('audio')">{{ project.microphonePath ? project.edits.microphoneVolume > 0 ? 'Microphone audible' : 'Microphone muted' : 'No microphone recorded' }}</button><span v-if="project.systemAudioPath">Computer audio</span><span v-if="drawing && !correctionRegion" class="drawing-status" role="status">Draw a region. Escape cancels.</span></div>
          <div v-if="hiddenCaptions" class="recovery-note">Captions crossing cuts are hidden. <button type="button" class="text-button" @click="showTool('subtitles')">Review or regenerate captions</button></div>
          <div v-if="correctionRegion" class="recovery-note" role="status">Original frame for correction. Playback is stopped. Draw the new mask; Escape restores the masked preview. <button type="button" class="text-button" @click="stopCorrection">Cancel correction</button></div>
          <div v-if="unreviewedMasks.length" class="recovery-note">{{ unreviewedMasks.length }} tracked {{ unreviewedMasks.length === 1 ? 'mask needs' : 'masks need' }} review before export. <button type="button" class="text-button" @click="selectRegion(unreviewedMasks[0].id)">Review masks</button></div>
          <Preview v-if="!previewReleased" ref="preview" :key="project.id" :project="project" :monitor="monitor" :loop-range="previewLoop" :drawing="drawing" :original-frame="!!correctionRegion" @exit-original="stopCorrection" :selected-region="selection?.kind === 'redactions' && activeTool === 'redactions' ? selection.id : null" :selected-camera="activeTool === 'camera'" :disabled="locked" @region="drawRegion" @select-camera="selectCamera" @select-region="selectRegion" @update-camera="updateCamera" @update-region="updateRegion" @begin-edit="beginPreviewEdit" @end-edit="endPreviewEdit" @time="time = $event" @playing="playing = $event" @error="showError" />
          <div v-else class="preview-releasing" role="status">Releasing recording media...</div>
          <div class="transport"><button type="button" class="play-button" :aria-label="playing ? 'Pause preview' : 'Play preview'" :disabled="locked || !!correctionRegion" @click="preview?.toggle()"><svg v-if="playing" width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><rect x="3" y="2" width="3" height="12"/><rect x="10" y="2" width="3" height="12"/></svg><svg v-else width="16" height="16" viewBox="0 0 16 16" fill="currentColor" aria-hidden="true"><path d="M4 2v12l10-6Z"/></svg></button><span class="time-display">{{ timeLabel(outputTime, true) }} <span>/ {{ timeLabel(editedDuration, true) }}</span></span><span class="transport-spacer"></span><button type="button" class="small-button" :disabled="locked" @click="insertZoom">Add zoom</button><button type="button" class="small-button" :disabled="locked" @click="setSourceTrim('trimStart', time)">Set start</button><button type="button" class="small-button" :disabled="locked" @click="setSourceTrim('trimEnd', time)">Set end</button></div>
          <EditorTimeline :audition-open="auditionOpen" :monitor="monitor" :audition-range="auditionRange" :loop-enabled="loopEnabled" @toggle-audition="toggleAudition" @monitor="monitor = $event" @audition-range="auditionRange = $event" @loop="loopEnabled = $event" :project="project" :disabled="locked" :audio-analysis="audioAnalysis.states" :selected-video-clip="selectedVideoClip" @select-video="selectVideo" @split="splitAtPlayhead" @delete-video="deleteVideoClip" @cut-range="openCutRange" @error="showError" :time="time" :selected-zoom="selectedZoom" :selection="selection" @seek="seek" @select-zoom="selectZoom" @select="selectTrack" @begin-edit="beginTimelineEdit" @end-edit="endTimelineEdit" @import-music="importMusic" @subtitles="showTool('subtitles')" @masks="showTool('redactions')" @tool="showTool" />
        </main>
        <Inspector :selected-reframe="selectedReframe" :adding-reframe="addingReframe" @add-reframe="addReframe" @select-reframe="selectReframe" @update-reframe="updateReframe" @remove-reframe="removeReframe" :correcting="!!correctionRegion" @update-mask="updateMask" @seek="seek" @track-mask="startMaskTracking" @correct-mask="correctMask" @stop-correction="stopCorrection" @apply-appearance="applyPreset" ref="inspector" :project="project" :audio-analysis="audioAnalysis.states" :selected-video-clip="selectedVideoClip" :selected-zoom="selectedZoom" :selection="selection" :time="time" :drawing="drawing" @analyze-audio="analyzeRecordedAudio($event)" @normalize-audio="normalizeRecordedAudio" @cancel-analysis="stopAudioAnalysis().catch(showError)" @active-tool="setActiveTool" @select-zoom="selectZoom" @remove-zoom="removeZoom" @regenerate="regenerate" @select="selectTrack" @import-music="importMusic" @transcribe="openTranscription" @export-srt="exportSrt" @import-srt="importSrt" @draw="toggleDrawing" @new-camera-recording="newRecording(true)" @error="showError" />
      </template>
    </div>
    <dialog ref="cutDialog" aria-labelledby="cut-range-title"><form @submit.prevent="applyCutRange"><div class="dialog-heading"><h2 id="cut-range-title">Cut video range</h2><button type="button" @click="cutDialog?.close()">Cancel</button></div><p class="field-help">Times refer to the edited video. Remaining clips close the gap. Original recordings stay unchanged.</p><div class="field-grid"><div><label for="cut-start">Output start (seconds)</label><input id="cut-start" v-model.number="cutStart" type="number" min="0" :max="editedDuration" step="any" /></div><div><label for="cut-end">Output end (seconds)</label><input id="cut-end" v-model.number="cutEnd" type="number" min="0" :max="editedDuration" step="any" /></div></div><p v-if="cutError" class="dialog-error" role="alert">{{ cutError }}</p><div class="dialog-footer"><button type="submit" class="delete-button" :disabled="locked">Cut range</button></div></form></dialog>
    <div v-if="projectMenu" ref="projectMenuElement" class="project-context-menu" role="menu" :aria-label="`Actions for ${projectMenu.item.name}`" :style="{ left: `${projectMenu.x}px`, top: `${projectMenu.y}px` }" @keydown.stop="menuKeyboard"><button type="button" role="menuitem" :disabled="locked" @click="confirmProjectDeletion">Delete project...</button></div>
    <dialog ref="deleteDialog" class="delete-dialog" aria-labelledby="delete-title" aria-describedby="delete-description" @cancel.prevent="cancelProjectDeletion"><form @submit.prevent="deleteProject"><h2 id="delete-title">Delete project?</h2><p class="delete-project-name">{{ deleteTarget?.name }}</p><p id="delete-description">The project folder and its recordings will move to the Windows Recycle Bin. Exports saved outside this folder are kept.</p><p v-if="deleteError" class="dialog-error" role="alert">{{ deleteError }}</p><div class="dialog-footer"><button type="button" data-cancel-delete :disabled="deleting" @click="cancelProjectDeletion">Cancel</button><button type="submit" class="delete-button" :disabled="deleting || locked">{{ deleting ? 'Moving to Recycle Bin...' : deleteError ? 'Retry delete' : 'Move to Recycle Bin' }}</button></div></form></dialog>
    <footer v-if="exportProgress && !exportDialogOpen" class="export-footer" role="status"><span>{{ exportProgress.state === 'running' ? `Exporting ${Math.round(exportProgress.progress * 100)}%` : exportProgress.state === 'completed' ? `Export complete${completedExportSize ? ` / ${completedExportSize}` : ''}` : exportProgress.state === 'cancelled' ? 'Export cancelled' : exportProgress.error }}</span><span v-if="exportStatusError">{{ exportStatusError }}</span><button v-if="exporting" class="text-button" type="button" :disabled="busy === 'Starting export'" @click="cancelExport">Cancel export</button><button v-else-if="exportProgress.state === 'completed'" class="text-button" type="button" @click="reveal(exportProgress.outputPath)">Show in folder</button></footer>
    <dialog ref="transcriptionDialog" aria-labelledby="transcription-title" @cancel.prevent="closeTranscriptionDialog" @close="transcriptionOptions.apiKey = ''; transcriptionDialogOpen = false; localInstallBusy = false"><form :inert="closing" @submit.prevent="startTranscription">
      <div class="dialog-heading"><h2 id="transcription-title">Generate subtitles</h2><button type="button" :disabled="!!busy || transcriptionDialogClosing" @click="closeTranscriptionDialog">{{ transcriptionDialogClosing ? 'Stopping setup...' : 'Cancel' }}</button></div>
      <label for="transcription-provider">Recognition provider</label><select id="transcription-provider" :value="transcriptionOptions.provider" :disabled="!!busy || transcriptionDialogClosing" @change="changeTranscriptionProvider"><option value="local">Local (on this computer)</option><option value="cloud">Cloud (Deepgram)</option></select>
      <LocalTranscriptionSetup v-if="transcriptionDialogOpen" v-show="transcriptionOptions.provider === 'local'" ref="localSetup" @ready="localReady = $event" @busy="localInstallBusy = $event" />
      <p v-if="transcriptionOptions.provider === 'cloud'" class="field-help">This uploads the selected audio in retained footage to Deepgram Nova-3. Deepgram usage fees apply. Your API key is used for this job only and is not saved. Cancelling cannot undo audio already uploaded.</p>
      <p v-if="project?.edits.subtitles.length" class="field-help">Generated cues replace the current subtitles. You can undo the replacement.</p>
      <label for="transcription-source">Speech audio</label><select id="transcription-source" v-model="transcriptionOptions.source" :disabled="!!busy"><option v-if="project?.microphonePath" value="microphone">Microphone</option><option v-if="project?.systemAudioPath" value="system">Computer audio</option></select>
      <label for="transcription-language">Language</label><select id="transcription-language" v-model="transcriptionOptions.language" :disabled="!!busy"><option value="id">Indonesian</option><option value="en">English</option><option value="auto">Detect language</option></select>
      <template v-if="transcriptionOptions.provider === 'cloud'"><label for="transcription-key">Deepgram API key</label><input id="transcription-key" v-model="transcriptionOptions.apiKey" type="password" autocomplete="off" spellcheck="false" :disabled="!!busy" /></template>
      <p v-if="error" role="alert" class="dialog-error">{{ error }}</p><div class="dialog-footer"><button type="submit" class="primary-button" :disabled="!!busy || transcriptionDialogClosing || (transcriptionOptions.provider === 'local' ? !localReady || localInstallBusy : !transcriptionOptions.apiKey.trim())">{{ busy || (transcriptionOptions.provider === 'local' ? 'Generate locally' : 'Upload audio and generate') }}</button></div>
    </form></dialog>
    <dialog ref="recordDialog" class="record-dialog" aria-labelledby="record-title" @cancel.prevent="closeRecordingDialog" @close="recordingDialogClosed">
      <form @submit.prevent="startRecording">
        <div class="dialog-heading"><h2 id="record-title">New recording</h2><button type="button" class="quiet-button" :disabled="!!busy || closing" @click="closeRecordingDialog">Cancel</button></div>
        <div class="record-dialog-body">
        <div class="record-layout" :inert="countdown !== null || !!busy || trialBusy || closing">
          <section class="record-source-panel" aria-label="Recording source">
            <div class="source-toolbar"><div class="segmented" aria-label="Source type"><button type="button" :disabled="devicesLoading" :class="{ selected: sourceKind === 'monitor' }" :aria-pressed="sourceKind === 'monitor'" @click="sourceKind = 'monitor'">Screen</button><button type="button" :disabled="devicesLoading" :class="{ selected: sourceKind === 'window' }" :aria-pressed="sourceKind === 'window'" @click="sourceKind = 'window'">Window</button><button type="button" :disabled="devicesLoading" :class="{ selected: sourceKind === 'apps' }" :aria-pressed="sourceKind === 'apps'" @click="sourceKind = 'apps'">Apps</button></div><button type="button" class="text-button" :disabled="devicesLoading || !!busy" @click="refreshDevices()">Refresh devices</button></div>
            <p v-if="devicesLoading" role="status" class="field-help">Finding screens and devices...</p>
            <div v-else-if="devicesReady && sourceKind === 'apps'" class="app-picker">
              <p v-if="!captureApps.length" class="field-help" role="status">No apps with available windows. Open the app you want to record, then refresh devices.</p>
              <template v-else>
                <label for="capture-app-search">Find an app or window</label><input id="capture-app-search" v-model="appSearch" type="search" autocomplete="off" @keydown.enter.prevent />
                <div class="source-list" role="group" aria-label="Running apps">
                  <p v-if="!filteredApps.length" class="field-help" role="status">No apps match this search.</p>
                  <label v-for="app in filteredApps" :key="app.id" class="source-choice app-choice" :class="{ selected: appSelection.appId === app.id }"><input type="radio" name="capture-app" :value="app.id" :checked="appSelection.appId === app.id" :aria-label="app.name" :aria-describedby="`capture-app-detail-${app.id}`" @change="selectCaptureApp(app)" /><span><strong>{{ app.name }}</strong><small :id="`capture-app-detail-${app.id}`">{{ app.windows.length }} {{ app.windows.length === 1 ? 'window' : 'windows' }} / {{ app.windows[0]?.title }}</small></span></label>
                </div>
                <template v-if="selectedApp"><label for="capture-app-window">Window from {{ selectedApp.name }}</label><select id="capture-app-window" :value="appSelection.windowId" @change="selectAppWindow"><option value="" disabled>Choose a window</option><option v-for="window in selectedApp.windows" :key="window.id" :value="window.id">{{ window.title }} ({{ window.width }} x {{ window.height }})</option></select></template>
              </template>
              <p v-if="appSelection.needsReselection" class="field-help app-selection-needed" role="status">The selected app or window is no longer available. Select an app and window before recording.</p>
            </div>
            <div v-else-if="devicesReady" class="source-list"><p v-if="!filteredSources.length" class="field-help">No {{ sourceKind === 'monitor' ? 'screens' : 'windows' }} available. {{ sourceKind === 'monitor' ? 'Connect or enable a display, then refresh devices.' : 'Open the window you want to capture, then refresh devices.' }}</p><label v-for="source in filteredSources" :key="source.id" class="source-choice" :class="{ selected: options.sourceId === source.id }"><input v-model="options.sourceId" type="radio" name="capture-source" :value="source.id" /><svg width="32" height="28" viewBox="0 0 32 28" fill="none" aria-hidden="true"><rect x="2" y="2" width="28" height="19" rx="2" stroke="currentColor" stroke-width="1.5"/><path d="M11 26h10m-5-5v5" stroke="currentColor" stroke-width="1.5"/></svg><span><strong>{{ source.title }}</strong><small>{{ source.width }} x {{ source.height }}</small></span></label></div>
            <p v-else class="field-help">Refresh devices to load available recording sources.</p>
            <p v-if="sourceKind === 'apps'" class="field-help">Records one selected window. Other windows and new popups are not included.</p>
            <p v-if="(sourceKind === 'window' || sourceKind === 'apps') && selectedSource" class="field-help">Keep this window open and restored while recording.</p>
            <SourceRegion ref="sourceRegion" v-model="options.region" :source="selectedSource" :disabled="devicesLoading || !devicesReady || !!busy || trialBusy || closing" />
            <div class="record-audio-settings"><h3>Audio and quality</h3>
              <div class="record-field-heading"><label for="microphone">Microphone</label><span v-if="!options.microphoneId" class="device-off" aria-hidden="true">Off</span></div>
              <select id="microphone" v-model="options.microphoneId" :disabled="devicesLoading || !devicesReady || !!busy" @change="rememberRecordingOptions"><option :value="null">{{ devices.microphones.length ? 'Off' : 'No microphone detected' }}</option><option v-for="mic in devices.microphones" :key="mic.id" :value="mic.id">{{ mic.name }}</option></select>
              <MicrophoneMeter v-if="recordingDialogOpen" ref="microphoneMeter" :microphone-id="options.microphoneId" :disabled="devicesLoading || !devicesReady || !!busy || trialBusy || closing" :active="recordingDialogOpen" />
              <label class="check-label"><input v-model="options.systemAudio" type="checkbox" :disabled="!!busy" @change="rememberRecordingOptions" />Record computer audio</label><template v-if="sourceKind === 'apps' && options.systemAudio"><label for="computer-audio-scope">Computer audio source</label><select id="computer-audio-scope" v-model="appAudioScope"><option value="system">Whole system</option><option value="application">Selected application only</option></select><p class="field-help">{{ options.systemAudioScope === 'application' ? 'Records audio from the selected application and its child processes. Other applications are excluded.' : 'Computer audio includes sound from all apps.' }}</p></template><div class="record-quality"><div><label for="record-fps">Frame rate</label><select id="record-fps" v-model.number="options.fps" :disabled="!!busy" @change="rememberRecordingOptions"><option :value="30">30 fps</option><option :value="60">60 fps</option></select></div><div><label for="record-countdown">Countdown</label><select id="record-countdown" v-model.number="countdownSeconds" :disabled="!!busy" @change="rememberCountdown"><option :value="0">No countdown</option><option :value="3">3 seconds</option><option :value="5">5 seconds</option></select></div></div>
            </div>
          </section>
          <section class="record-camera-panel" aria-label="Camera setup"><div class="record-field-heading"><label for="webcam">Webcam</label><span class="device-off">{{ options.cameraId ? 'Included in recording' : 'Off' }}</span></div><select id="webcam" v-model="options.cameraId" :disabled="devicesLoading || !devicesReady || !!busy" @change="rememberRecordingOptions"><option :value="null">{{ devices.cameras.length ? 'Off - record without camera' : 'No webcam detected' }}</option><option v-for="cam in devices.cameras" :key="cam.id" :value="cam.id">{{ cam.name }}</option></select><CameraPreview v-if="recordingDialogOpen" ref="cameraPreview" :camera-id="options.cameraId" :disabled="devicesLoading || !devicesReady || !!busy || trialBusy || closing" :active="recordingDialogOpen" /><p class="field-help">{{ options.cameraId ? 'This camera will be recorded. Enable preview to check framing before you start.' : 'Camera is off for this recording. Select a webcam to include it.' }}</p></section>
        </div>
        <RecordingTrial v-if="recordingDialogOpen" ref="recordingTrial" :options="options" :active="recordingDialogOpen" :disabled="!selectedSource || !devicesReady || devicesLoading || !!busy || closing || countdown !== null" :prepare="prepareRecordingTest" @busy="trialBusy = $event" />
        </div>
        <footer class="record-dialog-footer">
          <div v-if="recordError || error" ref="recordErrorElement" class="dialog-error record-error" role="alert" tabindex="-1"><strong v-if="recordError">{{ recordError.title }}</strong><p>{{ recordError?.message ?? error }}</p></div>
          <section v-if="countdown !== null" class="record-countdown" aria-label="Recording countdown"><strong aria-hidden="true">{{ countdown }}</strong><div><p role="status">Recording starts in {{ countdown }} {{ countdown === 1 ? 'second' : 'seconds' }}.</p><button ref="countdownCancel" type="button" @click="cancelCountdown">Cancel countdown</button></div></section>
          <p v-else class="field-help">The editor minimizes when recording starts. {{ stopShortcut ? `Press ${stopShortcut} to stop, or use the floating controls.` : 'Use the floating controls to stop recording.' }}</p>
          <div class="record-actions"><span class="record-summary"><span class="record-source-name" :title="sourceSummary">{{ devicesReady ? sourceSummary : devicesLoading ? 'Loading sources...' : 'Refresh devices to continue' }}</span><small>{{ options.cameraId ? 'Camera included' : 'No camera' }} / {{ options.microphoneId ? 'Microphone included' : 'No microphone' }}</small></span><button ref="recordStartButton" type="submit" class="primary-button" :disabled="!selectedSource || !devicesReady || devicesLoading || !!busy || trialBusy || closing || countdown !== null">{{ busy || (countdown !== null ? 'Counting down...' : recordingFailed ? 'Retry recording' : 'Start recording') }}</button></div>
        </footer>
      </form>
    </dialog>
    <dialog ref="exportDialog" class="export-dialog" aria-labelledby="export-title" @cancel.prevent="closeExportDialog" @close="!exportDialog?.open && exportClosed()">
      <div class="dialog-heading"><h2 id="export-title">Export MP4</h2><button type="button" class="quiet-button" @click="closeExportDialog">{{ exporting ? 'Hide' : 'Close' }}</button></div>
      <dl v-if="project" class="export-summary"><div><dt>Video</dt><dd>{{ exportDimensions }} / {{ exportPreset === 'whatsapp' ? 30 : exportOptions.fps }} fps</dd></div><div><dt>Duration</dt><dd>{{ timeLabel(editedDuration, true) }}</dd></div><div><dt>Webcam</dt><dd>{{ !project.cameraPath ? 'Not recorded' : project.edits.camera.enabled ? 'Included' : 'Hidden' }}</dd></div></dl>
      <template v-if="!exportProgress || exportProgress.state === 'failed' || exportProgress.state === 'cancelled'">
        <label for="export-preset">Export preset</label><select id="export-preset" v-model="exportPreset" :disabled="!!busy || choosingOutput"><option value="standard">Standard</option><option value="whatsapp">WhatsApp</option></select>
        <div v-if="exportPreset === 'standard'" class="field-grid"><div><label for="export-size">Resolution</label><select id="export-size" v-model="exportOptions.resolution" :disabled="!!busy"><option value="1080p">1080p</option><option value="1440p">1440p</option><option value="2160p">4K</option></select></div><div><label for="export-fps">Frame rate</label><select id="export-fps" v-model.number="exportOptions.fps" :disabled="!!busy"><option :value="30">30 fps</option><option :value="60">60 fps</option></select></div></div>
        <template v-else>
          <p class="field-help">16 MB app limit. H.264 MP4 at 30 fps, saved locally for manual sharing.</p>
          <p v-if="exportPlanLoading" role="status">Calculating WhatsApp export size...</p>
          <template v-else-if="exportPlanError || !validWhatsAppPlan"><p class="dialog-error" role="alert">{{ exportPlanError || 'The project changed. Calculate the export size again.' }}</p><button type="button" class="small-button" :disabled="!!busy" @click="loadWhatsAppPlan">Retry size calculation</button></template>
        </template>
        <label for="output-path">Save location</label><div class="output-field"><input id="output-path" :value="exportOptions.outputPath" readonly placeholder="Choose a location" /><button type="button" :disabled="!!busy || choosingOutput" @click="chooseOutput">{{ choosingOutput ? 'Choosing...' : 'Browse' }}</button></div>
        <p class="field-help">Choose a new filename. Existing files are never overwritten.</p>
        <p v-if="exportProgress?.error" class="dialog-error" role="alert">{{ exportProgress.error }}</p><p v-if="exportProgress?.state === 'cancelled'" role="status">Export cancelled.</p><p v-if="error && error !== exportProgress?.error" class="dialog-error" role="alert">{{ error }}</p>
        <div class="dialog-footer"><button type="button" class="primary-button" :disabled="!exportOptions.outputPath || !!busy || choosingOutput || exportPreset === 'whatsapp' && !validWhatsAppPlan" @click="startExport">{{ busy || 'Export video' }}</button></div>
      </template>
      <template v-else-if="exportProgress.state === 'running'"><p class="export-state">Exporting video <strong>{{ Math.round(exportProgress.progress * 100) }}%</strong></p><progress :value="exportProgress.progress" max="1" aria-label="Export progress"></progress><p class="field-help">You can hide this window while the video exports.</p><p v-if="exportStatusError" class="dialog-error" role="alert">{{ exportStatusError }}</p><div class="dialog-footer"><button type="button" :disabled="busy === 'Starting export'" @click="cancelExport">Cancel export</button></div></template>
      <template v-else-if="exportProgress.state === 'completed'"><p class="export-state">Video exported</p><p class="export-path">{{ exportProgress.outputPath }}</p><p v-if="completedExportSize" class="field-help">File size: {{ completedExportSize }}</p><div class="dialog-footer"><button type="button" class="primary-button" @click="reveal(exportProgress!.outputPath)">Show in folder</button></div></template>
    </dialog>
  </div>
</template>
