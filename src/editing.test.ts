import { describe, expect, it } from 'vitest'
import { addZoom, cursorAt, hasMinimumDuration, normalizeEdits, smoothCursor, timeLabel, zoomAt } from './editing'
import type { CursorSample, EditSettings } from './types'

const sample = (time: number, x: number): CursorSample => ({ time, x, y: .5, pressed: false, visible: true })
const edits = (): EditSettings => ({ videoClips: [], audioProcessing: { microphoneGain: 1, systemGain: 1, ducking: { enabled: false, amountDb: 12 } }, trimStart: 0, trimEnd: 20, aspectRatio: 'source', background: '#202020', padding: 32, cornerRadius: 12, cursorSize: 1, cursorSmoothing: .5, showCursor: true, zooms: [], camera: { enabled: false, layout: 'bubble', x: .7, y: .7, width: .2, shape: 'circle', mirror: true }, crop: null, systemVolume: 1, microphoneVolume: 1, music: [], subtitles: [], subtitleStyle: { enabled: true, fontSize: 44, color: '#ffffff', background: '#000000', position: 'bottom', highlightWords: false, highlightColor: '#E9B451' }, redactions: [] })
describe('timeline editing', () => {
  it('keeps genuine 100ms intervals despite floating point roundoff', () => {
    const e = edits(); e.trimStart = 11.9; e.trimEnd = 12
    e.zooms = [{ id: 'short', start: 11.9, end: 12, scale: 2, x: .5, y: .5, followCursor: true }]
    const normalized = normalizeEdits(e, 12)
    expect(normalized.trimStart).toBe(11.9); expect(normalized.zooms).toHaveLength(1)
    expect(hasMinimumDuration(11.9, 12)).toBe(true)
    expect(hasMinimumDuration(11.900001, 12)).toBe(false)
    expect(addZoom({ ...e, zooms: [] }, 11.9, 12)?.end).toBe(12)
  })
  it('places a new zoom after an occupied segment without crossing the next segment', () => {
    const e = edits(); e.zooms = [{ id: 'a', start: 2, end: 5, scale: 2, x: .5, y: .5, followCursor: true }, { id: 'b', start: 6, end: 9, scale: 2, x: .5, y: .5, followCursor: true }]
    const added = addZoom(e, 3, 20)
    expect(added?.start).toBe(5); expect(added?.end).toBe(6)
  })
  it('rejects a zoom when the trimmed interval has no space', () => {
    const e = edits(); e.zooms = [{ id: 'a', start: 0, end: 20, scale: 2, x: .5, y: .5, followCursor: true }]
    expect(addZoom(e, 2, 20)).toBeNull()
  })
  it('keeps crop and trim within the source after invalid numeric edits', () => {
    const e = edits(); e.trimStart = -4; e.trimEnd = 50; e.crop = { x: .8, y: -.2, width: .8, height: 1.1 }
    const result = normalizeEdits(e, 20)
    expect(result.trimStart).toBe(0); expect(result.trimEnd).toBe(20)
    expect(result.crop?.width).toBeCloseTo(.2); expect(result.crop?.y).toBe(0); expect(e.crop.width).toBe(.8)
  })
})
describe('preview timing', () => {
  it('interpolates position but retains a button state until the next sample', () => {
    const samples = [sample(0, 0), { ...sample(.1, 1), pressed: true }]
    expect(cursorAt(samples, .05)).toMatchObject({ x: .5, pressed: false }); expect(cursorAt(samples, .1)?.pressed).toBe(true)
  })
  it('smooths by elapsed time rather than number of samples', () => {
    const result = smoothCursor([sample(0, 0), sample(.12, 1)], 1)
    expect(result[1].x).toBeCloseTo(1 - Math.exp(-1))
  })
  it('does not interpolate missing capture periods or show a cursor before capture begins', () => {
    const samples = [sample(1, 0), sample(2, 1)]
    expect(cursorAt(samples, .5)?.visible).toBe(false)
    expect(cursorAt(samples, 1.5)?.x).toBe(0)
    expect(smoothCursor(samples, 1)[1].x).toBe(1)
  })
  it('eases into and out of zoom without jumping at segment boundaries', () => {
    const z = [{ id: 'z', start: 1, end: 3, scale: 2, x: .7, y: .4, followCursor: false }]
    expect(zoomAt(z, 1, null).scale).toBe(1); expect(zoomAt(z, 2, null).scale).toBe(2); expect(zoomAt(z, 3, null).scale).toBe(1)
    expect(zoomAt(z, 1.175, null).scale).toBeCloseTo(1.5)
  })
  it('formats long recordings without minute rollover ambiguity', () => { expect(timeLabel(3661)).toBe('1:01:01'); expect(timeLabel(Number.NaN)).toBe('0:00') })
  it('shows elapsed tenths at hour boundaries without rounding into the next frame', () => {
    expect(timeLabel(3599.999, true)).toBe('59:59.9')
    expect(timeLabel(3600.099, true)).toBe('1:00:00.0')
    expect(timeLabel(3600.1, true)).toBe('1:00:00.1')
    expect(timeLabel(10801.9, true)).toBe('3:00:01.9')
  })
})
