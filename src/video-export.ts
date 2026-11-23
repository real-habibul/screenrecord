import { computed, ref, watch, type Ref } from 'vue'
import { save } from '@tauri-apps/plugin-dialog'
import { command, errorText } from './api'
import { aspectRatio, even } from './editing'
import type { ExportOptions, ExportProgress, Project, WhatsAppExportPlan } from './types'

interface VideoExportContext {
  project: Ref<Project | null>
  busy: Ref<string>
  error: Ref<string>
  dirty: Readonly<Ref<boolean>>
  savedSnapshot: Readonly<Ref<string>>
  persist: () => Promise<void>
  prepareProject: () => Promise<void>
  canStart: () => boolean
  showError: (value: unknown) => void
}

export function useVideoExport({
  project, busy, error, dirty, savedSnapshot, persist, prepareProject, canStart, showError,
}: VideoExportContext) {
  const exportDialog = ref<HTMLDialogElement>()
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
  const validWhatsAppPlan = computed(() =>
    !!whatsappPlan.value && !exportPlanLoading.value && !dirty.value &&
    whatsappPlanSnapshot.value === savedSnapshot.value,
  )
  const exportDimensions = computed(() => {
    const current = project.value
    if (!current) return ''
    if (exportPreset.value === 'whatsapp') {
      const plan = whatsappPlan.value
      return plan && validWhatsAppPlan.value ? `${plan.width} x ${plan.height}` : 'Auto size'
    }
    const ratio = aspectRatio(current)
    const resolution = exportOptions.value.resolution
    const edge = resolution === '2160p' ? 2160 : resolution === '1440p' ? 1440 : 1080
    return `${even(edge * Math.max(1, ratio))} x ${even(edge * Math.max(1, 1 / ratio))}`
  })

  const exportProgress = ref<ExportProgress | null>(null)
  const exportStatusError = ref('')
  const completedExportSize = computed(() => {
    const progress = exportProgress.value
    const bytes = progress?.outputBytes
    return progress?.state === 'completed' && typeof bytes === 'number' &&
      Number.isFinite(bytes) && bytes >= 0 ? `${(bytes / 1_000_000).toFixed(2)} MB` : ''
  })
  let exportTargetPath: string | null = null
  let exportAttempt = 0
  function acceptExportProgress(value: ExportProgress | null, event = false) {
    if (!value) return
    const previous = exportProgress.value
    if (event && value.state === 'running' && previous?.state !== 'running' && busy.value !== 'Starting export') {
      exportAttempt++
      exportTargetPath = value.outputPath
    }
    if (exportTargetPath && value.outputPath !== exportTargetPath) return
    if (!event && previous?.outputPath === value.outputPath && previous.state !== 'running' && value.state === 'running') return
    exportStatusError.value = ''
    exportProgress.value = value
  }
  const exporting = computed(() => exportProgress.value?.state === 'running')
  const progressRevision = () => exportAttempt

  async function openExport() {
    busy.value = 'Preparing export'
    error.value = ''
    try {
      await prepareProject()
      const current = project.value
      if (!current) return
      exportSession++
      exportAttempt++
      exportOptions.value.fps = current.fps
      exportOptions.value.outputPath = ''
      exportProgress.value = null
      exportStatusError.value = ''
      exportTargetPath = null
      exportDialog.value?.showModal()
      exportDialogOpen.value = true
    } catch (failure) {
      showError(failure)
    } finally {
      busy.value = ''
    }
  }
  function exportClosed() {
    exportDialogOpen.value = false
    exportSession++
    exportPlanRequest++
    outputRequest++
    whatsappPlan.value = null
    whatsappPlanSnapshot.value = ''
    exportPlanLoading.value = false
    exportPlanError.value = ''
    choosingOutput.value = false
  }

  function closeExportDialog() {
    exportDialog.value?.close()
    exportClosed()
  }

  watch(project, (current, previous) => {
    if (current !== previous && exportDialogOpen.value) closeExportDialog()
  })

  watch([exportPreset, exportDialogOpen], () => {
    if (exportDialogOpen.value && exportPreset.value === 'whatsapp') {
      void loadWhatsAppPlan()
    } else {
      exportPlanRequest++
      whatsappPlan.value = null
      exportPlanLoading.value = false
      exportPlanError.value = ''
    }
  })

  async function loadWhatsAppPlan() {
    const current = project.value
    const session = exportSession
    const request = ++exportPlanRequest
    if (!current || !exportDialogOpen.value || exportPreset.value !== 'whatsapp') return
    const matches = () => request === exportPlanRequest && session === exportSession &&
      current === project.value && exportDialogOpen.value && exportPreset.value === 'whatsapp'
    whatsappPlan.value = null
    whatsappPlanSnapshot.value = ''
    exportPlanError.value = ''
    exportPlanLoading.value = true
    try {
      await persist()
      if (!matches()) return
      const snapshot = savedSnapshot.value
      const plan = await command<WhatsAppExportPlan>('get_whatsapp_export_plan', { id: current.id })
      if (!matches()) return
      if (snapshot !== savedSnapshot.value || dirty.value) throw new Error('The project changed. Calculate the export size again.')
      if (![plan.width, plan.height, plan.videoBitrate, plan.audioBitrate].every(value => Number.isFinite(value) && value > 0)) throw new Error('Export dimensions are unavailable. Try calculating the size again.')
      whatsappPlan.value = plan
      whatsappPlanSnapshot.value = snapshot
    } catch (failure) {
      if (matches()) exportPlanError.value = errorText(failure)
    } finally {
      if (matches()) exportPlanLoading.value = false
    }
  }
  async function chooseOutput() {
    const current = project.value
    const session = exportSession
    const preset = exportPreset.value
    if (!current || !exportDialogOpen.value || choosingOutput.value || busy.value || exporting.value) return
    const request = ++outputRequest
    choosingOutput.value = true
    try {
      const name = current.name.replace(/[<>:"/\\|?*]/g, '_')
      const path = await save({
        defaultPath: `${name}${preset === 'whatsapp' ? '-whatsapp' : ''}.mp4`,
        filters: [{ name: 'MP4 video', extensions: ['mp4'] }],
      })
      if (path && request === outputRequest && session === exportSession &&
        current === project.value && preset === exportPreset.value && exportDialogOpen.value) {
        exportOptions.value.outputPath = path.toLowerCase().endsWith('.mp4') ? path : `${path}.mp4`
      }
    } catch (failure) {
      if (session === exportSession) showError(failure)
    } finally {
      if (request === outputRequest) choosingOutput.value = false
    }
  }
  async function startExport() {
    const current = project.value
    const session = exportSession
    const preset = exportPreset.value
    const path = exportOptions.value.outputPath
    if (!current || !path || exporting.value || busy.value || choosingOutput.value ||
      !exportDialogOpen.value || (preset === 'whatsapp' && !validWhatsAppPlan.value)) return
    if (!canStart()) return
    const attempt = ++exportAttempt
    busy.value = 'Starting export'
    error.value = ''
    let admitted = false
    try {
      await prepareProject()
      if (current !== project.value || session !== exportSession || !exportDialogOpen.value ||
        preset !== exportPreset.value || path !== exportOptions.value.outputPath) return
      if (preset === 'whatsapp' && !validWhatsAppPlan.value) {
        exportPlanError.value = 'The project changed. Calculate the export size again.'
        return
      }
      const options: ExportOptions = preset === 'whatsapp'
        ? { outputPath: path, preset, resolution: 'auto', fps: 30 }
        : { ...exportOptions.value, preset: 'standard' }
      exportTargetPath = path
      exportProgress.value = { state: 'running', progress: 0, outputPath: path, error: null }
      await command('export_project', { id: current.id, options })
      admitted = true
      if (attempt !== exportAttempt) return
      if (busy.value === 'Starting export') busy.value = ''
      const progress = await command<ExportProgress | null>('get_export_progress')
      if (attempt === exportAttempt) acceptExportProgress(progress)
    } catch (failure) {
      if (attempt !== exportAttempt) return
      const runningExport = exportProgress.value
      if (admitted) {
        if (runningExport?.state === 'running') {
          exportStatusError.value = `Export started. Status check failed: ${errorText(failure)} Retrying status automatically.`
        }
      } else {
        if (runningExport?.state === 'running') {
          exportProgress.value = { state: 'failed', progress: runningExport.progress, outputPath: path, error: errorText(failure) }
        }
        showError(failure)
      }
    } finally {
      if (attempt === exportAttempt && busy.value === 'Starting export') busy.value = ''
    }
  }

  async function cancelExport() {
    if (busy.value === 'Starting export') return
    try {
      await command('cancel_export')
    } catch (failure) {
      showError(failure)
    }
  }

  async function pollExport() {
    if (!exporting.value || busy.value === 'Starting export') return
    const attempt = exportAttempt
    const isCurrent = () => attempt === exportAttempt && busy.value !== 'Starting export'
    try {
      const progress = await command<ExportProgress | null>('get_export_progress')
      if (isCurrent()) acceptExportProgress(progress)
    } catch (failure) {
      if (isCurrent() && exporting.value) {
        exportStatusError.value = `Status check failed: ${errorText(failure)} Retrying automatically.`
      }
    }
  }

  return {
    exportDialog, exportDialogOpen, exportOptions, exportPreset,
    exportProgress, exportStatusError, exporting, completedExportSize,
    validWhatsAppPlan, exportDimensions, exportPlanLoading, exportPlanError, choosingOutput,
    openExport, exportClosed, closeExportDialog, loadWhatsAppPlan, chooseOutput,
    startExport, cancelExport, acceptExportProgress, progressRevision, pollExport,
  }
}
