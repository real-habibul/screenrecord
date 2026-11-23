export interface CaptureSource { id: string; title: string; kind: 'monitor' | 'window'; width: number; height: number }
export interface CaptureApp { id: string; name: string; windows: CaptureSource[] }
export interface Device { id: string; name: string }
export interface CaptureDevices { sources: CaptureSource[]; microphones: Device[]; cameras: Device[]; apps?: CaptureApp[] }
export interface CaptureRegion { x: number; y: number; width: number; height: number }
export interface SourceThumbnail { width: number; height: number; data: string }
export interface RecordingOptions { sourceId: string; fps: number; microphoneId: string | null; systemAudio: boolean; cameraId: string | null; systemAudioScope?: 'system' | 'application'; region?: CaptureRegion | null }
export interface RecordingTestResult { id: string; duration: number; width: number; height: number; screenPath: string; cameraPath: string | null; cameraOffsetSeconds: number; microphonePath: string | null; systemAudioPath: string | null }
export interface CursorSample { time: number; x: number; y: number; pressed: boolean; visible: boolean; buttons?: number }
export interface ZoomSegment { id: string; start: number; end: number; scale: number; x: number; y: number; followCursor: boolean }
export interface VideoClip { id: string; start: number; end: number }
export interface ReframeKeyframe { id: string; time: number; x: number; y: number; scale: number }
export interface CropSettings { x: number; y: number; width: number; height: number }
export interface CameraSettings { enabled: boolean; layout: 'bubble' | 'fullscreen' | 'split'; x: number; y: number; width: number; shape: 'rectangle' | 'circle'; mirror: boolean }
export interface MusicClip { id: string; name: string; path: string; sourceDuration: number; start: number; sourceStart: number; duration: number; volume: number; fadeIn: number; fadeOut: number }
export interface SubtitleRange { start: number; end: number }
export interface SubtitleWord extends SubtitleRange { text: string }
export interface SubtitleCue { id: string; start: number; end: number; text: string; words?: SubtitleWord[]; sourceRanges?: SubtitleRange[] }
export interface SubtitleStyle { enabled: boolean; fontSize: number; color: string; background: string; position: 'bottom' | 'top'; highlightWords: boolean; highlightColor: string }
export interface MaskKeyframe { time: number; x: number; y: number; width: number; height: number; confidence: number; kind: 'auto' | 'manual' | 'lost' }
export interface MaskTracking { keyframes: MaskKeyframe[]; reviewed: boolean }
export interface MaskTrackingProgress { jobId: string; projectId: string; regionId: string; state: 'running' | 'completed' | 'cancelled' | 'failed'; progress: number; error: string | null; tracking: MaskTracking | null }
export interface Redaction { id: string; start: number; end: number; x: number; y: number; width: number; height: number; mode: 'blur' | 'solid'; strength: number; tracking?: MaskTracking | null }
export type TrackKind = 'music' | 'subtitles' | 'redactions'
export interface TrackSelection { kind: TrackKind; id: string }
export interface TranscriptionOptions { provider: 'local' | 'cloud'; apiKey: string; source: 'microphone' | 'system'; language: 'id' | 'en' | 'auto' }
export interface LocalTranscriptionStatus { ready: boolean; runtimeReady: boolean; modelReady: boolean; runtimeVersion: string; modelName: string; downloadBytes: number; installedBytes: number; error: string | null }
export interface LocalTranscriptionInstallProgress { jobId: string; state: 'running' | 'completed' | 'cancelled' | 'failed'; progress: number; error: string | null; status: LocalTranscriptionStatus | null }
export interface TranscriptionProgress { state: 'running' | 'completed' | 'cancelled' | 'failed'; projectId: string; jobId: string; progress: number; error: string | null; cues: SubtitleCue[] | null }
export type AudioBus = 'microphone' | 'system' | 'music'
export interface VolumeKeyframe { id: string; time: number; volume: number }
export interface AudioProcessing {
  microphoneGain: number
  systemGain: number
  ducking: { enabled: boolean; systemEnabled?: boolean; amountDb: number }
  volumeKeyframes?: Partial<Record<AudioBus, VolumeKeyframe[]>>
}
export interface WaveformBin { min: number; max: number }
export interface AudioAnalysis { sourceDuration: number; bins: WaveformBin[]; peak: number; rms: number; analyzedSeconds: number; recommendedGain: number }
export type RecordedAudioSource = 'microphone' | 'system'
export interface EditSettings {
  fitMode?: 'fit' | 'fill';
  reframeKeyframes?: ReframeKeyframe[];
  backgroundMode?: 'solid' | 'videoBlur'; backgroundBlur?: number;
  videoClips: VideoClip[];
  trimStart: number; trimEnd: number; aspectRatio: 'source' | '16:9' | '9:16' | '1:1';
  background: string; padding: number; cornerRadius: number; cursorSize: number;
  cursorSmoothing: number; showCursor: boolean; showClicks?: boolean; zooms: ZoomSegment[]; camera: CameraSettings;
  crop: CropSettings | null; systemVolume: number; microphoneVolume: number; audioProcessing: AudioProcessing;
  music: MusicClip[]; subtitles: SubtitleCue[]; subtitleStyle: SubtitleStyle; redactions: Redaction[];
}
export interface Project {
  version: number; id: string; name: string; createdAt: string; directory: string;
  screenPath: string; microphonePath: string | null; systemAudioPath: string | null; cameraPath: string | null;
  cameraOffsetSeconds: number;
  cameraIntervals?: Array<{ start: number; end: number }> | null;
  width: number; height: number; fps: number; duration: number; cursor: CursorSample[];
  edits: EditSettings; status: 'ready' | 'interrupted';
}
export interface SavedEdits { name: string; edits: EditSettings }
export type AppearanceSettings = Pick<EditSettings, 'fitMode' | 'backgroundMode' | 'backgroundBlur' | 'aspectRatio' | 'background' | 'padding' | 'cornerRadius' | 'cursorSize' | 'cursorSmoothing' | 'showCursor' | 'showClicks' | 'camera' | 'subtitleStyle'>
export interface AppearancePreset { id: string; name: string; settings: AppearanceSettings }
export type ProjectSummary = Pick<Project, 'id' | 'name' | 'createdAt' | 'duration' | 'directory' | 'status'>
export interface RecordingState { active: boolean; paused: boolean; phase: 'idle' | 'recording' | 'paused' | 'finalizing'; cameraAvailable: boolean; elapsed: number; projectId: string | null }
export type RecordingError = { projectId: string; error: string } | string
export interface RecordingCameraFrame { generation: number; sequence: number; mimeType: string; data: string | null; active: boolean; error: string | null }
export interface ExportOptions { outputPath: string; preset?: 'standard' | 'whatsapp'; resolution: '1080p' | '1440p' | '2160p' | 'auto'; fps: number }
export interface WhatsAppExportPlan { width: number; height: number; videoBitrate: number; audioBitrate: number }
export interface ExportProgress { state: 'running' | 'completed' | 'cancelled' | 'failed'; progress: number; outputPath: string; error: string | null; outputBytes?: number }
export interface AppInfo { version: string; projectsDirectory: string; ffmpegAvailable: boolean; ffmpegPath: string | null }
