import { describe, expect, it } from 'vitest'
import { appearanceSettings, applyAppearance, builtInPresets } from './appearance'
import { compositionLayout, previewGeometry } from './editing'
import type { EditSettings, Project } from './types'

function project(): Project {
  return {
    id: 'appearance-test', cameraPath: 'camera.mp4', width: 960, height: 540,
    edits: { ...appearanceSettings(builtInPresets[1].settings), trimStart: 1, trimEnd: 10,
      videoClips: [{ id: 'keep', start: 1, end: 4 }], crop: { x: .2, y: .1, width: .6, height: .8 },
      reframeKeyframes: [{ id: 'manual-focus', time: 2, x: .2, y: .8, scale: 2 }],
      zooms: [{ id: 'zoom', start: 2, end: 3, scale: 2, x: .4, y: .5, followCursor: false }],
      music: [{ id: 'music', path: 'private.wav' }], subtitles: [{ id: 'cue', start: 2, end: 3, text: 'Keep these words' }],
      redactions: [{ id: 'mask', mode: 'solid' }], microphoneVolume: .3, systemVolume: .7,
      audioProcessing: { microphoneGain: 2, systemGain: 1, ducking: { enabled: true, amountDb: 12 } },
    } as EditSettings,
  } as Project
}

describe('appearance presets', () => {
  it('whitelists style fields including nested settings without copying source content', () => {
    const value = project(); const extra = { ...builtInPresets[2].settings, trimStart: 99, reframeKeyframes: value.edits.reframeKeyframes, music: ['leak'], camera: { ...builtInPresets[2].settings.camera, path: 'private.mp4' }, subtitleStyle: { ...builtInPresets[2].settings.subtitleStyle, text: 'private text' } }
    const saved = appearanceSettings(extra)
    expect(Object.keys(saved).sort()).toEqual(['fitMode', 'backgroundMode', 'backgroundBlur', 'aspectRatio', 'background', 'camera', 'cornerRadius', 'cursorSize', 'cursorSmoothing', 'padding', 'showCursor', 'showClicks', 'subtitleStyle'].sort())
    expect(saved.camera).not.toHaveProperty('path'); expect(saved.subtitleStyle).not.toHaveProperty('text')
    const applied = applyAppearance(value.edits, extra, true)
    for (const key of ['trimStart', 'trimEnd', 'videoClips', 'crop', 'zooms', 'music', 'subtitles', 'redactions', 'microphoneVolume', 'systemVolume', 'audioProcessing', 'reframeKeyframes'] as const) expect(applied[key]).toEqual(value.edits[key])
    expect(applied.camera.layout).toBe('split')
    applied.camera.width = .6; applied.subtitleStyle.fontSize = 60
    expect(extra.camera.width).toBe(.35); expect(extra.subtitleStyle.fontSize).toBe(42)
  })
  it('disables absent camera tracks and rejects camera-only presets without one', () => {
    expect(applyAppearance(project().edits, builtInPresets[1].settings, false).camera.enabled).toBe(false)
    expect(applyAppearance(project().edits, builtInPresets[2].settings, false).camera.enabled).toBe(false)
    expect(() => applyAppearance(project().edits, builtInPresets[3].settings, false)).toThrow('requires recorded camera video')
  })
  it('saves screen sizing and defaults legacy presets to Fit without replacing source edits', () => {
    const value = project(); value.edits.fitMode = 'fill'
    expect(appearanceSettings(value.edits).fitMode).toBe('fill')
    const legacy = { ...builtInPresets[1].settings }; delete legacy.fitMode
    const applied = applyAppearance(value.edits, legacy, true)
    expect(applied.fitMode).toBe('fit'); expect(applied.crop).toEqual(value.edits.crop); expect(applied.videoClips).toEqual(value.edits.videoClips)
  })
  it('roundtrips video background settings while legacy presets retain solid defaults and never store keys', () => {
    const value = project(); value.edits.backgroundMode = 'videoBlur'; value.edits.backgroundBlur = 64
    const saved = appearanceSettings(value.edits)
    expect(saved.backgroundMode).toBe('videoBlur'); expect(saved.backgroundBlur).toBe(64); expect(saved).not.toHaveProperty('reframeKeyframes')
    const applied = applyAppearance(value.edits, saved, true)
    expect(applied.reframeKeyframes).toEqual(value.edits.reframeKeyframes)
    delete saved.backgroundMode; delete saved.backgroundBlur
    expect(appearanceSettings(saved)).toMatchObject({ backgroundMode: 'solid', backgroundBlur: 24 })
  })
  it('defaults legacy click appearance on and preserves explicit suppression through save and apply', () => {
    const value = project(); delete value.edits.showClicks
    expect(appearanceSettings(value.edits).showClicks).toBe(true)
    value.edits.showClicks = false
    const saved = appearanceSettings(value.edits); expect(saved.showClicks).toBe(false)
    const applied = applyAppearance(project().edits, saved, true)
    expect(applied.showClicks).toBe(false); expect(applied.videoClips).toEqual(project().edits.videoClips)
  })
})

describe('shared camera pane geometry', () => {
  it('matches renderer reference coordinates for landscape and portrait split panes', () => {
    const value = project(); value.edits.crop = null; value.edits.padding = 32; value.edits.camera.layout = 'split'; value.edits.camera.width = .3
    value.edits.aspectRatio = '16:9'
    const landscape = previewGeometry(value, 0, null)
    expect([landscape.fx, landscape.fy, landscape.fw, landscape.fh]).toEqual([32, 180, 1282, 720])
    expect(landscape.cameraPane).toEqual({ x: 1338, y: 32, width: 550, height: 1016 })
    value.edits.aspectRatio = '9:16'
    const portrait = previewGeometry(value, 0, null)
    expect([portrait.fx, portrait.fy, portrait.fw, portrait.fh]).toEqual([32, 387, 1016, 572])
    expect(portrait.cameraPane).toEqual({ x: 32, y: 1338, width: 1016, height: 550 })
  })
  it('never reveals screen in fullscreen and never collapses split when camera is disabled or absent', () => {
    const value = project(); value.edits.camera.layout = 'fullscreen'
    const fullscreen = compositionLayout(value)
    value.edits.camera.enabled = false; value.cameraPath = null
    expect(compositionLayout(value)).toEqual(fullscreen); expect(fullscreen.screenVisible).toBe(false)
    value.edits.camera.layout = 'split'; const split = compositionLayout(value)
    value.cameraPath = 'camera.mp4'; value.edits.camera.enabled = true
    expect(compositionLayout(value)).toEqual(split); expect(split.cameraPane).not.toBeNull()
  })
  it('keeps source crop and zoom independent of panes, with zero-gap and minimum-share bounds', () => {
    const value = project(); value.edits.camera.layout = 'split'; value.edits.padding = 0; value.edits.camera.width = .08
    const split = previewGeometry(value, 2.5, null)
    value.edits.camera.layout = 'bubble'; const bubble = previewGeometry(value, 2.5, null)
    expect([split.sx, split.sy, split.sw, split.sh]).toEqual([bubble.sx, bubble.sy, bubble.sw, bubble.sh])
    expect(split.cameraPane!.x).toBe(split.screenPane.width); expect(split.fw).toBeLessThanOrEqual(split.screenPane.width)
  })
})
