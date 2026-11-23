import type { AppearancePreset, AppearanceSettings, EditSettings } from './types'

export function appearanceSettings(edits: AppearanceSettings): AppearanceSettings {
  const { aspectRatio, background, padding, cornerRadius, cursorSize, cursorSmoothing, showCursor } = edits
  const { enabled, x, y, width, shape, mirror } = edits.camera
  const subtitle = edits.subtitleStyle
  return { fitMode: edits.fitMode ?? 'fit', backgroundMode: edits.backgroundMode ?? 'solid', backgroundBlur: edits.backgroundBlur ?? 24, aspectRatio, background, padding, cornerRadius, cursorSize, cursorSmoothing, showCursor, showClicks: edits.showClicks ?? true,
    camera: { enabled, layout: edits.camera.layout ?? 'bubble', x, y, width, shape, mirror },
    subtitleStyle: { enabled: subtitle.enabled, fontSize: subtitle.fontSize, color: subtitle.color, background: subtitle.background, position: subtitle.position, highlightWords: subtitle.highlightWords ?? false, highlightColor: subtitle.highlightColor ?? '#E9B451' } }
}

export function applyAppearance(edits: EditSettings, settings: AppearanceSettings, hasCamera: boolean): EditSettings {
  const appearance = appearanceSettings(settings)
  if (!hasCamera && appearance.camera.layout === 'fullscreen') throw new Error('This preset requires recorded camera video. Record with a camera or choose another preset.')
  if (!hasCamera) appearance.camera.enabled = false
  return { ...edits, ...appearance }
}

const base: AppearanceSettings = {
  fitMode: 'fit', backgroundMode: 'solid', backgroundBlur: 24, aspectRatio: 'source', background: '#20252b', padding: 42, cornerRadius: 16,
  cursorSize: 1.5, cursorSmoothing: .5, showCursor: true, showClicks: true,
  camera: { enabled: true, layout: 'bubble', x: .75, y: .64, width: .2, shape: 'circle', mirror: true },
  subtitleStyle: { enabled: true, fontSize: 42, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: false, highlightColor: '#E9B451' },
}
export const builtInPresets: readonly AppearancePreset[] = [
  { id: 'builtin-screen', name: 'Screen only', settings: { ...appearanceSettings(base), camera: { ...base.camera, enabled: false }, padding: 0, cornerRadius: 0 } },
  { id: 'builtin-bubble', name: 'Presenter bubble', settings: appearanceSettings(base) },
  { id: 'builtin-split', name: 'Side by side', settings: { ...appearanceSettings(base), aspectRatio: '16:9', camera: { ...base.camera, layout: 'split', width: .35 } } },
  { id: 'builtin-camera', name: 'Camera only', settings: { ...appearanceSettings(base), aspectRatio: '16:9', camera: { ...base.camera, layout: 'fullscreen' } } },
]
